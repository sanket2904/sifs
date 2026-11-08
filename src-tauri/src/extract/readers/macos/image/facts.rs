//! An image's facts from its properties, as ImageIO gives them: checked and
//! written the way SIFS stores them. A value that isn't what it claims (a
//! date of zeros, a position off the globe) gives no fact.

use super::frame::FactKey;
use crate::extract::MAX_FACT_CHARS;

/// The properties a fact comes from, as read.
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct Properties {
    pub original: Option<String>,
    pub original_offset: Option<String>,
    pub digitized: Option<String>,
    pub digitized_offset: Option<String>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub latitude: Option<f64>,
    pub latitude_ref: Option<String>,
    pub longitude: Option<f64>,
    pub longitude_ref: Option<String>,
    pub orientation: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
}

/// The facts, in the order they are sent.
pub(super) fn facts(p: &Properties) -> Vec<(FactKey, String)> {
    let taken = p
        .original
        .as_deref()
        .and_then(|date| taken(date, p.original_offset.as_deref()))
        .or_else(|| p.digitized.as_deref().and_then(|date| taken(date, p.digitized_offset.as_deref())));
    let position = position(
        (p.latitude, p.latitude_ref.as_deref()),
        (p.longitude, p.longitude_ref.as_deref()),
    );
    let mut facts = Vec::new();
    facts.extend(taken.map(|t| (FactKey::Taken, t)));
    facts.extend(label(p.make.as_deref()).map(|m| (FactKey::CameraMake, m)));
    facts.extend(label(p.model.as_deref()).map(|m| (FactKey::CameraModel, m)));
    if let Some((latitude, longitude)) = position {
        facts.push((FactKey::Latitude, latitude));
        facts.push((FactKey::Longitude, longitude));
    }
    facts.extend(p.orientation.and_then(orientation).map(|o| (FactKey::Orientation, o.to_string())));
    facts.extend(p.width.and_then(pixels).map(|w| (FactKey::Width, w.to_string())));
    facts.extend(p.height.and_then(pixels).map(|h| (FactKey::Height, h.to_string())));
    facts
}

