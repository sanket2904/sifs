//! Bytes to text, for every reader: a byte-order mark decides UTF-8 or
//! UTF-16; UTF-16 without one is recognised from its zero bytes; other zero
//! bytes mean the file isn't text; valid UTF-8 is used as is; anything else
//! is guessed (chardetng) and decoded (encoding_rs).

use super::ReadError;
use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8};
use std::io::Read;
use std::path::Path;

/// How much of the start of a file is looked at for zero bytes.
const SNIFF: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    /// The encoding's name, when it was not UTF-8.
    pub encoding: Option<&'static str>,
}

/// Reads at most max_bytes of path; a longer file is TooLarge.
pub fn read_capped(path: &Path, max_bytes: u64) -> Result<Vec<u8>, ReadError> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(ReadError::TooLarge);
    }
    Ok(bytes)
}

/// Decodes bytes. hint is an encoding the file declares (HTML's meta
/// charset): used before guessing, though a byte-order mark still wins.
pub fn decode(bytes: &[u8], hint: Option<&'static Encoding>) -> Result<Decoded, ReadError> {
    if let Some((encoding, bom)) = Encoding::for_bom(bytes) {
        return Ok(finish(encoding, &bytes[bom..]));
    }
    let head = &bytes[..bytes.len().min(SNIFF)];
    if head.contains(&0) {
        if let Some(encoding) = utf16_without_bom(bytes) {
            return Ok(finish(encoding, bytes));
        }
        return Err(ReadError::Binary);
    }
    if bytes[head.len()..].contains(&0) {
        return Err(ReadError::Binary);
    }
    // A declared UTF-16 can't be right for bytes with no zeros (browsers
    // treat it as UTF-8 too).
    if let Some(encoding) = hint.filter(|e| *e != UTF_16LE && *e != UTF_16BE) {
        return Ok(finish(encoding, bytes));
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(Decoded { text: clean(text), encoding: None });
    }
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(bytes, true);
    Ok(finish(detector.guess(None, true), bytes))
}

/// UTF-16 has a zero byte for every ASCII character, all on even positions
/// (big-endian) or all on odd ones (little-endian). Binary files have zeros
/// on both. A candidate must also decode cleanly into mostly printable text.
fn utf16_without_bom(bytes: &[u8]) -> Option<&'static Encoding> {
    let head = &bytes[..bytes.len().min(SNIFF) & !1];
    let (mut even, mut odd) = (0usize, 0usize);
    for (i, b) in head.iter().enumerate() {
        if *b == 0 {
            if i % 2 == 0 {
                even += 1
            } else {
                odd += 1
            }
        }
    }
    let encoding = if odd > 0 && even * 20 <= odd {
        UTF_16LE
    } else if even > 0 && odd * 20 <= even {
        UTF_16BE
    } else {
        return None;
    };
    let even_len = &bytes[..bytes.len() & !1];
    let text = encoding.decode_without_bom_handling_and_without_replacement(even_len)?;
    let total = text.chars().count().max(1);
    let control = text.chars().filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')).count();
    (control * 50 <= total).then_some(encoding)
}

fn finish(encoding: &'static Encoding, bytes: &[u8]) -> Decoded {
    let (text, _) = encoding.decode_without_bom_handling(bytes);
    Decoded { text: clean(&text), encoding: (encoding != UTF_8).then(|| encoding.name()) }
}

/// Drops control characters other than newline and tab (so "\r\n" becomes
/// "\n"), and any stray byte-order mark.
fn clean(text: &str) -> String {
    if !text.chars().any(|c| (c.is_control() && c != '\n' && c != '\t') || c == '\u{feff}') {
        return text.to_string();
    }
    text.chars().filter(|c| !((c.is_control() && *c != '\n' && *c != '\t') || *c == '\u{feff}')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str, le: bool) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() }).collect()
    }

    #[test]
    fn valid_utf8_is_used_as_is() {
        let d = decode("naïve café, किराया".as_bytes(), None).unwrap();
        assert_eq!(d, Decoded { text: "naïve café, किराया".into(), encoding: None });
    }

    #[test]
    fn a_byte_order_mark_decides_utf8_or_utf16() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend("café".as_bytes());
        assert_eq!(decode(&bytes, None).unwrap(), Decoded { text: "café".into(), encoding: None });
        let mut le = vec![0xFF, 0xFE];
        le.extend(utf16("alpaca café", true));
        assert_eq!(decode(&le, None).unwrap(), Decoded { text: "alpaca café".into(), encoding: Some("UTF-16LE") });
        let mut be = vec![0xFE, 0xFF];
        be.extend(utf16("narwhal café", false));
        assert_eq!(decode(&be, None).unwrap(), Decoded { text: "narwhal café".into(), encoding: Some("UTF-16BE") });
        // A mark wins over what the file declares.
        assert_eq!(decode(&le, Some(encoding_rs::WINDOWS_1252)).unwrap().text, "alpaca café");
    }

    #[test]
    fn utf16_without_a_mark_is_recognised_from_its_zero_bytes() {
        let text = "pangolin notes\nline two, café\n";
        assert_eq!(
            decode(&utf16(text, true), None).unwrap(),
            Decoded { text: text.into(), encoding: Some("UTF-16LE") }
        );
        assert_eq!(
            decode(&utf16(text, false), None).unwrap(),
            Decoded { text: text.into(), encoding: Some("UTF-16BE") }
        );
    }

    #[test]
    fn other_zero_bytes_mean_binary() {
        assert!(matches!(decode(b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0\x03\0>\0", None), Err(ReadError::Binary)));
        // Zeros after the part sniffed for UTF-16 still mean binary.
        let mut late = vec![b'a'; SNIFF + 10];
        late.push(0);
        assert!(matches!(decode(&late, None), Err(ReadError::Binary)));
    }

    #[test]
    fn other_encodings_are_guessed_and_recorded() {
        // "Crème brûlée à la française, déjà vu" in Latin-1 / Windows-1252.
        let latin1: Vec<u8> = "Crème brûlée à la française, déjà vu, très célèbre"
            .chars()
            .map(|c| u8::try_from(c as u32).unwrap())
            .collect();
        let d = decode(&latin1, None).unwrap();
        assert_eq!(d.text, "Crème brûlée à la française, déjà vu, très célèbre");
        assert_eq!(d.encoding, Some("windows-1252"));
    }

    #[test]
    fn a_declared_encoding_is_honoured_before_guessing() {
        // 0x92 is a right quote in Windows-1252.
        let d = decode(b"it\x92s caf\xe9", Some(encoding_rs::WINDOWS_1252)).unwrap();
        assert_eq!(d, Decoded { text: "it\u{2019}s café".into(), encoding: Some("windows-1252") });
        // A declared UTF-16 on bytes without zeros is not believed.
        assert_eq!(decode(b"plain", Some(UTF_16LE)).unwrap().text, "plain");
    }

    #[test]
    fn control_characters_other_than_newline_and_tab_are_removed() {
        assert_eq!(decode(b"a\r\nb\tc\x07d\x1b[0m", None).unwrap().text, "a\nb\tcd[0m");
    }
}
