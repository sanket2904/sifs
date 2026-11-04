//! The step 2 reviewer's second-round tests, kept: each pins a finding of
//! handover/step2-review-r2/REPORT.txt, or a cap's edge it checked.
use super::*;
use super::sample::{self, paras, shape, W};
use crate::extract::{Part, Reader};
use std::time::{Duration, Instant};

type Output = (Vec<Part>, Vec<(String, String)>);
fn read(reader: &dyn Reader, bytes: &[u8]) -> Result<Output, ReadError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("review.office");
    std::fs::write(&path, bytes).unwrap();
    let mut sink = Sink::new(1_000_000, Instant::now() + Duration::from_secs(10));
    reader.read(&path, &mut sink)?;
    Ok(sink.finish())
}
fn truncated(out: &Output) -> bool { out.1.iter().any(|(k,v)| k == "truncated" && v == "1") }
fn text(out: &Output) -> String { out.0.iter().map(|p| p.text.as_str()).collect() }
fn letters(n: usize) -> String {
    let mut seed = 99u64;
    (0..n).map(|_| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (b'a' + ((seed >> 33) % 26) as u8) as char
    }).collect()
}

#[test]
fn r2_relationship_path_memory() {
    // Padding makes the honest compression ratio <100; no altered headers/caps.
    let target = "a/".repeat(10 * MB as usize);
    let rels = format!("<Relationships><!--{}--><Relationship Id=\"x\" Type=\"t/header\" Target=\"{target}\"/></Relationships>", letters(500_000));
    let document = format!("<w:document {W}><w:body/></w:document>");
    let bytes = sample::zip(&[("word/document.xml", document.as_bytes()), ("word/_rels/document.xml.rels", rels.as_bytes())]);
    let start = Instant::now();
    let (got, held) = sample::held::peak(|| read(&Word::default(), &bytes));
    eprintln!("relationship path: file={} peak={} MiB elapsed={:?} result={:?}", bytes.len(), held / MB as usize, start.elapsed(), got.as_ref().map(|x| x.0.len()));
    assert!(held < 256 * MB as usize, "relationship target exceeded 256 MiB");
}

#[test]
fn r2_single_xml_text_memory() {
    let mut document = format!("<w:document {W}><w:body><w:p><w:r><w:t>");
    for _ in 0..80 {
        document.push_str(&letters(20_000));
        document.push_str(&"x".repeat(MB as usize - 20_001));
        document.push('\r'); // XML newline normalization allocates a second whole node.
    }
    document.push_str("</w:t></w:r></w:p></w:body></w:document>");
    let bytes = sample::zip(&[("word/document.xml", document.as_bytes())]);
    let start = Instant::now();
    let (got, held) = sample::held::peak(|| read(&Word::default(), &bytes));
    eprintln!("single XML text: file={} peak={} MiB elapsed={:?} result={:?}", bytes.len(), held / MB as usize, start.elapsed(), got.as_ref().map(|x| text(x).len()));
    assert!(held < 256 * MB as usize, "single text event exceeded 256 MiB");
}

#[test]
fn r2_events_span_small_parts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tiny.zip");
    std::fs::write(&path, sample::zip(&[("x.xml", b"<x/>")])).unwrap();
    let mut package = Package::open(&path, Kind::Word, Caps::default()).unwrap();
    let mut sink = Sink::new(1_000_000, Instant::now());
    for n in 1..=4096 {
        let got = package.parse("x.xml", &mut Props::new(&[]), &mut sink);
        if matches!(got, Err(ReadError::Timeout)) {
            eprintln!("cumulative events: timeout after {n} tiny parts, events={}", package.events.get());
            // At the latest once CHECK_EVERY events have gone by (each part's
            // compressed bytes now ask the time too).
            assert!(package.events.get() <= CHECK_EVERY);
            return;
        }
        assert!(got.is_ok());
    }
    panic!("small parts reset the XML event clock");
}

#[test]
fn r2_blank_pages_memory() {
    let body = format!("<!--{}--><w:p><w:r>{}</w:r></w:p>", letters(1_200_000), "<w:br w:type=\"page\"/>".repeat(3_500_000));
    let bytes = sample::docx(&body);
    let start = Instant::now();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blank.docx");
    std::fs::write(&path, &bytes).unwrap();
    let mut sink = Sink::new(1_000_000, Instant::now() + Duration::from_secs(10));
    let (got, held) = sample::held::peak(|| Word::default().read(&path, &mut sink));
    // Exact live capacity, independent of the peak allocator's realloc accounting.
    let live = sink.parts.capacity() * std::mem::size_of::<Part>() + sink.parts.iter().map(|p| p.label.capacity() + p.text.capacity()).sum::<usize>();
    eprintln!("blank pages: file={} peak={} MiB retained={} MiB parts={} elapsed={:?} result={got:?}", bytes.len(), held / MB as usize, live / MB as usize, sink.parts.len(), start.elapsed());
    assert!(live < 256 * MB as usize, "empty page parts exceeded 256 MiB");
}

