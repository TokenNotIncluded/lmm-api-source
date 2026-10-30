//! OpenAI embeddings and Jina text reranking wire contracts.
//! Official references: developers.openai.com/api/reference/resources/embeddings/methods/create
//! and api.jina.ai/scalar. Neither endpoint reports generated-token consumption.
use crate::{
    relay::{RelayError, Usage},
    routing::Protocol,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Embeddings,
    Rerank,
}
impl Kind {
    pub(super) fn protocol(self) -> Protocol {
        match self {
            Self::Embeddings => Protocol::OpenAiEmbeddings,
            Self::Rerank => Protocol::JinaRerank,
        }
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Embeddings => "embeddings",
            Self::Rerank => "rerank",
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum EmbeddingInput {
    Text(String),
    Texts(Vec<String>),
    Tokens(Vec<u32>),
    TokenArrays(Vec<Vec<u32>>),
}
impl EmbeddingInput {
    fn count(&self) -> usize {
        match self {
            Self::Text(_) | Self::Tokens(_) => 1,
            Self::Texts(values) => values.len(),
            Self::TokenArrays(values) => values.len(),
        }
    }
    fn validate(&self) -> bool {
        (1..=2048).contains(&self.count())
            && match self {
                Self::Text(value) => !value.is_empty(),
                Self::Texts(values) => values.iter().all(|value| !value.is_empty()),
                Self::Tokens(values) => !values.is_empty(),
                Self::TokenArrays(values) => values.iter().all(|value| !value.is_empty()),
            }
    }
    fn token_count(&self) -> Option<usize> {
        match self {
            Self::Tokens(values) => Some(values.len()),
            Self::TokenArrays(values) => Some(values.iter().map(Vec::len).sum()),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Encoding {
    #[default]
    Float,
    Base64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Embeddings {
    model: String,
    input: EmbeddingInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding_format: Option<Encoding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Rerank {
    model: String,
    query: String,
    documents: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_n: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    return_documents: Option<bool>,
}

pub(super) struct Request {
    kind: Kind,
    body: Body,
}
enum Body {
    Embeddings(Embeddings),
    Rerank(Rerank),
}

impl Request {
    pub(super) fn parse(kind: Kind, raw: &[u8]) -> Result<Self, RelayError> {
        let invalid = || RelayError::Invalid("invalid input-only model request");
        let body = match kind {
            Kind::Embeddings => {
                let body: Embeddings = serde_json::from_slice(raw).map_err(|_| invalid())?;
                if !body.input.validate()
                    || body.dimensions == Some(0)
                    || body.user.as_ref().is_some_and(|value| {
                        value.len() > 256 || value.chars().any(char::is_control)
                    })
                {
                    return Err(invalid());
                }
                Body::Embeddings(body)
            }
            Kind::Rerank => {
                let body: Rerank = serde_json::from_slice(raw).map_err(|_| invalid())?;
                if body.query.is_empty()
                    || !(1..=2048).contains(&body.documents.len())
                    || body.documents.iter().any(String::is_empty)
                    || body
                        .top_n
                        .is_some_and(|n| n == 0 || n as usize > body.documents.len())
                {
                    return Err(invalid());
                }
                Body::Rerank(body)
            }
        };
        let request = Self { kind, body };
        let model = request.model();
        if model.is_empty() || model.len() > 256 || model.chars().any(char::is_control) {
            return Err(invalid());
        }
        Ok(request)
    }
    pub(super) fn kind(&self) -> Kind {
        self.kind
    }
    pub(super) fn model(&self) -> &str {
        match &self.body {
            Body::Embeddings(body) => &body.model,
            Body::Rerank(body) => &body.model,
        }
    }
    pub(super) fn wire(&self, model: &str) -> Result<Vec<u8>, RelayError> {
        let mut body = match &self.body {
            Body::Embeddings(body) => serde_json::to_value(body),
            Body::Rerank(body) => serde_json::to_value(body),
        }
        .map_err(|_| RelayError::Invalid("input-only request encoding"))?;
        body["model"] = model.into();
        serde_json::to_vec(&body).map_err(|_| RelayError::Invalid("input-only request encoding"))
    }
    pub(super) fn validate_bound(&self, input_max: u64) -> Result<(), RelayError> {
        if let Body::Embeddings(body) = &self.body
            && body
                .input
                .token_count()
                .is_some_and(|n| n as u64 > input_max)
        {
            return Err(RelayError::Limit("embedding batch input token limit"));
        }
        Ok(())
    }

    pub(super) fn response(&self, raw: &[u8]) -> Result<(Value, Usage), RelayError> {
        let invalid = || RelayError::Protocol("invalid input-only provider response");
        let (output, input) = match &self.body {
            Body::Embeddings(request) => {
                let response: EmbeddingResponse =
                    serde_json::from_slice(raw).map_err(|_| invalid())?;
                if response.object != "list"
                    || response.model.is_empty()
                    || response.data.len() != request.input.count()
                    || response.usage.prompt_tokens != response.usage.total_tokens
                {
                    return Err(invalid());
                }
                let mut indices = BTreeSet::new();
                let mut dimensions = None;
                for vector in &response.data {
                    if vector.object != "embedding"
                        || vector.index >= request.input.count()
                        || !indices.insert(vector.index)
                    {
                        return Err(invalid());
                    }
                    let length = match (
                        &vector.embedding,
                        request.encoding_format.unwrap_or_default(),
                    ) {
                        (Embedding::Floats(values), Encoding::Float)
                            if values.iter().all(|v| v.is_finite()) =>
                        {
                            values.len()
                        }
                        (Embedding::Base64(value), Encoding::Base64) => {
                            let bytes = STANDARD.decode(value).map_err(|_| invalid())?;
                            if bytes.len() % 4 != 0
                                || !bytes.chunks_exact(4).all(|chunk| {
                                    f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
                                        .is_finite()
                                })
                            {
                                return Err(invalid());
                            }
                            bytes.len() / 4
                        }
                        _ => return Err(invalid()),
                    };
                    if length == 0
                        || dimensions.is_some_and(|n| n != length)
                        || request.dimensions.is_some_and(|n| n as usize != length)
                    {
                        return Err(invalid());
                    }
                    dimensions = Some(length);
                }
                let input = response.usage.prompt_tokens;
                (
                    serde_json::to_value(response).map_err(|_| invalid())?,
                    input,
                )
            }
            Body::Rerank(request) => {
                let response: RerankResponse =
                    serde_json::from_slice(raw).map_err(|_| invalid())?;
                if response.results.is_empty()
                    || response.results.len()
                        > request
                            .top_n
                            .map_or(request.documents.len(), |n| n as usize)
                    || response
                        .usage
                        .prompt_tokens
                        .is_some_and(|n| n != response.usage.total_tokens)
                {
                    return Err(invalid());
                }
                let mut indices = BTreeSet::new();
                for result in &response.results {
                    if result.index >= request.documents.len()
                        || !indices.insert(result.index)
                        || !result.relevance_score.is_finite()
                        || (request.return_documents == Some(true) && result.document.is_none())
                        || result
                            .document
                            .as_ref()
                            .is_some_and(|doc| doc.text != request.documents[result.index])
                    {
                        return Err(invalid());
                    }
                }
                let input = response.usage.total_tokens;
                (
                    serde_json::to_value(response).map_err(|_| invalid())?,
                    input,
                )
            }
        };
        Ok((
            output,
            Usage {
                input_tokens: Some(input),
                cached_input_tokens: Some(0),
                output_tokens: Some(0),
                cache_creation_input_tokens: Some(0),
                ..Default::default()
            },
        ))
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EmbeddingUsage {
    prompt_tokens: u64,
    total_tokens: u64,
}
#[derive(Deserialize, Serialize)]
struct EmbeddingResponse {
    object: String,
    model: String,
    data: Vec<Vector>,
    usage: EmbeddingUsage,
}
#[derive(Deserialize, Serialize)]
struct Vector {
    object: String,
    index: usize,
    embedding: Embedding,
}
#[derive(Serialize)]
#[serde(untagged)]
enum Embedding {
    Floats(Vec<f64>),
    Base64(String),
}

impl<'de> Deserialize<'de> for Embedding {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Untagged deserialization buffers serde Content, which does not
        // preserve serde_json's arbitrary_precision number representation.
        // Read the actual string/array tokens directly so valid decimals and
        // scientific notation reach f64's numeric deserializer unchanged.
        struct EmbeddingVisitor;
        impl<'de> serde::de::Visitor<'de> for EmbeddingVisitor {
            type Value = Embedding;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a numeric embedding array or base64 string")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<f64>()? {
                    values.push(value);
                }
                Ok(Embedding::Floats(values))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(Embedding::Base64(value.to_owned()))
            }

            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(Embedding::Base64(value))
            }
        }
        deserializer.deserialize_any(EmbeddingVisitor)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RerankUsage {
    total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_tokens: Option<u64>,
}
#[derive(Deserialize, Serialize)]
struct RerankResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    results: Vec<Ranked>,
    usage: RerankUsage,
}
#[derive(Deserialize, Serialize)]
struct Ranked {
    index: usize,
    relevance_score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    document: Option<Document>,
}
#[derive(Deserialize, Serialize)]
struct Document {
    text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn embeddings_accept_all_four_inputs_and_preserve_batch_usage() {
        for input in [
            json!("hello"),
            json!(["hello", "world"]),
            json!([10, 20]),
            json!([[10], [20]]),
        ] {
            let request = Request::parse(
                Kind::Embeddings,
                &serde_json::to_vec(&json!({"model":"e","input":input})).unwrap(),
            )
            .unwrap();
            let Body::Embeddings(body) = &request.body else {
                unreachable!()
            };
            let data: Vec<_> = (0..body.input.count())
                .map(|index| json!({"object":"embedding","index":index,"embedding":[0.25,0.5]}))
                .collect();
            let (_, usage) = request.response(&serde_json::to_vec(&json!({"object":"list","model":"e","data":data,"usage":{"prompt_tokens":17,"total_tokens":17}})).unwrap()).unwrap();
            assert_eq!(usage.input_tokens, Some(17));
            assert_eq!(usage.output_tokens, Some(0));
        }
        for input in [json!([]), json!(""), json!(["x", 1]), json!([[]])] {
            assert!(
                Request::parse(
                    Kind::Embeddings,
                    &serde_json::to_vec(&json!({"model":"e","input":input})).unwrap()
                )
                .is_err()
            );
        }
        assert!(
            Request::parse(
                Kind::Embeddings,
                br#"{"model":"a","model":"b","input":"x"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn embedding_receipts_require_complete_indices_usage_and_finite_vectors() {
        let request = Request::parse(
            Kind::Embeddings,
            br#"{"model":"e","input":["x","y"],"dimensions":2}"#,
        )
        .unwrap();
        let receipt = json!({"object":"list","model":"e","data":[
            {"object":"embedding","index":0,"embedding":[0.25,0.5]},
            {"object":"embedding","index":1,"embedding":[0.5,0.25]}],
            "usage":{"prompt_tokens":4,"total_tokens":4}});
        assert!(
            request
                .response(&serde_json::to_vec(&receipt).unwrap())
                .is_ok()
        );
        for variant in 0..4 {
            let mut invalid = receipt.clone();
            match variant {
                0 => {
                    invalid.as_object_mut().unwrap().remove("usage");
                }
                1 => invalid["data"][1]["index"] = json!(0),
                2 => invalid["usage"]["total_tokens"] = json!(5),
                _ => invalid["data"][1]["embedding"] = json!([0.25]),
            }
            assert!(
                request
                    .response(&serde_json::to_vec(&invalid).unwrap())
                    .is_err()
            );
        }
        let request = Request::parse(
            Kind::Embeddings,
            br#"{"model":"e","input":"x","encoding_format":"base64"}"#,
        )
        .unwrap();
        let make = |value: f32| {
            serde_json::to_vec(&json!({"object":"list","model":"e","data":[{"object":"embedding","index":0,"embedding":STANDARD.encode(value.to_le_bytes())}],"usage":{"prompt_tokens":1,"total_tokens":1}})).unwrap()
        };
        assert!(request.response(&make(0.5)).is_ok());
        assert!(request.response(&make(f32::NAN)).is_err());
    }

    #[test]
    fn rerank_prices_total_processed_tokens_not_top_n() {
        let request = Request::parse(
            Kind::Rerank,
            br#"{"model":"r","query":"q","documents":["a","b"],"top_n":1,"return_documents":true}"#,
        )
        .unwrap();
        let receipt = json!({"model":"r","results":[{"index":1,"relevance_score":0.9,"document":{"text":"b"}}],"usage":{"total_tokens":25}});
        let (_, usage) = request
            .response(&serde_json::to_vec(&receipt).unwrap())
            .unwrap();
        assert_eq!(usage.input_tokens, Some(25));
        for variant in 0..3 {
            let mut invalid = receipt.clone();
            match variant {
                0 => invalid["usage"]["prompt_tokens"] = json!(24),
                1 => invalid["results"][0]["index"] = json!(2),
                _ => invalid["results"][0]["document"]["text"] = json!("other"),
            }
            assert!(
                request
                    .response(&serde_json::to_vec(&invalid).unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn embedding_decimals_and_exponents_are_numbers_without_string_or_object_coercion() {
        let request = Request::parse(
            Kind::Embeddings,
            br#"{"model":"e","input":"x","dimensions":3}"#,
        )
        .unwrap();
        let receipt = |embedding: &str| {
            format!(
                r#"{{"object":"list","model":"e","data":[{{"object":"embedding","index":0,"embedding":{embedding}}}],"usage":{{"prompt_tokens":4,"total_tokens":4}}}}"#
            )
        };
        for embedding in ["[0,0.25,-1e-3]", "[0,1,2]", "[1e-300,-1e100,0.0]"] {
            let (response, usage) = request.response(receipt(embedding).as_bytes()).unwrap();
            assert_eq!(usage.input_tokens, Some(4));
            assert_eq!(
                response["data"][0]["embedding"].as_array().unwrap().len(),
                3
            );
        }
        for embedding in [
            r#"[0,"0.25",1]"#,
            "[0,null,1]",
            "[0,true,1]",
            "[0,[0.25],1]",
            "[0,1e400,1]",
            r#"[0,{"$serde_json::private::Number":"0.25"},1]"#,
            r#""AAAAAA==""#,
        ] {
            assert!(
                request.response(receipt(embedding).as_bytes()).is_err(),
                "{embedding}"
            );
        }
    }
}
