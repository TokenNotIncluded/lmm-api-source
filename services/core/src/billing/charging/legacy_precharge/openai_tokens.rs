//! Ordinary-text BPE for the fixed Go tokenizer v0.6.2. The caller supplies the
//! source DTO's combined text. Message overhead and media are separate inputs.
use std::sync::OnceLock;

use fancy_regex::Regex;
use tiktoken_rs::{CoreBPE, Rank};

use super::Error;

#[path = "go_unicode.rs"]
mod go_unicode;

/// Fixed cc52 CountTextToken: OpenAI BPE or its provider-specific heuristic.
pub(crate) fn count_text(original_model: &str, text: &str) -> Result<u32, Error> {
    match count(original_model, text)? {
        Some(count) => Ok(count),
        None => estimate_text(text, original_model),
    }
}

// cc52 token_estimator.go, reused from its Rust parity implementation under
// AGPL-3.0-or-later. Keep the original f64 accumulation and classification order.
fn in_table(ch: char, ranges: &[(u32, u32, u32)]) -> bool {
    let ch = u32::from(ch);
    let index = ranges.partition_point(|(_, high, _)| *high < ch);
    ranges.get(index).is_some_and(|(low, high, stride)| {
        ch >= *low && ch <= *high && (ch - low).is_multiple_of(*stride)
    })
}

fn estimate_text(text: &str, model: &str) -> Result<u32, Error> {
    // Order: word, number, CJK, symbol, math, URL, @, emoji, newline, space.
    let model = model.to_lowercase();
    let m: [f64; 10] = if model.contains("gemini") {
        [1.15, 2.8, 0.68, 0.38, 1.05, 1.2, 2.5, 1.08, 1.15, 0.2]
    } else if model.contains("claude") {
        [1.13, 1.63, 1.21, 0.4, 4.52, 1.26, 2.82, 2.6, 0.89, 0.39]
    } else {
        [1.02, 1.55, 0.85, 0.4, 2.68, 1.0, 2.0, 2.12, 0.5, 0.42]
    };
    let mut count = 0.0_f64;
    let mut previous = 0;
    let mut run: usize = 0;
    for ch in text.chars() {
        let number = in_table(ch, go_unicode::NUMBERS);
        let cost = if in_table(ch, go_unicode::SPACE) {
            if ch == '\n' || ch == '\t' { m[8] } else { m[9] }
        } else if in_table(ch, go_unicode::HAN)
            || ('\u{3040}'..='\u{30ff}').contains(&ch)
            || ('\u{ac00}'..='\u{d7a3}').contains(&ch)
        {
            m[2]
        } else if ('\u{1f300}'..='\u{1f9ff}').contains(&ch)
            || ('\u{2600}'..='\u{26ff}').contains(&ch)
            || ('\u{2700}'..='\u{27bf}').contains(&ch)
            || ('\u{1fa00}'..='\u{1faff}').contains(&ch)
        {
            m[7]
        } else if number || in_table(ch, go_unicode::LETTERS) {
            let kind = if number { 2 } else { 1 };
            if previous != kind {
                count += if number { m[1] } else { m[0] };
                run = usize::from(number);
                previous = kind;
            } else if number {
                run += 1;
                if (run - 1).is_multiple_of(3) {
                    count += 1.0;
                }
            }
            continue;
        } else if "∑∫∂√∞≤≥≠≈±×÷∈∉∋∌⊂⊃⊆⊇∪∩∧∨¬∀∃∄∅∆∇∝∟∠∡∢°′″‴⁺⁻⁼⁽⁾ⁿ₀₁₂₃₄₅₆₇₈₉₊₋₌₍₎²³¹⁴⁵⁶⁷⁸⁹⁰"
            .contains(ch)
            || ('\u{2200}'..='\u{22ff}').contains(&ch)
            || ('\u{2a00}'..='\u{2aff}').contains(&ch)
            || ('\u{1d400}'..='\u{1d7ff}').contains(&ch)
        {
            m[4]
        } else if ch == '@' {
            m[6]
        } else if "/:?&=;#%".contains(ch) {
            m[5]
        } else {
            m[3]
        };
        previous = 0;
        run = 0;
        count += cost;
    }
    let count = count.ceil();
    if !count.is_finite() || count > u32::MAX as f64 {
        return Err(Error::QuotaRange);
    }
    Ok(count as u32)
}

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
    fn non_openai_count_uses_fixed_provider_heuristic() {
        assert_eq!(
            super::count_text("anthropic/claude-fable-5.1", "hello"),
            Ok(2)
        );
        assert_eq!(
            super::count_text("anthropic/claude-fable-5.1", "Hello world"),
            Ok(3)
        );
        assert_eq!(
            super::count_text("anthropic/claude-fable-5.1", "中国"),
            Ok(3)
        );
        assert_eq!(
            super::count_text("anthropic/claude-fable-5.1", "1234567"),
            Ok(4)
        );
        assert_eq!(super::count_text("gemini", "中国"), Ok(2));
        assert_eq!(super::count_text("unknown", ""), Ok(0));
        assert_eq!(super::count_text("gpt-6.1-sol", "hello"), Ok(1));
    }

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