#[test]
fn r2_styles_repeated_format_obeys_deadline() {
    // One format referenced by 1,500 styles is rescanned 1,500 times, outside walk.
    let styles = format!("<numFmts><numFmt numFmtId=\"164\" formatCode=\"{}\"/></numFmts><cellXfs>{}</cellXfs>", letters(1_000_000), "<xf numFmtId=\"164\"/>".repeat(1500));
    let bytes = sample::xlsx(&[("S", "")], &[], &styles, false);
    let got = read(&Excel::default(), &bytes);
    // Each format is classified once, as it is read (and one this long
    // reads as absent): well within the reader's time.
    assert!(got.is_ok(), "{:?}", got.map(|_| ()));
}

#[test]
fn r2_two_pending_typed_breaks_keep_blank_page() {
    let body = concat!("<w:tbl><w:tr><w:tc><w:p><w:r><w:t>one</w:t>",
        "<w:br w:type=\"page\"/><w:br w:type=\"page\"/>",
        "</w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>two</w:t></w:r></w:p></w:tc></w:tr></w:tbl>");
    let out = read(&Word::default(), &sample::docx(body)).unwrap();
    eprintln!("pending breaks: {:?}", out.0);
    assert_eq!(out.0[1].label, "page 3");
}

#[test]
fn r2_shared_unicode_rich_runs_at_character_cap() {
    for value in ["<t>ééé</t>", "<r><t>éé</t></r><r><t>é</t></r>"] {
        let bytes = sample::xlsx(&[("S", "<row><c t=\"s\"><v>0</v></c></row>")], &[value], "", false);
        let out = read(&Excel { caps: Caps { shared_chars: 3, ..Caps::default() } }, &bytes).unwrap();
        eprintln!("shared rich runs: text={:?} truncated={}", text(&out), truncated(&out));
        assert_eq!(text(&out), "ééé\n");
        assert!(!truncated(&out));
    }
}

#[test]
fn r2_cell_cap_records_loss() {
    for n in [32_767, 32_768] {
        let sheet = format!("<row><c t=\"inlineStr\"><is><t>{}</t></is></c></row>", "a".repeat(n));
        let out = read(&Excel::default(), &sample::xlsx(&[("S", &sheet)], &[], "", false)).unwrap();
        assert_eq!(text(&out).trim_end().chars().count(), n.min(super::excel::MAX_CELL_CHARS));
        assert_eq!(truncated(&out), n > super::excel::MAX_CELL_CHARS, "a cut cell must record truncated");
    }
}

#[test]
fn r2_caps_exact_and_one_past() {
    for count in [2,3] {
        let row = format!("<row>{}</row>", "<c><v>1</v></c>".repeat(count));
        let out = read(&Excel { caps: Caps { cells: 2, ..Caps::default() } }, &sample::xlsx(&[("S", &row)], &[], "", false)).unwrap();
        assert_eq!(text(&out), if count == 2 { "1\t1\n" } else { "1\t1" });
        assert_eq!(truncated(&out), count > 2);
        let shared: Vec<&str> = vec!["<t>x</t>"; count];
        let refs = "<row><c t=\"s\"><v>0</v></c><c t=\"s\"><v>1</v></c><c t=\"s\"><v>2</v></c></row>";
        for caps in [Caps {shared_strings: 2, ..Caps::default()}, Caps {shared_chars: 2, ..Caps::default()}] {
            let out = read(&Excel {caps}, &sample::xlsx(&[("S", refs)], &shared, "", false)).unwrap();
            assert_eq!(text(&out), "x\tx\n");
            assert_eq!(truncated(&out), count > 2);
        }
        let sheets = vec![("S", "<row><c><v>1</v></c></row>"); count];
        let out = read(&Excel { caps: Caps {listed: 2, ..Caps::default()} }, &sample::xlsx(&sheets, &[], "", false)).unwrap();
        assert_eq!(out.0.len(), 2);
        assert_eq!(truncated(&out), count > 2);
        let s = shape(&["x"]);
        let out = read(&PowerPoint { caps: Caps {listed: 2, ..Caps::default()} }, &sample::pptx(&vec![(s.as_str(), None); count])).unwrap();
        assert_eq!(out.0.len(), 2);
        assert_eq!(truncated(&out), count > 2);
    }
    for count in [2,3] {
        let mut sink = Sink::new(2, Instant::now() + Duration::from_secs(10));
        let flow = sink.text(&"é".repeat(count)).unwrap();
        let out = sink.finish();
        assert_eq!(text(&out), "éé");
        assert_eq!(flow == Flow::Full, count > 2);
        assert_eq!(truncated(&out), count > 2);
    }
    eprintln!("caps: exact/one-past cells, shared count/chars, sheets/slides and sink passed");
}

