//! What the helper sends SIFS on its stdout: a marker, then frames (see
//! supervise.rs).

use crate::extract::readers::macos::supervise;

pub(super) const MARKER: &[u8] = b"\0SIFS-PDF-1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Frame {
    Pages(u32),
    Fact(String, String),
    Page(u32, String),
    /// A page with no text: a scan, for OCR, or blank.
    Textless(u32),
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
            Frame::Textless(index) => (b't', index.to_le_bytes().to_vec()),
            Frame::Cut => (b'c', Vec::new()),
            Frame::Locked => (b'l', Vec::new()),
            Frame::Unopenable => (b'u', Vec::new()),
            Frame::End => (b'e', Vec::new()),
        };
        supervise::encode(tag, &body)
    }
}

pub(super) type Decoder = supervise::Decoder<Frame>;

impl Default for Decoder {
    fn default() -> Decoder {
        Decoder::new(MARKER, decode)
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
        b't' => Frame::Textless(index(body)?),
        b'c' => Frame::Cut,
        b'l' => Frame::Locked,
        b'u' => Frame::Unopenable,
        b'e' => Frame::End,
        _ => return Err(format!("an unknown frame ({tag})")),
    })
}
