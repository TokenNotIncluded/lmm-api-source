//! Streaming construction of a bounded CSV from already validated native receipts.
use super::{Error, ValidatedRequest};

const MAX_BYTES: usize = 32 * 1024 * 1024;
const COLUMNS: &[&str] = &[
    "request_id",
    "user_id",
    "created_at",
    "state",
    "outcome",
    "provider_http_status",
    "model",
    "group",
    "token_name",
    "price_version",
    "payer_kind",
    "billing_source",
    "platform_sponsored",
    "reserved_credits",
    "gross_settled_credits",
    "refunded_credits",
    "net_consumption_credits",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "cache_write_5m_tokens",
    "cache_write_1h_tokens",
    "cache_write_12h_tokens",
    "output_images",
    "billing_method",
    "tool_kind",
    "successful_tool_requests",
    "grounded_requests",
    "tool_input_tokens",
];

pub(super) struct NativeCsv(Vec<u8>);

fn number(value: Option<i64>) -> String {
    value.map(|n| n.to_string()).unwrap_or_default()
}

impl NativeCsv {
    pub(super) fn new() -> Result<Self, Error> {
        let mut csv = Self(Vec::new());
        csv.row(COLUMNS.iter().copied())?;
        Ok(csv)
    }

    pub(super) fn include(&mut self, request: &ValidatedRequest) -> Result<(), Error> {
        let settled = request.settled();
        let text = settled && request.price.image.is_none() && request.price.tool.is_none();
        let creation = request.usage.cache_creation.filter(|_| text);
        let cells = [
            request.id.clone(),
            request.user.to_string(),
            request.at.to_string(),
            request.state.clone(),
            request.outcome.clone().unwrap_or_default(),
            number(
                request
                    .usage
                    .tool
                    .filter(|_| settled)
                    .map(|tool| i64::from(tool.http_status)),
            ),
            request.price.model.clone(),
            request.price.group.clone(),
            request.token_name.clone(),
            request.price.version.clone(),
            request.payer_kind.clone(),
            if request.subscription() {
                "subscription"
            } else {
                "wallet"
            }
            .into(),
            request.sponsored.to_string(),
            if request.held() { request.reserved } else { 0 }.to_string(),
            number(settled.then_some(request.amounts.gross)),
            number(settled.then_some(request.amounts.refunded)),
            number(settled.then_some(request.amounts.net())),
            number(text.then_some(request.usage.input)),
            number(text.then_some(request.usage.output)),
            number(text.then_some(request.usage.cached)),
            number(creation.map(|c| c.total)),
            number(creation.and_then(|c| c.five_minute)),
            number(creation.and_then(|c| c.one_hour)),
            number(creation.and_then(|c| c.twelve_hour)),
            number(
                request
                    .usage
                    .image
                    .filter(|_| settled)
                    .map(|i| i64::from(i.output_images)),
            ),
            if request.price.tool.is_some() {
                "per_successful_request"
            } else {
                "measured_usage"
            }
            .into(),
            request
                .price
                .tool
                .as_ref()
                .map(|price| price.kind.protocol_name().to_owned())
                .unwrap_or_default(),
            number(
                request
                    .usage
                    .tool
                    .filter(|_| settled)
                    .map(|tool| i64::from(tool.successful_requests)),
            ),
            number(
                request
                    .usage
                    .grounding
                    .filter(|_| settled)
                    .map(|grounding| i64::from(grounding.grounded_requests)),
            ),
            request
                .usage
                .grounding
                .filter(|_| settled)
                .and_then(|g| g.tool_input_tokens)
                .map(|n| n.to_string())
                .unwrap_or_default(),
        ];
        self.row(cells.iter().map(String::as_str))
    }

    fn append(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if self
            .0
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > MAX_BYTES)
        {
            return Err(Error::TooManyBuckets);
        }
        self.0.extend_from_slice(bytes);
        Ok(())
    }

    fn row<'a>(&mut self, cells: impl Iterator<Item = &'a str>) -> Result<(), Error> {
        for (index, cell) in cells.enumerate() {
            if index > 0 {
                self.append(b",")?;
            }
            self.append(b"\"")?;
            let trimmed = cell.trim_start_matches(|c: char| {
                c.is_whitespace() || c.is_control() || c == '\u{feff}'
            });
            if trimmed.starts_with(['=', '+', '-', '@']) {
                self.append(b"'")?;
            }
            for part in cell.split_inclusive('"') {
                self.append(part.as_bytes())?;
                if part.ends_with('"') {
                    self.append(b"\"")?;
                }
            }
            self.append(b"\"")?;
        }
        self.append(b"\r\n")
    }

    pub(super) fn finish(self) -> Vec<u8> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_escapes_quotes_newlines_and_enforces_byte_limit() {
        let mut csv = NativeCsv(Vec::new());
        csv.row(["line\nnext", "a\"b", "\u{feff} =1"].into_iter())
            .unwrap();
        assert_eq!(
            String::from_utf8(csv.0).unwrap(),
            "\"line\nnext\",\"a\"\"b\",\"'\u{feff} =1\"\r\n"
        );
        let mut csv = NativeCsv(vec![0; MAX_BYTES - 1]);
        assert!(csv.append(b"ab").is_err());
        assert_eq!(csv.0.len(), MAX_BYTES - 1);
    }
}
