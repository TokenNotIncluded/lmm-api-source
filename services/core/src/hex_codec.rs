//! Allocation adapters for zhex's caller-owned buffers.
pub(crate) fn encode(input: impl AsRef<[u8]>) -> String {
    let input = input.as_ref();
    let len = zhex::encoded_len(input.len()).expect("hex output length overflow");
    let mut output = vec![0; len];
    zhex::encode_lower(input, &mut output).expect("exact hex output capacity");
    String::from_utf8(output).expect("zhex emits ASCII")
}
pub(crate) fn decode(input: impl AsRef<[u8]>) -> Result<Vec<u8>, zhex::Error> {
    let input = input.as_ref();
    let mut output = vec![0; zhex::decoded_len(input.len())?];
    zhex::decode(input, &mut output)?;
    Ok(output)
}
#[cfg(test)]
mod tests {
    use super::{decode, encode};
    #[test]
    fn preserves_hex_representation_and_input_errors() {
        assert_eq!(encode([]), "");
        assert_eq!(decode("").unwrap(), Vec::<u8>::new());
        let bytes: Vec<u8> = (0..=255).collect();
        let text = encode(&bytes);
        assert_eq!(text.len(), 512);
        assert!(
            text.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        );
        assert_eq!(decode(&text).unwrap(), bytes);
        assert_eq!(decode(text.to_ascii_uppercase()).unwrap(), bytes);
        assert!(decode("0").is_err());
        assert!(decode("0g").is_err());
        assert!(decode("é").is_err());
        assert!(decode("00 1").is_err());
    }
}
