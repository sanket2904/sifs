//! The step 2 reviewer's fifth-round tests, kept (handover/step2-review-r5/REPORT.txt).
use super::sample;
use super::*;
use crate::extract::{Part, Reader, MAX_PARTS};
use std::fmt::Write as _;
use std::time::{Duration, Instant};

type Output = (Vec<Part>, Vec<(String, String)>);

fn read_at(reader: &dyn Reader, name: &str, bytes: &[u8], deadline: Instant) -> Result<Output, ReadError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    let mut sink = Sink::new(1_000_000, deadline);
    reader.read(&path, &mut sink)?;
    Ok(sink.finish())
}

fn random_comment(n: usize, seed: &mut u64) -> String {
    let mut s = String::with_capacity(n + 7);
    s.push_str("<!--");
    for _ in 0..n {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        s.push((b'a' + (*seed % 26) as u8) as char);
    }
    s.push_str("-->");
    s
}

/// A workbook whose only shared string is `chars` ideographic spaces
/// (U+3000, blank to str::trim), and one sheet of `cells` cells naming it.
fn blank_shared_xlsx(chars: usize, cells: usize, wrapped: bool, mixed: bool) -> Vec<u8> {
    blank_shared_xlsx_after(chars, cells, wrapped, mixed, 0)
}

