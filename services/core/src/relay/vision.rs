//! Validated image inputs. Core forwards references without fetching image URLs.
use super::{ClientProtocol, RelayError, UpstreamProtocol};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
enum Source {
    Url(String),
    Base64 { media_type: String, data: String },
}

#[derive(Clone, Debug)]
pub struct ImageInput {
    source: Source,
    detail: Option<String>,
}

fn keys(value: &Value, allowed: &[&str]) -> Result<(), RelayError> {
    let value = value
        .as_object()
        .ok_or(RelayError::Invalid("expected image object"))?;
    if value.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(RelayError::Unsupported("unsupported image field"));
    }
    Ok(())
}

fn string(value: &Value) -> Result<&str, RelayError> {
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or(RelayError::Invalid("expected nonempty image string"))
}

impl ImageInput {
    fn base64(media_type: &str, data: &str) -> Result<Self, RelayError> {
        if !matches!(
            media_type,
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        ) {
            return Err(RelayError::Unsupported("unsupported image media type"));
        }
        if data.is_empty() || STANDARD.decode(data).is_err() {
            return Err(RelayError::Invalid("invalid base64 image"));
        }
        Ok(Self {
            source: Source::Base64 {
                media_type: media_type.into(),
                data: data.into(),
            },
            detail: None,
        })
    }

    fn url(value: &str) -> Result<Self, RelayError> {
        if let Some(data) = value.strip_prefix("data:") {
            let (media_type, data) = data
                .split_once(";base64,")
                .ok_or(RelayError::Invalid("invalid image data URL"))?;
            return Self::base64(media_type, data);
        }
        let url =
            reqwest::Url::parse(value).map_err(|_| RelayError::Invalid("invalid image URL"))?;
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || value.chars().any(char::is_control)
        {
            return Err(RelayError::Invalid("invalid image URL"));
        }
        Ok(Self {
            source: Source::Url(value.into()),
            detail: None,
        })
    }

    pub(super) fn openai(value: &Value, client: ClientProtocol) -> Result<Self, RelayError> {
        let (url, detail) = match client {
            ClientProtocol::Chat => {
                keys(value, &["type", "image_url"])?;
                let image = &value["image_url"];
                keys(image, &["url", "detail"])?;
                (string(&image["url"])?, image.get("detail"))
            }
            ClientProtocol::Responses => {
                keys(value, &["type", "image_url", "detail"])?;
                (string(&value["image_url"])?, value.get("detail"))
            }
            _ => return Err(RelayError::Invalid("invalid image client protocol")),
        };
        let mut image = Self::url(url)?;
        image.detail = detail
            .map(|detail| match detail.as_str() {
                Some(detail @ ("auto" | "low" | "high" | "original")) => Ok(detail.into()),
                _ => Err(RelayError::Invalid("invalid image detail")),
            })
            .transpose()?;
        Ok(image)
    }

    pub(super) fn anthropic(value: &Value) -> Result<Self, RelayError> {
        keys(value, &["type", "source"])?;
        let source = &value["source"];
        match source["type"].as_str() {
            Some("base64") => {
                keys(source, &["type", "media_type", "data"])?;
                Self::base64(string(&source["media_type"])?, string(&source["data"])?)
            }
            Some("url") => {
                keys(source, &["type", "url"])?;
                let image = Self::url(string(&source["url"])?)?;
                if !matches!(image.source, Source::Url(_)) {
                    return Err(RelayError::Invalid("Anthropic URL source requires HTTP(S)"));
                }
                Ok(image)
            }
            _ => Err(RelayError::Unsupported("unsupported image source")),
        }
    }

    pub(super) fn gemini_inline(value: &Value) -> Result<Self, RelayError> {
        keys(value, &["mimeType", "data"])?;
        Self::base64(string(&value["mimeType"])?, string(&value["data"])?)
    }

    pub(crate) fn encoded_len(&self) -> usize {
        match &self.source {
            Source::Url(url) => url.len(),
            Source::Base64 { media_type, data } => media_type.len() + data.len(),
        }
    }

    pub(super) fn wire(&self, protocol: UpstreamProtocol) -> Result<Value, RelayError> {
        if !matches!(
            protocol,
            UpstreamProtocol::Chat | UpstreamProtocol::Responses
        ) && self
            .detail
            .as_deref()
            .is_some_and(|detail| detail != "auto")
        {
            return Err(RelayError::Unsupported(
                "image detail requires an OpenAI protocol route",
            ));
        }
        match protocol {
            UpstreamProtocol::Chat | UpstreamProtocol::Responses => {
                let url = match &self.source {
                    Source::Url(url) => url.clone(),
                    Source::Base64 { media_type, data } => {
                        format!("data:{media_type};base64,{data}")
                    }
                };
                let mut image = if protocol == UpstreamProtocol::Chat {
                    json!({"type":"image_url", "image_url":{"url":url}})
                } else {
                    json!({"type":"input_image", "image_url":url})
                };
                if let Some(detail) = &self.detail {
                    if protocol == UpstreamProtocol::Chat {
                        image["image_url"]["detail"] = json!(detail);
                    } else {
                        image["detail"] = json!(detail);
                    }
                }
                Ok(image)
            }
            UpstreamProtocol::Anthropic => Ok(json!({"type":"image", "source":match &self.source {
                Source::Url(url) => json!({"type":"url", "url":url}),
                Source::Base64 { media_type, data } => json!({"type":"base64", "media_type":media_type, "data":data}),
            }})),
            UpstreamProtocol::Gemini => match &self.source {
                Source::Base64 { media_type, data } => {
                    Ok(json!({"inlineData":{"mimeType":media_type,"data":data}}))
                }
                Source::Url(_) => Err(RelayError::Unsupported(
                    "Gemini image conversion requires inline data",
                )),
            },
        }
    }
}