/// An EXIF date ("2025:12:24 18:30:05") and its optional offset ("-05:00")
/// as ISO 8601: "2025-12-24T18:30:05-05:00". An offset that isn't one is
/// left out; a date that isn't one gives nothing.
pub(super) fn taken(date: &str, offset: Option<&str>) -> Option<String> {
    let date = date.trim_matches(|c: char| c.is_whitespace() || c == '\0').as_bytes();
    if date.len() != 19 || !matches!((date[4], date[7], date[10], date[13], date[16]), (b':', b':', b' ', b':', b':')) {
        return None;
    }
    let field = |from: usize, len: usize| -> Option<u32> {
        let digits = &date[from..from + len];
        digits.iter().all(u8::is_ascii_digit).then(|| std::str::from_utf8(digits).ok()?.parse().ok()).flatten()
    };
    let (year, month, day) = (field(0, 4)?, field(5, 2)?, field(8, 2)?);
    let (hour, minute, second) = (field(11, 2)?, field(14, 2)?, field(17, 2)?);
    if year == 0 || !(1..=12).contains(&month) || day == 0 || day > days_in(year, month) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let offset = offset.and_then(utc_offset).unwrap_or_default();
    Some(format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{offset}"))
}

/// "+09:00" or "-03:30", as written, if it is an offset any clock uses.
fn utc_offset(offset: &str) -> Option<String> {
    let offset = offset.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    let b = offset.as_bytes();
    if b.len() != 6 || !matches!(b[0], b'+' | b'-') || b[3] != b':' {
        return None;
    }
    if ![1, 2, 4, 5].iter().all(|&i| b[i].is_ascii_digit()) {
        return None;
    }
    let hours = (b[1] - b'0') * 10 + (b[2] - b'0');
    let minutes = (b[4] - b'0') * 10 + (b[5] - b'0');
    (hours <= 14 && minutes <= 59).then(|| offset.to_string())
}

fn days_in(year: u32, month: u32) -> u32 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A latitude or longitude in degrees and its reference ("N", "S", "E",
/// "W"), signed: south and west negative. Nothing past max (90 or 180) or
/// not a number.
pub(super) fn coordinate(degrees: f64, reference: Option<&str>, max: f64) -> Option<f64> {
    if !degrees.is_finite() || degrees.abs() > max {
        return None;
    }
    let negative = matches!(reference.map(str::trim), Some("S" | "s" | "W" | "w"));
    Some(if negative { -degrees.abs() } else { degrees })
}

/// Latitude and longitude, six decimals each. Exactly 0, 0 is what a
/// camera without a fix writes: no position.
pub(super) fn position(
    latitude: (Option<f64>, Option<&str>),
    longitude: (Option<f64>, Option<&str>),
) -> Option<(String, String)> {
    let latitude = coordinate(latitude.0?, latitude.1, 90.0)?;
    let longitude = coordinate(longitude.0?, longitude.1, 180.0)?;
    if latitude == 0.0 && longitude == 0.0 {
        return None;
    }
    // Adding 0.0 turns -0.0 into 0.0, which prints without its sign.
    Some((format!("{:.6}", latitude + 0.0), format!("{:.6}", longitude + 0.0)))
}

/// EXIF orientation: 1 to 8.
pub(super) fn orientation(value: f64) -> Option<u8> {
    (value.fract() == 0.0 && (1.0..=8.0).contains(&value)).then_some(value as u8)
}

/// A side in pixels: a whole number above 0.
pub(super) fn pixels(value: f64) -> Option<u64> {
    (value.fract() == 0.0 && value >= 1.0 && value <= u64::MAX as f64).then_some(value as u64)
}

/// A camera's make or model, trimmed, if there is one.
fn label(value: Option<&str>) -> Option<String> {
    let value = value?.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    (!value.is_empty()).then(|| value.chars().take(MAX_FACT_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_is_iso_8601_with_its_offset_when_it_has_one() {
        assert_eq!(taken("2025:12:24 18:30:05", None).as_deref(), Some("2025-12-24T18:30:05"));
        assert_eq!(taken("2025:12:24 18:30:05", Some("+09:00")).as_deref(), Some("2025-12-24T18:30:05+09:00"));
        assert_eq!(taken("2025:12:24 18:30:05", Some("-03:30")).as_deref(), Some("2025-12-24T18:30:05-03:30"));
        assert_eq!(taken("2025:12:24 18:30:05", Some("+00:00")).as_deref(), Some("2025-12-24T18:30:05+00:00"));
        assert_eq!(taken(" 2024:02:29 00:00:00\0", Some(" +14:00 ")).as_deref(), Some("2024-02-29T00:00:00+14:00"));
        assert_eq!(taken("2025:12:24 23:59:59", Some("+15:00")).as_deref(), Some("2025-12-24T23:59:59"), "no such offset");
        assert_eq!(taken("2025:12:24 23:59:59", Some("09:00")).as_deref(), Some("2025-12-24T23:59:59"), "no sign");
        assert_eq!(taken("2025:12:24 23:59:59", Some("+9:00")).as_deref(), Some("2025-12-24T23:59:59"));
        assert_eq!(taken("2025:12:24 23:59:59", Some("009:00")).as_deref(), Some("2025-12-24T23:59:59"));
        assert_eq!(taken("2025:12:24 23:59:59", Some("-03:60")).as_deref(), Some("2025-12-24T23:59:59"));
        assert_eq!(taken("2025:12:24 23:59:59", Some("")).as_deref(), Some("2025-12-24T23:59:59"));
    }

    #[test]
    fn a_date_that_isnt_one_gives_nothing() {
        for date in [
            "0000:00:00 00:00:00",
            "0000:01:01 00:00:00",
            "2025:13:01 00:00:00",
            "2025:00:01 00:00:00",
            "2025:01:00 00:00:00",
            "2025:02:29 00:00:00",
            "1900:02:29 00:00:00",
            "2025:04:31 00:00:00",
            "2025:01:32 00:00:00",
            "2025:01:01 24:00:00",
            "2025:01:01 00:60:00",
            "2025:01:01 00:00:60",
            "2025-01-01 00:00:00",
            "2025:01:01T00:00:00",
            "2025:1:01 00:00:00",
            "2025:01:01 00:00:0",
            "+025:01:01 00:00:00",
            "2025:01:01 00:00:+1",
            "    :  :     :  :  ",
            "",
        ] {
            assert_eq!(taken(date, Some("+01:00")), None, "{date:?}");
        }
        assert_eq!(taken("2000:02:29 12:00:00", None).as_deref(), Some("2000-02-29T12:00:00"), "a leap century");
        assert_eq!(taken("9999:12:31 23:59:59", None).as_deref(), Some("9999-12-31T23:59:59"));
    }

    #[test]
    fn the_original_date_comes_first_then_the_digitized_one() {
        let both = Properties {
            original: Some("2025:12:24 18:30:05".into()),
            original_offset: Some("+01:00".into()),
            digitized: Some("2026:01:01 00:00:00".into()),
            digitized_offset: Some("-08:00".into()),
            ..Properties::default()
        };
        assert_eq!(facts(&both), [(FactKey::Taken, "2025-12-24T18:30:05+01:00".to_string())]);
        let bad_original = Properties { original: Some("0000:00:00 00:00:00".into()), ..both.clone() };
        assert_eq!(facts(&bad_original), [(FactKey::Taken, "2026-01-01T00:00:00-08:00".to_string())]);
        let digitized = Properties { original: None, original_offset: None, ..both };
        assert_eq!(facts(&digitized), [(FactKey::Taken, "2026-01-01T00:00:00-08:00".to_string())]);
    }

    #[test]
    fn south_and_west_are_negative() {
        assert_eq!(coordinate(64.1466, Some("N"), 90.0), Some(64.1466));
        assert_eq!(coordinate(22.9068, Some("S"), 90.0), Some(-22.9068));
        assert_eq!(coordinate(43.1729, Some("W"), 180.0), Some(-43.1729));
        assert_eq!(coordinate(139.7671, Some("E"), 180.0), Some(139.7671));
        assert_eq!(coordinate(-43.1729, Some("W"), 180.0), Some(-43.1729), "already signed");
        assert_eq!(coordinate(139.7671, None, 180.0), Some(139.7671));
        let rio = position((Some(22.9068), Some("S")), (Some(43.1729), Some("W")));
        assert_eq!(rio, Some(("-22.906800".to_string(), "-43.172900".to_string())));
        let sydney = position((Some(33.8568), Some("S")), (Some(151.2153), Some("E")));
        assert_eq!(sydney, Some(("-33.856800".to_string(), "151.215300".to_string())));
        let reykjavik = position((Some(64.1466), Some("N")), (Some(21.9426), Some("W")));
        assert_eq!(reykjavik, Some(("64.146600".to_string(), "-21.942600".to_string())));
    }

    #[test]
    fn a_position_off_the_globe_or_at_zero_zero_gives_nothing() {
        let at = |lat: f64, lon: f64| position((Some(lat), Some("N")), (Some(lon), Some("E")));
        assert_eq!(at(0.0, 0.0), None, "a camera without a fix");
        assert_eq!(position((Some(0.0), Some("S")), (Some(0.0), Some("W"))), None);
        assert_eq!(at(90.5, 10.0), None);
        assert_eq!(at(10.0, 180.5), None);
        assert_eq!(at(f64::NAN, 10.0), None);
        assert_eq!(at(10.0, f64::INFINITY), None);
        assert_eq!(position((None, None), (Some(10.0), Some("E"))), None, "half a position");
        assert_eq!(position((Some(10.0), Some("N")), (None, None)), None);
        assert_eq!(at(90.0, 180.0), Some(("90.000000".to_string(), "180.000000".to_string())));
        assert_eq!(at(0.0, 10.0), Some(("0.000000".to_string(), "10.000000".to_string())), "the equator is a place");
        let equator = position((Some(0.0), Some("S")), (Some(10.0), Some("E")));
        assert_eq!(equator, Some(("0.000000".to_string(), "10.000000".to_string())), "no minus zero");
    }

    #[test]
    fn orientation_is_one_to_eight_and_sizes_are_positive() {
        assert_eq!((1..=8).map(|n| orientation(n as f64)).collect::<Vec<_>>(), (1..=8).map(Some).collect::<Vec<_>>());
        for bad in [0.0, 9.0, -1.0, 6.5, f64::NAN] {
            assert_eq!(orientation(bad), None, "{bad}");
        }
        assert_eq!(pixels(1.0), Some(1));
        assert_eq!(pixels(4032.0), Some(4032));
        for bad in [0.0, -5.0, 0.5, f64::NAN, f64::INFINITY] {
            assert_eq!(pixels(bad), None, "{bad}");
        }
    }

    #[test]
    fn camera_and_size_facts_are_sent_with_the_rest() {
        let p = Properties {
            make: Some(" Apple\0".into()),
            model: Some("iPhone 15 Pro".into()),
            latitude: Some(35.6812),
            latitude_ref: Some("N".into()),
            longitude: Some(139.7671),
            longitude_ref: Some("E".into()),
            orientation: Some(6.0),
            width: Some(4032.0),
            height: Some(3024.0),
            ..Properties::default()
        };
        let got = facts(&p);
        let got: Vec<(&str, &str)> = got.iter().map(|(k, v)| (k.name(), v.as_str())).collect();
        assert_eq!(
            got,
            [
                ("camera_make", "Apple"),
                ("camera_model", "iPhone 15 Pro"),
                ("latitude", "35.681200"),
                ("longitude", "139.767100"),
                ("orientation", "6"),
                ("width", "4032"),
                ("height", "3024"),
            ]
        );
        let blank = Properties { make: Some("  ".into()), model: Some("\0".into()), ..Properties::default() };
        assert!(facts(&blank).is_empty());
    }
}