/// The same, after `cheap` empty comments (one event each, nearly free),
/// which set where in the sheet the sink's checks fall.
fn blank_shared_xlsx_after(chars: usize, cells: usize, wrapped: bool, mixed: bool, cheap: usize) -> Vec<u8> {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    // Padding keeps each part under the 100:1 bomb ratio.
    // Runs of a million characters (3 MB each), under the 4 MB event cap;
    // the shared-string list joins them.
    let mut si = String::new();
    let mut left = chars;
    while left > 0 {
        let n = left.min(1_000_000);
        if mixed {
            // Blank characters of one, two and three bytes, at random.
            const BLANKS: [char; 16] = [' ', '\u{85}', '\u{A0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2009}', '\u{2028}', '\u{202F}', '\u{205F}', '\u{3000}'];
            si.push_str("<r><t>");
            for _ in 0..n {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                si.push(BLANKS[(seed % 16) as usize]);
            }
            si.push_str("</t></r>");
        } else {
            write!(si, "<r><t>{}</t></r>", "\u{3000}".repeat(n)).unwrap();
        }
        left -= n;
    }
    si.push_str(&random_comment(chars * 3 / 60 + 1000, &mut seed));
    let mut data = random_comment(cells * 22 / 60 + 1000, &mut seed);
    data.push_str(&"<!---->".repeat(cheap));
    let per_row = 1000;
    let mut left = cells;
    while left > 0 {
        let n = left.min(per_row);
        // Wrapped: one v around the row's cells, so a cell is three events
        // (open, its index, close) rather than five.
        if wrapped {
            data.push_str("<row><v>");
            for _ in 0..n {
                data.push_str(r#"<c t="s">0</c>"#);
            }
            data.push_str("</v></row>");
        } else {
            data.push_str("<row>");
            for _ in 0..n {
                data.push_str(r#"<c t="s"><v>0</v></c>"#);
            }
            data.push_str("</row>");
        }
        left -= n;
    }
    sample::xlsx(&[("S", &data)], &[&si], "", false)
}

/// Each cell naming a blank shared string trims the whole string (up to
/// 4 million characters) and writes nothing, so the sink is asked only
/// every 4,096 events. Measurement, for a release build:
/// `cargo test --release --no-default-features --lib r5_blank_interval -- --ignored --nocapture`.
#[test]
fn r5_blank_interval_release() {
    let chars = Caps::default().shared_chars - 1;
    for (cells, wrapped, mixed) in [(819, false, false), (1365, true, false), (1365, true, true)] {
        let bytes = blank_shared_xlsx(chars, cells, wrapped, mixed);
        let start = Instant::now();
        let r = read_at(&Excel::default(), "b.xlsx", &bytes, Instant::now() + Duration::from_secs(600));
        eprintln!("blank interval: zip={} cells={cells} wrapped={wrapped} mixed={mixed} elapsed={:?} result={:?}", bytes.len(), start.elapsed(), r.as_ref().map(|o| o.0.len()));
    }
}

/// FINDING 2. Past MAX_PARTS, every new part trims the last kept part's text again:
/// a last page of ~940,000 blank characters then "x", followed by page
/// breaks, costs a full trim per break.
#[test]
fn r5_parts_full_retrims_the_last_part_per_break() {
    let base = parts_full_read(0);
    let with = parts_full_read(2048);
    eprintln!("parts full: 0 breaks {base:?}, 2048 breaks {with:?}");
    assert!(with.saturating_sub(base) < Duration::from_secs(1), "2,048 page breaks took {:?} more", with.saturating_sub(base));
}

fn parts_full_read(breaks: usize) -> Duration {
    let mut body = String::new();
    let brk = r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#;
    for _ in 0..MAX_PARTS - 1 {
        body.push_str(r#"<w:p><w:r><w:t>x</w:t></w:r></w:p>"#);
        body.push_str(brk);
    }
    body.push_str("<w:p><w:r>");
    // Blank characters of mixed widths, 30,000 to a run (over 64 KB, so
    // each run is written to the sink as it comes).
    const BLANKS: [char; 16] = [' ', '\u{85}', '\u{A0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2009}', '\u{2028}', '\u{202F}', '\u{205F}', '\u{3000}'];
    let mut seed = 99u64;
    for _ in 0..31 {
        body.push_str("<w:t>");
        for _ in 0..30_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            body.push(BLANKS[(seed % 16) as usize]);
        }
        body.push_str("</w:t>");
    }
    body.push_str("<w:t>x</w:t></w:r></w:p>");
    for _ in 0..breaks {
        body.push_str(r#"<w:br w:type="page"/>"#);
    }
    body.push_str(&random_comment(200_000, &mut seed));
    let bytes = sample::docx(&body);
    let start = Instant::now();
    let r = read_at(&Word::default(), "p.docx", &bytes, Instant::now() + Duration::from_secs(600));
    let elapsed = start.elapsed();
    eprintln!("parts full: zip={} breaks={breaks} elapsed={elapsed:?} result={:?}", bytes.len(), r.as_ref().map(|o| o.0.len()).map_err(|e| format!("{e:?}")));
    elapsed
}

/// FINDING 3. A zip64 end record may declare as many entries as fit in the file at
/// 46 bytes each; the zip crate reserves room for all of them before
/// reading one.
fn zip64_claiming(len: u64) -> Vec<u8> {
    let n = len / 47;
    let mut f = vec![0u8; len as usize];
    f[..4].copy_from_slice(b"PK\x03\x04");
    let eocd64_at = len;
    // Zip64 end of central directory record.
    f.extend_from_slice(&0x0606_4b50u32.to_le_bytes());
    f.extend_from_slice(&44u64.to_le_bytes());
    f.extend_from_slice(&45u16.to_le_bytes());
    f.extend_from_slice(&45u16.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&n.to_le_bytes());
    f.extend_from_slice(&n.to_le_bytes());
    f.extend_from_slice(&(n * 46).to_le_bytes());
    f.extend_from_slice(&n.to_le_bytes()); // central directory offset (>= n)
    // Locator.
    f.extend_from_slice(&0x0706_4b50u32.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&eocd64_at.to_le_bytes());
    f.extend_from_slice(&1u32.to_le_bytes());
    // End of central directory, all fields deferring to zip64.
    f.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    f.extend_from_slice(&0u16.to_le_bytes());
    f.extend_from_slice(&0u16.to_le_bytes());
    f.extend_from_slice(&0xFFFFu16.to_le_bytes());
    f.extend_from_slice(&0xFFFFu16.to_le_bytes());
    f.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    f.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    f.extend_from_slice(&0u16.to_le_bytes());
    f
}

#[test]
fn r5_zip64_declared_entries_reserve_memory_before_reading() {
    for len in [4 * MB, 99 * MB] {
        let bytes = zip64_claiming(len);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("z.docx");
        std::fs::write(&path, &bytes).unwrap();
        drop(bytes);
        let ((r, ms), peak) = sample::held::peak(|| {
            let start = Instant::now();
            let mut sink = Sink::new(1_000_000, Instant::now() + Duration::from_secs(10));
            (Word::default().read(&path, &mut sink).map_err(|e| format!("{e:?}")), start.elapsed())
        });
        eprintln!("zip64 claim: file={} MB peak={} MiB elapsed={ms:?} result={r:?}", len / MB, peak / MB as usize);
        assert!(peak < 256 * MB as usize, "a {} MB file reserved {} MiB", len / MB, peak / MB as usize);
    }
}

/// Checked, no finding. 10,000 sheets with 4,000-byte names: each name is held in the workbook
/// list, joined once for the sheets fact, and again as a part's label.
#[test]
fn r5_sheet_names_memory() {
    let mut seed = 0x1234_5678u64;
    let n = 10_000;
    let mut workbook = format!(r#"<workbook {}><sheets>"#, sample::S);
    let mut rels = Vec::new();
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    let sheet = format!(r#"<worksheet {}><sheetData><row><c t="inlineStr"><is><t>v</t></is></c></row></sheetData></worksheet>"#, sample::S);
    for i in 0..n {
        let name = format!("{}{i:05}", "a".repeat(3995));
        // Case variants of one part: distinct targets, one zip entry.
        let file: String = "abcdefghijklmn".chars().enumerate().map(|(b, c)| if i >> b & 1 == 1 { c.to_ascii_uppercase() } else { c }).collect();
        write!(workbook, r#"<sheet name="{name}" sheetId="{}" r:id="r{i}"/>"#, i + 1).unwrap();
        rels.push((format!("r{i}"), format!("ws/{file}.xml")));
    }
    workbook.push_str(&random_comment(700_000, &mut seed));
    workbook.push_str("</sheets></workbook>");
    parts.push(("xl/ws/abcdefghijklmn.xml".into(), sheet.into_bytes()));
    let rels: Vec<(&str, &str, &str)> = rels.iter().map(|(a, b)| (a.as_str(), "worksheet", b.as_str())).collect();
    let root = sample::rels(&[("rId1", "officeDocument", "xl/workbook.xml")]);
    parts.push(("_rels/.rels".into(), root.into_bytes()));
    parts.push(("xl/workbook.xml".into(), workbook.into_bytes()));
    parts.push(("xl/_rels/workbook.xml.rels".into(), sample::rels(&rels).into_bytes()));
    let parts: Vec<(&str, &[u8])> = parts.iter().map(|(a, b)| (a.as_str(), b.as_slice())).collect();
    let bytes = sample::zip(&parts);
    let start = Instant::now();
    let (r, peak) = sample::held::peak(|| read_at(&Excel::default(), "n.xlsx", &bytes, Instant::now() + Duration::from_secs(60)));
    eprintln!("sheet names: zip={} peak={} MiB elapsed={:?} result={:?}", bytes.len(), peak / MB as usize, start.elapsed(), r.as_ref().map(|o| o.0.len()).map_err(|e| format!("{e:?}")));
    assert!(peak < 128 * MB as usize);
}

/// FINDING 1. The worker's 10 s deadline, with the checks' phase set by
/// cheap events so that one falls just before the deadline: the next comes
/// a whole interval (1,365 cells, each trimming 4 million blank characters)
/// later. Release: 23.8 s from a 3.5 MB file. Debug: minutes.
#[test]
fn r5_blank_shared_string_cells_ask_the_sink_in_time() {
    let chars = Caps::default().shared_chars - 1;
    let cheap: usize = std::env::var("R5_CHEAP").ok().and_then(|v| v.parse().ok()).unwrap_or(1450);
    let bytes = blank_shared_xlsx_after(chars, 1_000_000, true, true, cheap);
    let start = Instant::now();
    let r = read_at(&Excel::default(), "b.xlsx", &bytes, Instant::now() + Duration::from_secs(10));
    let elapsed = start.elapsed();
    eprintln!("blank phase: zip={} cheap={cheap} deadline=10s elapsed={elapsed:?} result={:?}", bytes.len(), r.as_ref().map(|o| o.0.len()).map_err(|e| format!("{e:?}")));
    assert!(elapsed < Duration::from_secs(15), "ran {elapsed:?} against a 10 s deadline");
}
