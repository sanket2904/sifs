//! What the helper sends SIFS on its stdout: a marker, then frames of a
//! tag byte, a little-endian u32 length and that many bytes. The marker
//! lets SIFS pass over anything printed before it (the test binary, run as
//! the helper, prints a line first).

pub(super) const MARKER: &[u8] = b"\0SIFS-PDF-1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Frame {
    Pages(u32),
    Fact(String, String),
    Page(u32, String),
    Cut,
    Locked,
    Unopenable,
    End,
}

impl Frame {
    pub(super) fn encode(&self) -> Vec<u8> {
        let (tag, body): (u8, Vec<u8>) = match self {
            Frame::Pages(n) => (b'n', n.to_le_bytes().to_vec()),
            Frame::Fact(key, value) => (b'f', [key.as_bytes(), b"\0", value.as_bytes()].concat()),
            Frame::Page(index, text) => (b'p', [&index.to_le_bytes()[..], text.as_bytes()].concat()),
            Frame::Cut => (b'c', Vec::new()),
            Frame::Locked => (b'l', Vec::new()),
            Frame::Unopenable => (b'u', Vec::new()),
            Frame::End => (b'e', Vec::new()),
        };
        let mut out = Vec::with_capacity(5 + body.len());
        out.push(tag);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }
}

#[derive(Default)]
pub(super) struct Decoder {
    buf: Vec<u8>,
    synced: bool,
}

impl Decoder {
    pub(super) fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, String> {
        self.buf.extend_from_slice(bytes);
        if !self.synced {
            match self.buf.windows(MARKER.len()).position(|w| w == MARKER) {
                Some(at) => {
                    self.buf.drain(..at + MARKER.len());
                    self.synced = true;
                }
                None => {
                    let keep = self.buf.len().min(MARKER.len() - 1);
                    self.buf.drain(..self.buf.len() - keep);
                    return Ok(Vec::new());
                }
            }
        }
        let mut frames = Vec::new();
        let mut at = 0;
        while self.buf.len() - at >= 5 {
            let len = u32::from_le_bytes(self.buf[at + 1..at + 5].try_into().unwrap()) as usize;
            if self.buf.len() - at - 5 < len {
                break;
            }
            frames.push(decode(self.buf[at], &self.buf[at + 5..at + 5 + len])?);
            at += 5 + len;
        }
        self.buf.drain(..at);
        Ok(frames)
    }
}

fn decode(tag: u8, body: &[u8]) -> Result<Frame, String> {
    let text = |b: &[u8]| String::from_utf8(b.to_vec()).map_err(|_| "text that isn't UTF-8".to_string());
    let index = |b: &[u8]| b.get(..4).map(|n| u32::from_le_bytes(n.try_into().unwrap())).ok_or("a short frame");
    Ok(match tag {
        b'n' => Frame::Pages(index(body)?),
        b'f' => {
            let split = body.iter().position(|&b| b == 0).ok_or("a fact without its value")?;
            Frame::Fact(text(&body[..split])?, text(&body[split + 1..])?)
        }
        b'p' => Frame::Page(index(body)?, text(&body[4..])?),
        b'c' => Frame::Cut,
        b'l' => Frame::Locked,
        b'u' => Frame::Unopenable,
        b'e' => Frame::End,
        _ => return Err(format!("an unknown frame ({tag})")),
    })
}
