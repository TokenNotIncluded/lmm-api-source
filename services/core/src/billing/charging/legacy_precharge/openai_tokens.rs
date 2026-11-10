//! Ordinary-text BPE for the fixed Go tokenizer v0.6.2. The caller supplies the
//! source DTO's combined text. Message overhead and media are separate inputs.
use std::sync::OnceLock;

use fancy_regex::Regex;
use tiktoken_rs::{CoreBPE, Rank};

use super::Error;

// Split patterns from tiktoken-go/tokenizer v0.6.2 (MIT). The newer Rust
// factories supply the matching vocabulary, but have different patterns.
const CL100K_PATTERN: &str = r"(?i:'s|'t|'re|'ve|'m|'ll|'d)|[^\r\n\p{L}\p{N}]?\p{L}+|\p{N}{1,3}| ?[^\s\p{L}\p{N}]+[\r\n]*|\s*[\r\n]+|\s+(?!\S)|\s+";
const O200K_PATTERN: &str = r"[^\r\n\p{L}\p{N}]?[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}]*[\p{Ll}\p{Lm}\p{Lo}\p{M}]+(?i:'s|'t|'re|'ve|'m|'ll|'d)?|[^\r\n\p{L}\p{N}]?[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}]+[\p{Ll}\p{Lm}\p{Lo}\p{M}]*(?i:'s|'t|'re|'ve|'m|'ll|'d)?|\p{N}{1,3}| ?[^\s\p{L}\p{N}]+[\r\n/]*|\s*[\r\n]+|\s+(?!\S)|\s+";

struct Encoder {
    pattern: Regex,
    bpe: CoreBPE,
}

impl Encoder {
    fn new(pattern: &str, bpe: CoreBPE, rank_count: Rank) -> Result<Self, Error> {
        let pattern = Regex::new(pattern).map_err(|_| Error::Invalid)?;
        let ranks = (0..rank_count)
            .map(|rank| {
                bpe.decode_bytes(&[rank])
                    .map(|piece| (piece, rank))
                    .map_err(|_| Error::Invalid)
            })
            .collect::<Result<_, _>>()?;
        // Each match from the fallible source-pattern split is one BPE piece.
        // Use the library's optimized long-piece merge as well as short-piece lookup.
        let bpe =
            CoreBPE::new(ranks, Default::default(), r"(?s:.+)").map_err(|_| Error::Invalid)?;
        Ok(Self { pattern, bpe })
    }

    fn count(&self, text: &str) -> Result<u32, Error> {
        let mut count = 0u32;
        let mut end = 0;
        for matched in self.pattern.find_iter(text) {
            let matched = matched.map_err(|_| Error::Invalid)?;
            if matched.start() != end {
                return Err(Error::Invalid);
            }
            end = matched.end();
            let tokens = u32::try_from(self.bpe.count_ordinary(matched.as_str()))
                .map_err(|_| Error::QuotaRange)?;
            count = count.checked_add(tokens).ok_or(Error::QuotaRange)?;
        }
        if end != text.len() {
            return Err(Error::Invalid);
        }
        Ok(count)
    }
}

static CL100K: OnceLock<Result<Encoder, Error>> = OnceLock::new();
static O200K: OnceLock<Result<Encoder, Error>> = OnceLock::new();

/// None selects the separate source heuristic for non-OpenAI models. The Go
/// model classifier ignores case, but its codec lookup uses the original name.
pub(crate) fn count(original_model: &str, text: &str) -> Result<Option<u32>, Error> {
    let model = original_model.to_lowercase();
    if !["gpt-", "o1", "o3", "o4", "chatgpt"]
        .iter()
        .any(|prefix| model.contains(prefix))
    {
        return Ok(None);
    }
    if text.is_empty() {
        return Ok(Some(0));
    }
    let encoder = if matches!(original_model, "o3-mini" | "gpt-4o")
        || ["o1-", "gpt-4o-", "chatgpt-4o-"]
            .iter()
            .any(|prefix| original_model.starts_with(prefix))
    {
        O200K.get_or_init(|| {
            Encoder::new(
                O200K_PATTERN,
                tiktoken_rs::o200k_base().map_err(|_| Error::Invalid)?,
                199_998,
            )
        })
    } else {
        CL100K.get_or_init(|| {
            Encoder::new(
                CL100K_PATTERN,
                tiktoken_rs::cl100k_base().map_err(|_| Error::Invalid)?,
                100_256,
            )
        })
    };
    encoder
        .as_ref()
        .map_err(|&error| error)?
        .count(text)
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::count;

    #[test]
    fn counts_match_fixed_go_codecs_and_original_model_lookup() {
        // Oracle values from tokenizer v0.6.2, using Count on ordinary text.
        let text = "简体中文 日本語 한국어";
        assert_eq!(count("gpt-6.1-sol", text), Ok(Some(12)));
        assert_eq!(count("GPT-4O", text), Ok(Some(12)));
        assert_eq!(count("gpt-4o", text), Ok(Some(7)));
        assert_eq!(count("o1-preview", text), Ok(Some(7)));
        assert_eq!(count("o3-mini", text), Ok(Some(7)));
        assert_eq!(count("claude-sonnet", text), Ok(None));
        assert_eq!(count("gpt-6.1-sol", ""), Ok(Some(0)));

        let markers = "<|endoftext|> <|fim_prefix|> <|endofprompt|>";
        assert_eq!(count("gpt-6.1-sol", markers), Ok(Some(19)));
        assert_eq!(count("gpt-4o", markers), Ok(Some(20)));
    }
}