#[test]
fn r2_zip_and_property_cap_edges() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("caps.zip");
    let bytes = sample::zip(&[("a", b"<x/>"), ("b", b"<y/>")]);
    std::fs::write(&path, &bytes).unwrap();
    assert!(Package::open(&path, Kind::Word, Caps {entries: 2, ..Caps::default()}).is_ok());
    assert!(matches!(Package::open(&path, Kind::Word, Caps {entries: 1, ..Caps::default()}), Err(ReadError::Unsupported(_))));
    // Find this zip's precise open-read budget (the budget includes seeks/rereads).
    let mut lo = 0;
    let mut hi = Caps::default().directory_bytes;
    while lo < hi {
        let mid = (lo + hi) / 2;
        if Package::open(&path, Kind::Word, Caps {directory_bytes: mid, ..Caps::default()}).is_ok() { hi = mid; } else { lo = mid + 1; }
    }
    assert!(Package::open(&path, Kind::Word, Caps {directory_bytes: lo, ..Caps::default()}).is_ok());
    assert!(matches!(Package::open(&path, Kind::Word, Caps {directory_bytes: lo - 1, ..Caps::default()}), Err(ReadError::Other(_))));
    for (caps, why) in [
        (Caps {inflated: 4, ..Caps::default()}, Stop::Inflated),
        (Caps {ratio_after: 4, ratio: 2, ..Caps::default()}, Stop::Bomb),
    ] {
        for n in [4,5] {
            let total = Cell::new(0);
            let stop = Cell::new(None);
            let data = vec![b'x'; n];
            let mut input = Inflate {file: std::io::Cursor::new(data), compressed: 2, read: 0, total: &total, stop: &stop, caps: &caps};
            let mut b = [0];
            for _ in 0..4 { assert_eq!(std::io::Read::read(&mut input, &mut b).unwrap(), 1); }
            let extra = std::io::Read::read(&mut input, &mut b);
            assert_eq!(extra.is_err(), n > 4);
            assert_eq!(stop.get(), if n > 4 {Some(why)} else {None});
        }
    }
    // Integration: crossing the inflated cap retains prior text and marks it cut.
    let body = format!("{}<!--{}-->{}", paras(&["kept"]), letters(9000), paras(&["later"]));
    let doc = format!("<w:document {W}><w:body>{body}</w:body></w:document>");
    let z = sample::zip(&[("word/document.xml", doc.as_bytes())]);
    let full = read(&Word {caps: Caps {inflated: doc.len() as u64, ..Caps::default()}}, &z).unwrap();
    assert_eq!(text(&full), "kept\nlater\n");
    assert!(!truncated(&full));
    let cut = read(&Word {caps: Caps {inflated: doc.len() as u64 - 1, ..Caps::default()}}, &z).unwrap();
    assert_eq!(text(&cut), "kept\n");
    assert!(truncated(&cut));
    for n in [1000,1001] {
        let mut sink = Sink::new(1_000_000, Instant::now() + Duration::from_secs(10));
        sink.fact("title", "é".repeat(n));
        let out = sink.finish();
        assert_eq!(out.1[0].1.chars().count(), 1000);
        assert!(!truncated(&out)); // Fact shortening is deliberately independent of text truncation.
    }
    eprintln!("zip/property edges: entries, directory budget ({lo} bytes), inflate, ratio/grace, fact passed");
}

#[test]
fn r2_streamed_table_cell_preserves_text_and_tabs() {
    let value = "abc ".repeat(20_000);
    let runs: String = value.as_bytes().chunks(1000).map(|c| format!("<w:r><w:t>{}</w:t></w:r>", std::str::from_utf8(c).unwrap())).collect();
    let body = format!("<w:tbl><w:tr><w:tc><w:p>{runs}</w:p></w:tc><w:tc>{}</w:tc></w:tr></w:tbl>", paras(&["end"]));
    let out = read(&Word::default(), &sample::docx(&body)).unwrap();
    assert_eq!(text(&out), format!("{}\tend\n", value.trim_end()));
}
