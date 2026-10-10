#[derive(Default)]
pub(super) struct LineBuffer {
    bytes: Vec<u8>,
}

impl LineBuffer {
    pub fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
    }

    pub fn next_line(&mut self) -> Option<String> {
        let pos = self.bytes.iter().position(|&b| b == b'\n')?;
        let line: Vec<u8> = self.bytes.drain(..=pos).collect();
        Some(String::from_utf8_lossy(&line).trim().to_owned())
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum SseEvent {
    Data(String),
    Done,
}

pub(super) fn parse_line(line: &str) -> Option<SseEvent> {
    let data = line.strip_prefix("data: ")?.trim();
    if data == "[DONE]" {
        Some(SseEvent::Done)
    } else {
        Some(SseEvent::Data(data.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_split_across_chunks() {
        let mut buf = LineBuffer::default();
        buf.push(b"data: {\"a\"");
        assert_eq!(buf.next_line(), None);
        buf.push(b":1}\n\n");
        assert_eq!(buf.next_line().as_deref(), Some("data: {\"a\":1}"));
        assert_eq!(buf.next_line().as_deref(), Some(""));
        assert_eq!(buf.next_line(), None);
    }

    #[test]
    fn multibyte_character_split_across_chunks() {
        let bytes = "data: é\n".as_bytes();
        let mut buf = LineBuffer::default();
        buf.push(&bytes[..7]);
        assert_eq!(buf.next_line(), None);
        buf.push(&bytes[7..]);
        assert_eq!(buf.next_line().as_deref(), Some("data: é"));
    }

    #[test]
    fn multiple_lines() {
        let mut buf = LineBuffer::default();
        buf.push(b"data: line1\n");
        buf.push(b"data: line2\n");
        assert_eq!(buf.next_line().as_deref(), Some("data: line1"));
        assert_eq!(buf.next_line().as_deref(), Some("data: line2"));
    }

    #[test]
    fn recognises_events() {
        assert_eq!(parse_line("data: [DONE]"), Some(SseEvent::Done));
        assert_eq!(parse_line("data: {}"), Some(SseEvent::Data("{}".into())));
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line(": keep-alive"), None);
    }
}
