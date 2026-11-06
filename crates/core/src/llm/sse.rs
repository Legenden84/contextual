//! A small incremental Server-Sent Events decoder (WHATWG rules: `event:`, `data:` lines joined
//! with `\n`, blank line dispatches, `:` comments, `\n` / `\r\n` / `\r` line endings).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// `event:` name, if any.
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseDecoder {
    buf: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
    has_data: bool,
    /// The previous chunk ended with `\r`; a leading `\n` belongs to it.
    after_cr: bool,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds bytes; returns the events completed by them.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        let mut out = Vec::new();
        let mut bytes = chunk;
        if self.after_cr && bytes.first() == Some(&b'\n') {
            bytes = &bytes[1..];
        }
        self.after_cr = false;
        self.buf.extend_from_slice(bytes);
        let mut start = 0;
        let mut i = 0;
        while i < self.buf.len() {
            let b = self.buf[i];
            if b == b'\n' || b == b'\r' {
                let line = String::from_utf8_lossy(&self.buf[start..i]).into_owned();
                let mut next = i + 1;
                if b == b'\r' {
                    if next < self.buf.len() {
                        if self.buf[next] == b'\n' {
                            next += 1;
                        }
                    } else {
                        self.after_cr = true;
                    }
                }
                self.line(&line, &mut out);
                start = next;
                i = next;
            } else {
                i += 1;
            }
        }
        self.buf.drain(..start);
        out
    }

    /// End of stream: dispatches a final event that wasn't followed by a blank line.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let line = String::from_utf8_lossy(&std::mem::take(&mut self.buf)).into_owned();
            self.line(&line, &mut out);
        }
        self.dispatch(&mut out);
        out
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        if self.has_data {
            out.push(SseEvent {
                event: self.event.take(),
                data: self.data.join("\n"),
            });
        }
        self.event = None;
        self.data.clear();
        self.has_data = false;
    }

    fn line(&mut self, line: &str, out: &mut Vec<SseEvent>) {
        if line.is_empty() {
            self.dispatch(out);
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.find(':') {
            Some(i) => {
                let v = &line[i + 1..];
                (&line[..i], v.strip_prefix(' ').unwrap_or(v))
            }
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => {
                self.data.push(value.to_string());
                self.has_data = true;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_split_chunks() {
        let mut d = SseDecoder::new();
        let mut all = d.push(b"event: a\r\ndata: {\"x\"");
        all.extend(d.push(b":1}\r"));
        all.extend(d.push(b"\n\r\n: comment\n\ndata: one\ndata: two\n\ndata: [DONE]"));
        all.extend(d.finish());
        assert_eq!(
            all,
            vec![
                SseEvent {
                    event: Some("a".into()),
                    data: "{\"x\":1}".into()
                },
                SseEvent {
                    event: None,
                    data: "one\ntwo".into()
                },
                SseEvent {
                    event: None,
                    data: "[DONE]".into()
                },
            ]
        );
    }

    #[test]
    fn multibyte_split() {
        let mut d = SseDecoder::new();
        let bytes = "data: é\n\n".as_bytes();
        let mut all = d.push(&bytes[..7]);
        all.extend(d.push(&bytes[7..]));
        assert_eq!(all[0].data, "é");
    }
}
