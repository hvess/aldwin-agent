//! Server-sent events off a response body: each event's `data`, the only
//! field a provider stream is read for.

use std::string::FromUtf8Error;

/// Turns an SSE stream's chunks into events. A line end is looked for only
/// in bytes not scanned before, and read bytes are dropped only once they
/// are most of the buffer, so an event costs time linear in its size
/// however finely it arrives: one `data:` line can be a whole tool call.
#[derive(Debug, Default)]
pub(crate) struct SseDecoder {
    buf: Vec<u8>,
    /// Bytes before this are read.
    start: usize,
    /// Bytes before this hold no line end still to read.
    scanned: usize,
    /// The event being read: its `data` lines, joined with `\n`.
    data: Option<Vec<u8>>,
    /// Whether a leading byte-order mark has been looked for.
    begun: bool,
}

const BOM: &[u8] = b"\xEF\xBB\xBF";

impl SseDecoder {
    pub(crate) fn push(&mut self, chunk: &[u8]) {
        if self.start > self.buf.len() / 2 {
            self.buf.drain(..self.start);
            self.scanned -= self.start;
            self.start = 0;
        }
        self.buf.extend_from_slice(chunk);
    }

    /// The next whole event's data, or `None` until more bytes arrive. Lines
    /// end in `\n`, `\r\n` or `\r`, and a leading byte-order mark is
    /// dropped; a comment and any field but `data` is skipped.
    pub(crate) fn next_event(&mut self) -> Option<Result<String, FromUtf8Error>> {
        if !self.begun {
            if self.buf.len() < BOM.len() && BOM.starts_with(&self.buf) {
                return None;
            }
            if self.buf.starts_with(BOM) {
                self.start = BOM.len();
                self.scanned = self.start;
            }
            self.begun = true;
        }
        loop {
            let Some(at) = self.buf[self.scanned..]
                .iter()
                .position(|&b| b == b'\n' || b == b'\r')
            else {
                self.scanned = self.buf.len();
                return None;
            };
            let end = self.scanned + at;
            // A `\r` ends the line alone unless a `\n` follows it, which the
            // next chunk may bring.
            let next = match (self.buf[end], self.buf.get(end + 1)) {
                (b'\r', Some(b'\n')) => end + 2,
                (b'\r', None) => {
                    self.scanned = end;
                    return None;
                }
                _ => end + 1,
            };
            let line = &self.buf[self.start..end];
            let (field, value) = match line.iter().position(|&b| b == b':') {
                Some(colon) => {
                    let value = &line[colon + 1..];
                    (&line[..colon], value.strip_prefix(b" ").unwrap_or(value))
                }
                None => (line, &[][..]),
            };
            let blank = line.is_empty();
            if field == b"data" {
                match &mut self.data {
                    Some(data) => {
                        data.push(b'\n');
                        data.extend_from_slice(value);
                    }
                    None => self.data = Some(value.to_vec()),
                }
            }
            self.start = next;
            self.scanned = self.start;
            if blank {
                if let Some(data) = self.data.take() {
                    return Some(String::from_utf8(data));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::slice;

    fn events(chunks: &[&[u8]]) -> Vec<String> {
        // Loops, not a chain: each chunk is pushed before its events are read.
        let mut sse = SseDecoder::default();
        let mut out = Vec::new();
        for chunk in chunks {
            sse.push(chunk);
            while let Some(event) = sse.next_event() {
                out.push(event.unwrap());
            }
        }
        out
    }

    #[test]
    fn an_event_is_its_data_lines_joined_and_the_rest_is_skipped() {
        assert_eq!(
            events(&[b": keepalive\n\nevent: delta\ndata: {\"a\":\ndata:1}\nid: 7\n\n"]),
            ["{\"a\":\n1}"]
        );
    }

    #[test]
    fn an_event_split_anywhere_reads_the_same() {
        let whole = b"data: first\r\n\r\ndata: second\n\n";
        for cut in 0..whole.len() {
            let (a, b) = whole.split_at(cut);
            assert_eq!(events(&[a, b]), ["first", "second"], "cut at {cut}");
        }
    }

    #[test]
    fn a_line_arriving_a_byte_at_a_time_is_scanned_once() {
        let line = format!("data: {}\n\n", "x".repeat(100_000));
        let mut sse = SseDecoder::default();
        for byte in line.as_bytes() {
            sse.push(slice::from_ref(byte));
            if let Some(event) = sse.next_event() {
                assert_eq!(event.unwrap().len(), 100_000);
            }
            assert!(sse.scanned == sse.buf.len(), "every byte scanned once");
        }
    }

    #[test]
    fn a_lone_carriage_return_ends_a_line_and_a_byte_order_mark_is_dropped() {
        let whole = b"\xEF\xBB\xBFdata: first\r\rdata: second\r\n\r\n";
        for cut in 0..whole.len() {
            let (a, b) = whole.split_at(cut);
            assert_eq!(events(&[a, b]), ["first", "second"], "cut at {cut}");
        }
    }

    #[test]
    fn an_unfinished_event_is_not_an_event() {
        assert!(events(&[b"data: half"]).is_empty());
        assert!(events(&[b"data: half\n"]).is_empty());
    }
}
