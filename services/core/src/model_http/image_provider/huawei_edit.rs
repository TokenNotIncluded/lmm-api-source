//! Huawei MaaS Qwen image editing, as documented in model-call-023.
//! Only a completed image payload is evidence for a returned-image tariff.
use super::*;

const ENDPOINT: &str = "https://api.modelarts-maas.com/v1/images/generations";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HuaweiEditModel {
    QwenImageEdit,
    QwenImageEdit2509,
}

pub(super) struct HuaweiOutput {
    pub(super) media_type: ImageMediaType,
    pub(super) reported_size: Option<(u64, u64)>,
}

impl HuaweiEditModel {
    pub(super) fn for_endpoint(url: &Url, model: &str) -> Option<Self> {
        if url.as_str() != ENDPOINT {
            return None;
        }
        match model {
            "qwen_image_edit" => Some(Self::QwenImageEdit),
            "qwen-image-edit-2509" => Some(Self::QwenImageEdit2509),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::QwenImageEdit => "qwen_image_edit",
            Self::QwenImageEdit2509 => "qwen-image-edit-2509",
        }
    }

    pub(super) fn valid_dimensions(self, width: u32, height: u32) -> bool {
        let width = u64::from(width);
        let height = u64::from(height);
        match self {
            Self::QwenImageEdit => (512..=2048).contains(&width) && (512..=2048).contains(&height),
            Self::QwenImageEdit2509 => {
                (512 * 512..=3072 * 3072).contains(&(width * height))
                    && width.max(height) <= 12 * width.min(height)
            }
        }
    }

    pub(super) fn body(
        self,
        quote: &ImageQuote,
        prompt: &str,
        input: &ImageMultipartInput,
        format: &str,
    ) -> Result<Vec<u8>, RelayError> {
        let b = quote.binding();
        if !quote.charges_returned_images()
            || b.n != 1
            || !b.quality.is_empty()
            || input.mask.is_some()
            || format == "url"
        {
            return Err(RelayError::Unsupported(
                "unsupported Huawei image edit options or tariff",
            ));
        }
        let (max_images, cap) = match self {
            Self::QwenImageEdit => (1, 20_000_000usize),
            Self::QwenImageEdit2509 => (2, 30_000_000usize),
        };
        if input.images.is_empty() || input.images.len() > max_images {
            return Err(RelayError::Unsupported(
                "unsupported Huawei image reference count",
            ));
        }
        if !b.size.is_empty() {
            let dimensions = b
                .size
                .split_once('x')
                .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
            if !dimensions.is_some_and(|(w, h)| {
                self.valid_dimensions(w, h)
                    && b.size == format!("{w}x{h}")
                    && (self != Self::QwenImageEdit
                        || (w.is_multiple_of(16) && h.is_multiple_of(16)))
            }) {
                return Err(RelayError::Unsupported(
                    "unsupported Huawei output dimensions",
                ));
            }
        }
        let mut image = String::new();
        for upload in &input.images {
            let prefix = match upload.media_type {
                ImageMediaType::Png => "data:image/png;base64,",
                ImageMediaType::Jpeg => "data:image/jpeg;base64,",
                ImageMediaType::Webp => "data:image/webp;base64,",
            };
            let encoded = base64::encoded_len(upload.bytes.len(), true)
                .and_then(|n| n.checked_add(prefix.len() + usize::from(!image.is_empty())))
                .ok_or(RelayError::Limit("Huawei image edit request"))?;
            if encoded > cap - image.len() {
                return Err(RelayError::Limit("Huawei image edit request"));
            }
            if !image.is_empty() {
                image.push(',');
            }
            image.push_str(prefix);
            STANDARD.encode_string(&upload.bytes, &mut image);
        }
        let mut payload = json!({"model":self.name(),"prompt":prompt});
        payload["image"] = Value::String(image);
        if !b.size.is_empty() {
            payload["size"] = json!(b.size);
        }
        let body = serde_json::to_vec(&payload)
            .map_err(|_| RelayError::Invalid("invalid image request"))?;
        if body.len() > cap {
            return Err(RelayError::Limit("Huawei image edit request"));
        }
        Ok(body)
    }

