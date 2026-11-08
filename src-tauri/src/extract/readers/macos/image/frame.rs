//! What SIFS and the image helper send each other. SIFS writes one request
//! per image to the helper's stdin; the helper answers on its stdout, after
//! a marker, with frames for that request: facts, at most one text, then
//! the end. Every frame names the request's id. Both ways are frames as in
//! supervise.rs.

use crate::extract::fact;
use crate::extract::readers::macos::supervise::{self, encode};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

pub(super) const MARKER: &[u8] = b"\0SIFS-IMAGE-1\0";

/// A request is an id, four numbers and a path, which macOS keeps under
/// 1,024 bytes.
const MAX_REQUEST: usize = 64 * 1024;

/// The longest frame the helper may send: text at the character cap, four
/// bytes a character.
pub(super) fn max_frame(chars: usize) -> usize {
    chars.saturating_mul(4).saturating_add(64)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Request {
    pub id: u64,
    pub path: PathBuf,
    /// How long it has: the helper ends itself a second after.
    pub deadline_ms: u64,
    pub max_chars: usize,
    pub max_pixels: u64,
}

impl Request {
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(32 + self.path.as_os_str().len());
        for n in [self.id, self.deadline_ms, self.max_chars as u64, self.max_pixels] {
            body.extend_from_slice(&n.to_le_bytes());
        }
        body.extend_from_slice(self.path.as_os_str().as_bytes());
        encode(b'r', &body)
    }
}

pub(super) type Requests = supervise::Decoder<Request>;

/// Requests on the helper's stdin, which carries nothing else.
pub(super) fn requests() -> Requests {
    Requests::new(b"", decode_request, MAX_REQUEST)
}

fn decode_request(tag: u8, body: &[u8]) -> Result<Request, String> {
    if tag != b'r' || body.len() < 32 {
        return Err("a request that isn't one".into());
    }
    let n = |at: usize| u64::from_le_bytes(body[at..at + 8].try_into().unwrap());
    Ok(Request {
        id: n(0),
        deadline_ms: n(8),
        max_chars: n(16).try_into().unwrap_or(usize::MAX),
        max_pixels: n(24),
        path: OsStr::from_bytes(&body[32..]).into(),
    })
}

/// The facts an image may have, and no others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FactKey {
    Taken,
    CameraMake,
    CameraModel,
    Latitude,
    Longitude,
    Orientation,
    Width,
    Height,
}

impl FactKey {
    const ALL: [FactKey; 8] = [
        FactKey::Taken,
        FactKey::CameraMake,
        FactKey::CameraModel,
        FactKey::Latitude,
        FactKey::Longitude,
        FactKey::Orientation,
        FactKey::Width,
        FactKey::Height,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            FactKey::Taken => fact::TAKEN,
            FactKey::CameraMake => fact::CAMERA_MAKE,
            FactKey::CameraModel => fact::CAMERA_MODEL,
            FactKey::Latitude => fact::LATITUDE,
            FactKey::Longitude => fact::LONGITUDE,
            FactKey::Orientation => fact::ORIENTATION,
            FactKey::Width => fact::WIDTH,
            FactKey::Height => fact::HEIGHT,
        }
    }

    /// Its bit in a set of keys.
    pub(super) fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// How the helper's read of an image ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Outcome {
    Read,
    /// Too small for a letter: no OCR.
    Small,
    /// More pixels than the request's cap: not decoded.
    TooManyPixels,
    /// Not an image ImageIO can open, or one with no image in it.
    CantOpen,
    /// Vision failed, and why.
    RecognitionFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Frame {
    Fact { id: u64, key: FactKey, value: String },
    /// cut: some of the text was left out at the character cap.
    Text { id: u64, text: String, cut: bool },
    End { id: u64, outcome: Outcome },
}

impl Frame {
    pub(super) fn id(&self) -> u64 {
        match self {
            Frame::Fact { id, .. } | Frame::Text { id, .. } | Frame::End { id, .. } => *id,
        }
    }

    pub(super) fn encode(&self) -> Vec<u8> {
        let mut body = self.id().to_le_bytes().to_vec();
        let tag = match self {
            Frame::Fact { key, value, .. } => {
                body.push(*key as u8);
                body.extend_from_slice(value.as_bytes());
                b'f'
            }
            Frame::Text { text, cut, .. } => {
                body.push(*cut as u8);
                body.extend_from_slice(text.as_bytes());
                b't'
            }
            Frame::End { outcome, .. } => {
                let (code, why) = match outcome {
                    Outcome::Read => (0, ""),
                    Outcome::Small => (1, ""),
                    Outcome::TooManyPixels => (2, ""),
                    Outcome::CantOpen => (3, ""),
                    Outcome::RecognitionFailed(why) => (4, why.as_str()),
                };
                body.push(code);
                body.extend_from_slice(why.as_bytes());
                b'e'
            }
        };
        encode(tag, &body)
    }
}

pub(super) type Decoder = supervise::Decoder<Frame>;

/// Frames on the helper's stdout, none longer than text of chars
/// characters.
pub(super) fn decoder(chars: usize) -> Decoder {
    Decoder::new(MARKER, decode, max_frame(chars))
}

fn decode(tag: u8, body: &[u8]) -> Result<Frame, String> {
    if !matches!(tag, b'f' | b't' | b'e') {
        return Err(format!("an unknown frame ({tag})"));
    }
    let (Some(id), Some(&byte)) = (body.get(..8), body.get(8)) else {
        return Err("a short frame".into());
    };
    let id = u64::from_le_bytes(id.try_into().unwrap());
    let text = || String::from_utf8(body[9..].to_vec()).map_err(|_| "text that isn't UTF-8".to_string());
    Ok(match tag {
        b'f' => {
            let key = *FactKey::ALL.get(byte as usize).ok_or(format!("an unknown fact ({byte})"))?;
            Frame::Fact { id, key, value: text()? }
        }
        b't' => {
            let cut = match byte {
                0 => false,
                1 => true,
                _ => return Err(format!("an unknown cut ({byte})")),
            };
            Frame::Text { id, text: text()?, cut }
        }
        b'e' => {
            let outcome = match byte {
                0 => Outcome::Read,
                1 => Outcome::Small,
                2 => Outcome::TooManyPixels,
                3 => Outcome::CantOpen,
                4 => Outcome::RecognitionFailed(text()?),
                _ => return Err(format!("an unknown outcome ({byte})")),
            };
            Frame::End { id, outcome }
        }
        _ => unreachable!("checked above"),
    })
}
