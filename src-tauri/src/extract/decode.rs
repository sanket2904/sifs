//! Bytes to text, for every reader: a byte-order mark decides UTF-8 or
//! UTF-16; UTF-16 without one is recognised from its zero bytes; other zero
//! bytes mean the file isn't text; UTF-8 is used as is, with a few invalid
//! bytes replaced; anything else is guessed (chardetng) and decoded
//! (encoding_rs). Line breaks become "\n", other control characters spaces.

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
    // UTF-8 but for a few bad bytes (a file two tools appended to, say):
    // kept as UTF-8, the bad bytes replaced, rather than all of it guessed.
    let (non_ascii, invalid) = utf8_damage(bytes);
    if non_ascii > 0 && invalid * 20 <= non_ascii {
        return Ok(Decoded { text: clean(&String::from_utf8_lossy(bytes)), encoding: None });
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
    // The zeros must also be a real share of their side, not a stray one or
    // two: at least one in 128. Hindi's are just its spaces and punctuation;
    // Japanese, with no spaces, has only its line breaks and ASCII (about
    // one in 40).
    let enough = |zeros: usize| zeros >= 2 && zeros * 128 >= head.len() / 2;
    let encoding = if enough(odd) && even * 20 <= odd {
        UTF_16LE
    } else if enough(even) && odd * 20 <= even {
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

/// How many valid characters outside ASCII bytes hold, and how many of
/// their bytes are not UTF-8.
fn utf8_damage(bytes: &[u8]) -> (usize, usize) {
    let (mut non_ascii, mut invalid) = (0, 0);
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(valid) => return (non_ascii + valid.chars().filter(|c| !c.is_ascii()).count(), invalid),
            Err(e) => {
                let valid = std::str::from_utf8(&rest[..e.valid_up_to()]).unwrap_or_default();
                non_ascii += valid.chars().filter(|c| !c.is_ascii()).count();
                let bad = e.error_len().unwrap_or(rest.len() - e.valid_up_to());
                invalid += bad;
                rest = &rest[e.valid_up_to() + bad..];
            }
        }
    }
}

/// Line breaks as "\n" ("\r\n", and a lone "\r" from old Mac files), other
/// control characters (form feeds, escapes) as spaces, so words either side
/// stay apart, and no stray byte-order marks.
fn clean(text: &str) -> String {
    if !text.chars().any(|c| (c.is_control() && c != '\n' && c != '\t') || c == '\u{feff}') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\n' | '\t' => out.push(c),
            '\u{feff}' => {}
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
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
    fn line_breaks_and_other_control_characters_keep_words_apart() {
        // "\r\n" and a lone "\r" (old Mac files) are line breaks; a form feed
        // or other control character is a space.
        assert_eq!(decode(b"a\r\nb\rc\td\x0ce\x07f\x1b[0m", None).unwrap().text, "a\nb\nc\td e f [0m");
        assert_eq!(decode(b"page one end\x0cpage two", None).unwrap().text, "page one end page two");
        // A byte-order mark inside the text is dropped.
        assert_eq!(decode("a\u{feff}b".as_bytes(), None).unwrap().text, "ab");
    }

    #[test]
    fn a_stray_zero_byte_means_binary_not_utf16() {
        assert!(matches!(decode(b"abc\0def", None), Err(ReadError::Binary)));
        let mut text = b"An ordinary note, with one stray zero byte in it. ".repeat(40);
        text[201] = 0;
        assert!(matches!(decode(&text, None), Err(ReadError::Binary)));
        // Real UTF-16 without a mark is still recognised, Hindi included
        // (its zero bytes are the spaces and punctuation).
        let hindi = "मकान का किराया दस हज़ार रुपये है और बिजली का बिल अलग है।\n".repeat(5);
        let d = decode(&utf16(&hindi, true), None).unwrap();
        assert_eq!((d.text.as_str(), d.encoding), (hindi.as_str(), Some("UTF-16LE")));
        // Two stray zeros in a longer note are not UTF-16 either.
        text[901] = 0;
        assert!(matches!(decode(&text, None), Err(ReadError::Binary)));
    }

    #[test]
    fn utf16_japanese_without_a_mark_is_recognised() {
        // No spaces: its only zero bytes are the line breaks' and the ASCII's.
        let ja = "東京都の天気は晴れ、明日は雨が降るでしょう。今週の予定を確認してください。\n".repeat(40);
        for little in [true, false] {
            let d = decode(&utf16(&ja, little), None).unwrap();
            assert_eq!(d.text, ja);
            assert_eq!(d.encoding, Some(if little { "UTF-16LE" } else { "UTF-16BE" }));
        }
    }

    #[test]
    fn utf8_with_a_stray_invalid_byte_stays_utf8() {
        let mut body = "résumé naïve café ".repeat(200).into_bytes();
        body.push(0xE9);
        let d = decode(&body, None).unwrap();
        assert_eq!(d.encoding, None);
        assert!(d.text.starts_with("résumé naïve café"), "{:?}", &d.text[..40]);
        assert!(d.text.ends_with('\u{fffd}'));
        // Without any UTF-8 in it, a lone high byte is still Latin-1.
        let mut ascii = b"the menu of the small cafe on the corner ".repeat(50);
        ascii.extend(b"caf\xe9");
        let d = decode(&ascii, None).unwrap();
        assert_eq!(d.encoding, Some("windows-1252"));
        assert!(d.text.ends_with("café"));
    }
}
