//! Streaming construction of a bounded CSV from already validated native receipts.
use super::{Error, ValidatedRequest};
use std::{
    borrow::Cow,
    io::{self, Write},
};

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
    "completed_text_requests",
    "grounded_requests",
    "tool_input_tokens",
];

pub(super) struct NativeCsv(csv::Writer<BoundedBytes>);

struct BoundedBytes(Vec<u8>);

fn number(value: Option<i64>) -> String {
    value.map(|n| n.to_string()).unwrap_or_default()
}

impl NativeCsv {
    pub(super) fn new() -> Result<Self, Error> {
        let mut csv = Self::with_bytes(Vec::new());
        csv.row(COLUMNS.iter().copied())?;
        Ok(csv)
    }

    pub(super) fn include(&mut self, request: &ValidatedRequest) -> Result<(), Error> {
        let settled = request.settled();
        let text = settled && request.measured_text();
        let creation = request.usage.cache_creation.filter(|_| text);
        let cells = [
            request.id.clone(),
            request.user.to_string(),
            request.at.to_string(),
            request.state.clone(),
            request.outcome.clone().unwrap_or_default(),
            number(request.provider_status().map(i64::from)),
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
            request.billing_method().into(),
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
                    .text_request
                    .filter(|_| settled)
                    .map(|text| i64::from(text.completed_requests())),
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

    fn with_bytes(bytes: Vec<u8>) -> Self {
        Self(
            csv::WriterBuilder::new()
                .quote_style(csv::QuoteStyle::Always)
                .terminator(csv::Terminator::CRLF)
                .from_writer(BoundedBytes(bytes)),
        )
    }

    fn row<'a>(&mut self, cells: impl Iterator<Item = &'a str>) -> Result<(), Error> {
        let cells = cells.map(|cell| {
            let trimmed = cell.trim_start_matches(|c: char| {
                c.is_whitespace() || c.is_control() || c == '\u{feff}'
            });
            if trimmed.starts_with(['=', '+', '-', '@']) {
                Cow::Owned(format!("'{cell}").into_bytes())
            } else {
                Cow::Borrowed(cell.as_bytes())
            }
        });
        self.0
            .write_record(cells)
            .map_err(|_| Error::TooManyBuckets)?;
        self.0.flush().map_err(|_| Error::TooManyBuckets)
    }

    pub(super) fn finish(self) -> Result<Vec<u8>, Error> {
        self.0
            .into_inner()
            .map(|bytes| bytes.0)
            .map_err(|_| Error::TooManyBuckets)
    }
}

impl BoundedBytes {
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
}

impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.append(bytes)
            .map_err(|_| io::Error::other("export size limit"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_escapes_quotes_newlines_and_enforces_byte_limit() {
        let mut csv = NativeCsv::with_bytes(Vec::new());
        csv.row(["line\nnext", "a\"b", "\u{feff} =1"].into_iter())
            .unwrap();
        assert_eq!(
            String::from_utf8(csv.finish().unwrap()).unwrap(),
            "\"line\nnext\",\"a\"\"b\",\"'\u{feff} =1\"\r\n"
        );
        let mut csv = BoundedBytes(vec![0; MAX_BYTES - 1]);
        assert!(csv.append(b"ab").is_err());
        assert_eq!(csv.0.len(), MAX_BYTES - 1);
    }

    #[test]
    fn csv_buffer_flush_cannot_bypass_the_export_limit() {
        let mut csv = NativeCsv::with_bytes(vec![0; MAX_BYTES - 4]);
        assert!(matches!(
            csv.row(["x"].into_iter()),
            Err(Error::TooManyBuckets)
        ));
        assert!(csv.0.get_ref().0.len() <= MAX_BYTES);
    }

    #[test]
    fn csv_large_field_keeps_quotes_and_utf8_across_writer_buffers() {
        let cell = "中文\"\n".repeat(4096);
        let mut csv = NativeCsv::with_bytes(Vec::new());
        csv.row([cell.as_str(), ""].into_iter()).unwrap();
        assert_eq!(
            String::from_utf8(csv.finish().unwrap()).unwrap(),
            format!("\"{}\",\"\"\r\n", cell.replace('"', "\"\""))
        );
    }
}
