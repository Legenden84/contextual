//! Bounded newline framing shared by the workspace client and agent.
//!
//! The buffer belongs to the reader, so cancelling a pending next_line future does not lose
//! a partially received frame. An oversized, invalid UTF-8 or unterminated frame fails the
//! reader permanently. A prefix of a request must never become a dispatchable message.
//!
//! This is a general workspace-frame ceiling. Command payloads have additional, smaller
//! limits in commands::wire. Outgoing queue bounds and command supervision are separate.

use std::io;

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// Allows ordinary workspace scans, file contents and diffs without applying the much smaller
/// command-output limit to them. Larger frames fail explicitly instead of allocating forever.
pub const MAX_WORKSPACE_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// A cancellation-safe, bounded line reader. The limit excludes LF but includes an optional CR.
pub struct BoundedLines<R> {
    reader: R,
    buffer: Vec<u8>,
    limit: usize,
    failed: bool,
}

impl<R: AsyncBufRead + Unpin> BoundedLines<R> {
    pub fn new(reader: R, limit: usize) -> Self {
        Self {
            reader,
            buffer: Vec::new(),
            limit,
            failed: false,
        }
    }

    fn fail(&mut self, kind: io::ErrorKind, message: &'static str) -> io::Error {
        self.failed = true;
        self.buffer.clear();
        io::Error::new(kind, message)
    }

    /// Returns one complete UTF-8 frame without its line terminator.
    ///
    /// Clean EOF is accepted only between frames. Errors never contain received contents.
    pub async fn next_line(&mut self) -> io::Result<Option<String>> {
        if self.failed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Workspace RPC framing is no longer usable.",
            ));
        }
        loop {
            let available = match self.reader.fill_buf().await {
                Ok(available) => available,
                Err(error) => {
                    self.failed = true;
                    self.buffer.clear();
                    return Err(error);
                }
            };
            if available.is_empty() {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                return Err(self.fail(
                    io::ErrorKind::UnexpectedEof,
                    "Workspace RPC connection ended inside a frame.",
                ));
            }

            let newline = available.iter().position(|byte| *byte == b'\n');
            let count = newline.unwrap_or(available.len());
            if count > self.limit.saturating_sub(self.buffer.len()) {
                return Err(self.fail(
                    io::ErrorKind::InvalidData,
                    "Workspace RPC frame exceeds the size limit.",
                ));
            }
            self.buffer.extend_from_slice(&available[..count]);
            self.reader.consume(count + usize::from(newline.is_some()));

            if newline.is_some() {
                if self.buffer.last() == Some(&b'\r') {
                    self.buffer.pop();
                }
                let bytes = std::mem::take(&mut self.buffer);
                return match String::from_utf8(bytes) {
                    Ok(line) => Ok(Some(line)),
                    Err(_) => Err(self.fail(
                        io::ErrorKind::InvalidData,
                        "Workspace RPC frame is not valid UTF-8.",
                    )),
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn exact_limit_and_multiple_frames_are_preserved() {
        let reader = BufReader::with_capacity(2, &b"abc\nxy\n"[..]);
        let mut lines = BoundedLines::new(reader, 3);
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("abc"));
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("xy"));
        assert!(lines.next_line().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn oversized_frame_is_refused_without_waiting_for_newline_or_eof() {
        let (mut writer, reader) = tokio::io::duplex(16);
        writer.write_all(b"abcd").await.unwrap();
        let mut lines = BoundedLines::new(BufReader::new(reader), 3);
        let error = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(lines.buffer.is_empty());
        assert_eq!(
            lines.next_line().await.unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[tokio::test]
    async fn cancelling_a_partial_read_does_not_lose_the_frame_prefix() {
        let (mut writer, reader) = tokio::io::duplex(32);
        let mut lines = BoundedLines::new(BufReader::new(reader), 16);
        writer.write_all(b"par").await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), lines.next_line())
                .await
                .is_err()
        );
        assert_eq!(lines.buffer, b"par");
        writer.write_all(b"tial\n").await.unwrap();
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("partial"));
    }

    #[tokio::test]
    async fn eof_inside_a_frame_does_not_return_a_partial_message() {
        let mut lines = BoundedLines::new(&b"{\"id\":1}"[..], 32);
        assert_eq!(
            lines.next_line().await.unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert!(lines.next_line().await.is_err());
    }

    #[tokio::test]
    async fn utf8_may_span_reader_chunks_but_must_be_valid_at_frame_end() {
        let reader = BufReader::with_capacity(1, "雪\r\n".as_bytes());
        let mut lines = BoundedLines::new(reader, 4);
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("雪"));

        let mut invalid = BoundedLines::new(&b"\xff\n"[..], 4);
        assert_eq!(
            invalid.next_line().await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(invalid.next_line().await.is_err());
    }

    #[tokio::test]
    async fn limits_count_bytes_not_unicode_characters() {
        let mut lines = BoundedLines::new("雪\n".as_bytes(), 2);
        assert_eq!(
            lines.next_line().await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