    pub(super) fn normalize(self, value: &mut Value) -> Result<HuaweiOutput, RelayError> {
        let receipt = object(value, &["model", "created", "data", "usage", "error"])?;
        if receipt.get("model").and_then(Value::as_str) != Some(self.name())
            || receipt.get("created").and_then(Value::as_u64).is_none()
            || receipt.get("error").is_some_and(|v| !v.is_null())
        {
            return Err(RelayError::SettlementPending);
        }
        // These counters describe the generation. The tariff counts actual images.
        let usage = value
            .get("usage")
            .and_then(Value::as_object)
            .ok_or(RelayError::SettlementPending)?;
        if usage.values().any(|v| v.as_u64().is_none()) {
            return Err(RelayError::SettlementPending);
        }
        let reported_size = match (usage.get("width"), usage.get("height")) {
            (None, None) => None,
            (Some(w), Some(h)) => Some((
                w.as_u64().ok_or(RelayError::SettlementPending)?,
                h.as_u64().ok_or(RelayError::SettlementPending)?,
            )),
            _ => return Err(RelayError::SettlementPending),
        };
        let [item] = value["data"]
            .as_array_mut()
            .ok_or(RelayError::SettlementPending)?
            .as_mut_slice()
        else {
            return Err(RelayError::SettlementPending);
        };
        object(item, &["url", "b64_json"])?;
        let fields = item.as_object_mut().ok_or(RelayError::SettlementPending)?;
        if fields.remove("url").is_some_and(|u| !u.is_null()) {
            return Err(RelayError::SettlementPending);
        }
        let Some(Value::String(data)) = fields.get_mut("b64_json") else {
            return Err(RelayError::SettlementPending);
        };
        let (prefix, _) = data.split_once(',').ok_or(RelayError::SettlementPending)?;
        let media_type = match prefix {
            "data:image/png;base64" => ImageMediaType::Png,
            "data:image/jpeg;base64" | "data:image/jpg;base64" => ImageMediaType::Jpeg,
            "data:image/webp;base64" => ImageMediaType::Webp,
            _ => return Err(RelayError::SettlementPending),
        };
        data.drain(..=prefix.len());
        let root = value.as_object_mut().ok_or(RelayError::SettlementPending)?;
        root.remove("usage");
        root.remove("model");
        root.remove("error");
        // Public image responses use Unix seconds. MaaS sends milliseconds.
        if let Some(milliseconds) = root.get("created").and_then(Value::as_u64) {
            root.insert("created".into(), json!(milliseconds / 1000));
        }
        Ok(HuaweiOutput {
            media_type,
            reported_size,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huawei_edits_require_the_published_origin_path_and_model() {
        for (endpoint, model, expected) in [
            (
                ENDPOINT,
                "qwen_image_edit",
                Some(HuaweiEditModel::QwenImageEdit),
            ),
            (
                ENDPOINT,
                "qwen-image-edit-2509",
                Some(HuaweiEditModel::QwenImageEdit2509),
            ),
            (ENDPOINT, "qwen-image", None),
            (
                "https://api.modelarts-maas.com:8443/v1/images/generations",
                "qwen-image-edit-2509",
                None,
            ),
            (
                "https://api.modelarts-maas.com/private/v1/images/generations",
                "qwen_image_edit",
                None,
            ),
            (
                "https://custom.example/v1/images/generations",
                "qwen_image_edit",
                None,
            ),
        ] {
            assert_eq!(
                HuaweiEditModel::for_endpoint(&Url::parse(endpoint).unwrap(), model),
                expected
            );
        }
    }

    #[test]
    fn huawei_edits_reject_ambiguous_or_incomplete_provider_evidence() {
        let receipt = json!({"model":"qwen_image_edit","created":1762134592684_u64,
            "usage":{"model_latency":40,"prompt_tokens":0,"completion_tokens":0,"total_tokens":0},
            "data":[{"url":null,"b64_json":"data:image/png;base64,AQID"}],"error":null});
        let check = |raw: &Value| {
            observe_wire(
                &serde_json::to_vec(raw).unwrap(),
                1,
                "fixture-secret",
                ImageTokenUsageContract::NativeStrict,
                Some((HuaweiEditModel::QwenImageEdit, "")),
            )
        };
        assert!(check(&receipt).is_err()); // Valid base64, incomplete PNG.
        for (path, bad) in [
            ("/model", json!("different")),
            ("/error", json!({"code":"pending"})),
            ("/data", json!([])),
            ("/data/0/url", json!("https://example.com/other")),
            ("/data/0/b64_json", json!("data:text/html;base64,AQID")),
            ("/usage", json!(null)),
        ] {
            let mut changed = receipt.clone();
            *changed.pointer_mut(path).unwrap() = bad;
            assert!(
                HuaweiEditModel::QwenImageEdit
                    .normalize(&mut changed)
                    .is_err()
            );
        }
        let mut normalized = receipt;
        assert!(matches!(
            HuaweiEditModel::QwenImageEdit.normalize(&mut normalized),
            Ok(HuaweiOutput {
                media_type: ImageMediaType::Png,
                reported_size: None
            })
        ));
        assert_eq!(
            normalized,
            json!({"created":1762134592,"data":[{"b64_json":"AQID"}]})
        );
    }
}
