//! The step 2 reviewer's tests, kept: each pins a finding of the review
//! (handover/step2-review/REPORT.txt), or something it checked. Time
//! bounds have room for a loaded machine: a reader past its limit stops
//! within a moment of it, where the defects found took 30 to 40 s.

use super::sample::{self, paras, shape, P, S, W};
use super::*;
use crate::extract::{Part, Reader};
use std::time::{Duration, Instant};

type Read = Result<(Vec<Part>, Vec<(String, String)>), ReadError>;

fn read_at(reader: &dyn Reader, name: &str, body: &[u8], deadline: Instant) -> Read {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, body).unwrap();
    let mut sink = Sink::new(1_000_000, deadline);
    reader.read(&path, &mut sink)?;
    Ok(sink.finish())
}

fn read(reader: &dyn Reader, name: &str, body: &[u8]) -> Read {
    read_at(reader, name, body, Instant::now() + Duration::from_secs(10))
}

fn text(read: Read) -> String {
    read.unwrap().0.into_iter().map(|p| format!("[{}]{}", p.label, p.text)).collect()
}

fn stored(parts: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, body) in parts {
        zip.start_file(*name, options).unwrap();
        zip.write_all(body).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn rng(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *seed >> 33
}

fn random_letters(n: usize, seed: &mut u64) -> String {
    (0..n).map(|_| (b'a' + (rng(seed) % 26) as u8) as char).collect()
}

/// MB this thread held at most while f ran (the reader runs on it).
fn held_mb<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let (out, bytes) = sample::held::peak(f);
    (out, bytes / MB as usize)
}

// ---------- Time: loops outside walk() never ask the sink ----------

fn pptx_lists(n: usize, rels_n: usize) -> Vec<u8> {
    let mut pres = format!(r#"<?xml version="1.0"?><p:presentation {P}><p:sldIdLst>"#);
    let mut rels = String::from(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for i in 0..n {
        pres.push_str(&format!(r#"<p:sldId r:id="x{i}"/>"#));
    }
    for i in 0..rels_n {
        rels.push_str(&format!(r#"<Relationship Id="y{i}" Type="t/slide" Target="s.xml"/>"#));
    }
    pres.push_str("</p:sldIdLst></p:presentation>");
    rels.push_str("</Relationships>");
    let root = sample::rels(&[("rId1", "officeDocument", "ppt/presentation.xml")]);
    sample::zip(&[
        ("_rels/.rels", root.as_bytes()),
        ("ppt/presentation.xml", pres.as_bytes()),
        ("ppt/_rels/presentation.xml.rels", rels.as_bytes()),
    ])
}

#[test]
fn review_pptx_slide_list_against_missing_ids_is_quadratic_and_unchecked() {
    let n = 60_000;
    for (label, bytes) in [("linear (1 rel)", pptx_lists(n, 1)), ("quadratic", pptx_lists(n, n))] {
        let start = Instant::now();
        let got = read_at(&PowerPoint::default(), "q.pptx", &bytes, start + Duration::from_secs(10));
        eprintln!("pptx {label}: {} bytes, {:?}, result {:?}", bytes.len(), start.elapsed(), got.map(|r| r.0.len()));
    }
    let bytes = pptx_lists(n, n);
    let start = Instant::now();
    let got = read_at(&PowerPoint::default(), "q.pptx", &bytes, start + Duration::from_secs(10));
    let took = start.elapsed();
    assert!(took < Duration::from_secs(20), "time limit 10 s, took {took:?}; result {:?}", got.map(|_| ()));
}

#[test]
fn review_xlsx_sheet_list_against_missing_ids_is_quadratic_and_unchecked() {
    let n = 60_000;
    let mut wb = format!(r#"<?xml version="1.0"?><workbook {S}><sheets>"#);
    let mut rels = String::from(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for i in 0..n {
        wb.push_str(&format!(r#"<sheet name="s{i}" r:id="x{i}"/>"#));
        rels.push_str(&format!(r#"<Relationship Id="y{i}" Type="t/worksheet" Target="s.xml"/>"#));
    }
    wb.push_str("</sheets></workbook>");
    rels.push_str("</Relationships>");
    let root = sample::rels(&[("rId1", "officeDocument", "xl/workbook.xml")]);
    let bytes = sample::zip(&[
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", wb.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
    ]);
    let start = Instant::now();
    let got = read_at(&Excel::default(), "q.xlsx", &bytes, start + Duration::from_secs(10));
    let took = start.elapsed();
    eprintln!("xlsx {n} x {n}: {} bytes, {took:?}, {:?}", bytes.len(), got.as_ref().map(|_| ()));
    assert!(took < Duration::from_secs(20), "time limit 10 s, took {took:?}");
}

#[test]
fn review_many_sheet_entries_to_one_empty_sheet_never_ask_the_sink() {
    // Every <sheet> points at the same tiny sheet: each parse is under
    // 4,096 events and writes no text, so nothing checks the time.
    let n = 400_000;
    let mut wb = format!(r#"<?xml version="1.0"?><workbook {S}><sheets>"#);
    for i in 0..n {
        wb.push_str(&format!(r#"<sheet name="s{i}" r:id="rId9"/>"#));
    }
    wb.push_str("</sheets></workbook>");
    let rels = sample::rels(&[("rId9", "worksheet", "worksheets/sheet1.xml")]);
    let root = sample::rels(&[("rId1", "officeDocument", "xl/workbook.xml")]);
    let sheet = format!(r#"<?xml version="1.0"?><worksheet {S}><sheetData/></worksheet>"#);
    let bytes = sample::zip(&[
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", wb.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
    ]);
    let start = Instant::now();
    let got = read_at(&Excel::default(), "many.xlsx", &bytes, start + Duration::from_secs(10));
    let took = start.elapsed();
    eprintln!("{n} sheet parses: {took:?}, result {:?}", got.as_ref().map(|_| ()));
    assert!(took < Duration::from_secs(20), "time limit 10 s, took {took:?}");
}

// ---------- Memory ----------

#[test]
fn review_excel_row_of_one_shared_string_repeated_grows_memory_past_caps() {
    // One shared string of 1M characters (well under the 4M cap), and a
    // single row that refers to it 300 times: each cell clones it into the
    // row before the row is written. 300 MB held for a 1 MB file.
    let mut seed = 1;
    let big = random_letters(1_000_000, &mut seed);
    let row: String = (0..300).map(|i| format!(r#"<c r="A{i}" t="s"><v>0</v></c>"#)).collect();
    let bytes = sample::xlsx(&[("S", &format!("<row>{row}</row>"))], &[&format!("<t>{big}</t>")], "", false);
    eprintln!("xlsx file {} bytes", bytes.len());
    let (got, grew_mb) = held_mb(|| read(&Excel::default(), "rep.xlsx", &bytes));
    eprintln!("peak memory grew by {grew_mb} MB; ok={}", got.is_ok());
    assert!(grew_mb < 64, "peak memory grew by {grew_mb} MB for a {} byte file", bytes.len());
}

#[test]
fn review_word_table_cell_is_held_no_more_than_a_paragraph() {
    // 40 MB of words in one table cell. Prose used to keep the whole cell,
    // then split it into a Vec<&str> and join it again: about 4x the
    // paragraph's memory. (One 40 MB text node is itself held by the XML
    // reader, whatever Prose does: that is the baseline.)
    let mut seed = 3;
    let mut s = String::new();
    while s.len() < 40 * MB as usize {
        s.push_str(&random_letters(7, &mut seed));
        s.push(' ');
    }
    let cell = sample::docx(&format!("<w:tbl><w:tr><w:tc><w:p><w:r><w:t>{s}</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"));
    let (got, cell_mb) = held_mb(|| read(&Word::default(), "cell.docx", &cell));
    assert!(got.is_ok());
    let para = sample::docx(&paras(&[&s]));
    let (_, para_mb) = held_mb(|| read(&Word::default(), "para.docx", &para));
    eprintln!("table cell: {cell_mb} MB, paragraph: {para_mb} MB, for 40 MB of text");
    assert!(cell_mb <= para_mb + 8, "a cell held {cell_mb} MB, a paragraph {para_mb} MB");
}

// ---------- Zip bombs with lying headers ----------

fn patch_sizes(bytes: &mut [u8], name: &[u8], compressed: u32, uncompressed: u32) {
    for i in 0..bytes.len() - 46 {
        if bytes[i..i + 4] == *b"PK\x01\x02" && bytes[i + 46..].starts_with(name) {
            bytes[i + 20..i + 24].copy_from_slice(&compressed.to_le_bytes());
            bytes[i + 24..i + 28].copy_from_slice(&uncompressed.to_le_bytes());
        }
        if bytes[i..i + 4] == *b"PK\x03\x04" && bytes[i + 30..].starts_with(name) {
            bytes[i + 18..i + 22].copy_from_slice(&compressed.to_le_bytes());
            bytes[i + 22..i + 26].copy_from_slice(&uncompressed.to_le_bytes());
        }
    }
}

#[test]
fn review_bomb_with_faked_sizes() {
    // 120 MB of empty paragraphs: no text, many events.
    let body = "<w:p/>".repeat(20 * MB as usize);
    let clean = sample::docx(&body);
    eprintln!("bomb file {} bytes", clean.len());
    let start = Instant::now();
    eprintln!("honest headers: {:?} in {:?}", read(&Word::default(), "b.docx", &clean).map(|_| ()), start.elapsed());
    for (c, u) in [(u32::MAX - 1, 10), (clean.len() as u32 * 2, 10), (0x00FF_FFFF, 0x10)] {
        let mut bytes = clean.clone();
        patch_sizes(&mut bytes, b"word/document.xml", c, u);
        let start = Instant::now();
        let got = read(&Word::default(), "b.docx", &bytes);
        eprintln!("compressed {c} uncompressed {u}: {:?} in {:?}", got.as_ref().map(|r| r.1.clone()), start.elapsed());
        assert!(start.elapsed() < Duration::from_secs(20));
    }
}

// ---------- Fuzz: truncations and flipped bytes never panic ----------

fn no_panic(reader: &dyn Reader, name: &str, bytes: &[u8]) -> Option<String> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    let mut sink = Sink::new(1_000_000, Instant::now() + Duration::from_secs(10));
    let start = Instant::now();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reader.read(&path, &mut sink)));
    assert!(start.elapsed() < Duration::from_secs(10), "slow: {:?}", start.elapsed());
    match r {
        Ok(_) => None,
        Err(p) => Some(
            p.downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_default(),
        ),
    }
}

#[test]
fn review_fuzz_truncations_and_flips() {
    let docx = sample::docx(&format!(
        "{}<w:tbl><w:tr><w:tc><w:p><w:r><w:t>c</w:t></w:r></w:p></w:tc></w:tr></w:tbl>",
        paras(&["one", "two"])
    ));
    let xlsx = sample::xlsx(&[("A", r#"<row><c t="s" s="1"><v>0</v></c><c s="1"><v>45000.5</v></c></row>"#)], &["<t>x</t>"], r#"<cellXfs><xf numFmtId="0"/><xf numFmtId="22"/></cellXfs>"#, false);
    let pptx = sample::pptx(&[(&shape(&["a"]), Some("n"))]);
    let cases: [(&dyn Reader, &str, &Vec<u8>); 3] =
        [(&Word::default(), "f.docx", &docx), (&Excel::default(), "f.xlsx", &xlsx), (&PowerPoint::default(), "f.pptx", &pptx)];
    let mut panics = Vec::new();
    for (reader, name, bytes) in cases {
        for cut in 0..bytes.len() {
            if let Some(p) = no_panic(reader, name, &bytes[..cut]) {
                panics.push(format!("{name} cut {cut}: {p}"));
            }
        }
        let mut seed = 99u64;
        for _ in 0..3000 {
            let mut b = bytes.clone();
            for _ in 0..(1 + rng(&mut seed) % 4) {
                let at = (rng(&mut seed) as usize) % b.len();
                b[at] ^= 1 << (rng(&mut seed) % 8);
            }
            if let Some(p) = no_panic(reader, name, &b) {
                panics.push(format!("{name} flip: {p}"));
            }
        }
    }
    assert!(panics.is_empty(), "{} panics, first: {:?}", panics.len(), &panics[..panics.len().min(5)]);
}

#[test]
fn review_fuzz_compound_files() {
    let base = sample::compound(&["EncryptionInfo", "EncryptedPackage"]);
    let mut seed = 5u64;
    let mut panics = 0;
    for _ in 0..3000 {
        let mut b = base.clone();
        for _ in 0..(1 + rng(&mut seed) % 8) {
            let at = (rng(&mut seed) as usize) % b.len();
            b[at] = rng(&mut seed) as u8;
        }
        if no_panic(&Word::default(), "c.docx", &b).is_some() {
            panics += 1;
        }
    }
    assert_eq!(panics, 0);
}

// ---------- XML ----------

#[test]
fn review_doctype_external_entity_and_deep_and_wide_xml() {
    let doc = format!(
        r#"<?xml version="1.0"?><!DOCTYPE w:document [<!ENTITY xxe SYSTEM "file:///etc/passwd"><!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">]><w:document {W}><w:body><w:p><w:r><w:t>x&xxe;y&b;z&#x41;&amp;</w:t></w:r></w:p></w:body></w:document>"#
    );
    let rels = sample::rels(&[("rId1", "officeDocument", "word/document.xml")]);
    let bytes = sample::zip(&[("_rels/.rels", rels.as_bytes()), ("word/document.xml", doc.as_bytes())]);
    assert_eq!(text(read(&Word::default(), "x.docx", &bytes)), "[]xyzA&\n");
    // Deep: 2M nested elements.
    let n = 2_000_000;
    let body = format!("{}<w:p><w:r><w:t>deep</w:t></w:r></w:p>{}", "<w:sdt>".repeat(n), "</w:sdt>".repeat(n));
    let bytes = sample::docx(&body);
    let start = Instant::now();
    let got = read(&Word::default(), "deep.docx", &bytes);
    eprintln!("deep: {:?} in {:?}", got.as_ref().map(|r| r.0.len()), start.elapsed());
    // Huge attribute: 60 MB of random letters in one attribute.
    let mut seed = 11;
    let attr = random_letters(60 * MB as usize, &mut seed);
    let body = format!(r#"<w:p w:x="{attr}"><w:r><w:t>after</w:t></w:r></w:p>"#);
    let bytes = sample::docx(&body);
    let start = Instant::now();
    let (got, grew_mb) = held_mb(|| read(&Word::default(), "attr.docx", &bytes));
    eprintln!(
        "60 MB attribute: {:?} in {:?}, peak grew {grew_mb} MB",
        got.map(|r| r.0.into_iter().map(|p| p.text).collect::<String>()),
        start.elapsed(),
    );
}

// ---------- Word correctness ----------

#[test]
fn review_word_visible_text_kept() {
    let body = concat!(
        r#"<w:p><w:ins w:id="1"><w:r><w:t>Inserted</w:t></w:r></w:ins></w:p>"#,
        r#"<w:p><w:hyperlink r:id="rId9"><w:r><w:t>Linked</w:t></w:r></w:hyperlink></w:p>"#,
        r#"<w:p><w:smartTag w:uri="u" w:element="place"><w:r><w:t>Tagged</w:t></w:r></w:smartTag></w:p>"#,
        r#"<w:sdt><w:sdtPr><w:alias w:val="A"/></w:sdtPr><w:sdtContent><w:p><w:r><w:t>Control</w:t></w:r></w:p></w:sdtContent></w:sdt>"#,
        r#"<w:p><w:fldSimple w:instr=" DATE "><w:r><w:t>Simple</w:t></w:r></w:fldSimple></w:p>"#,
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>PAGE</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Result</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        r#"<w:p><w:moveTo w:id="2"><w:r><w:t>MovedHere</w:t></w:r></w:moveTo><w:moveFrom w:id="3"><w:r><w:t>MovedAway</w:t></w:r></w:moveFrom><w:del w:id="4"><w:r><w:delText>Gone</w:delText></w:r></w:del></w:p>"#,
        r#"<w:p><w:r><w:t>Before</w:t></w:r><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wps:txbx xmlns:wps="x"><w:txbxContent><w:p><w:r><w:t>BoxText</w:t></w:r></w:p></w:txbxContent></wps:txbx></w:drawing></mc:Choice><mc:Fallback><w:pict><v:textbox xmlns:v="v"><w:txbxContent><w:p><w:r><w:t>BoxText</w:t></w:r></w:p></w:txbxContent></v:textbox></w:pict></mc:Fallback></mc:AlternateContent></w:r><w:r><w:t>After</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:pict><v:shape xmlns:v="v"><v:textbox><w:txbxContent><w:p><w:r><w:t>OldBox</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>well</w:t></w:r><w:r><w:noBreakHyphen/></w:r><w:r><w:t>known</w:t></w:r><w:r><w:sym w:font="Wingdings" w:char="F04A"/></w:r></w:p>"#,
        r#"<w:p><w:pPr><w:tabs><w:tab w:val="left" w:pos="720"/></w:tabs></w:pPr><w:r><w:t>Tabs</w:t></w:r></w:p>"#,
        r#"<w:p><m:oMathPara xmlns:m="m"><m:oMath><m:r><m:t>E=mc2</m:t></m:r></m:oMath></m:oMathPara></w:p>"#,
        r#"<w:p><w:customXml w:element="x"><w:r><w:t>Custom</w:t></w:r></w:customXml></w:p>"#,
    );
    let got = text(read(&Word::default(), "v.docx", &sample::docx(body)));
    eprintln!("{got}");
    for want in ["Inserted", "Linked", "Tagged", "Control", "Simple", "Result", "MovedHere", "BoxText", "OldBox", "E=mc2", "Custom", "Tabs"] {
        assert!(got.contains(want), "{want} missing from {got:?}");
    }
    for gone in ["PAGE", "MovedAway", "Gone"] {
        assert!(!got.contains(gone), "{gone} in {got:?}");
    }
    assert_eq!(got.matches("BoxText").count(), 1);
    // A text box's text is a line of its own; a non-breaking hyphen is a
    // hyphen; a symbol font's private-use character is left out.
    assert!(got.contains("\nBefore\nBoxText\nAfter\n"), "{got:?}");
    assert!(got.contains("\nwell-known\n"), "{got:?}");
}

#[test]
fn review_word_pages() {
    let p = |t: &str| format!("<w:p><w:r><w:t>{t}</w:t></w:r></w:p>");
    // Two typed page breaks in a row (a blank page), then a rendered break.
    let body = format!(
        "{}<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p><w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>{}<w:p><w:r><w:lastRenderedPageBreak/><w:t>three</w:t></w:r></w:p>",
        p("one"),
        p("two")
    );
    let got = text(read(&Word::default(), "p.docx", &sample::docx(&body)));
    // Every typed break counts: page 2 is blank, as Word shows it.
    assert_eq!(got, "[page 1]one\n[page 3]two\n[page 4]three\n");
    // A rendered break mid-paragraph.
    let body = "<w:p><w:r><w:t>first half </w:t></w:r><w:r><w:lastRenderedPageBreak/><w:t>second half</w:t></w:r></w:p>";
    let got = text(read(&Word::default(), "p2.docx", &sample::docx(body)));
    eprintln!("mid: {got:?}");
    assert_eq!(got, "[page 1]first half \n[page 2]second half\n");
}

// ---------- Excel correctness ----------

#[test]
fn review_excel_values() {
    let styles = r#"<numFmts count="1"><numFmt numFmtId="164" formatCode="[$-409]d\-mmm\-yy;@"/></numFmts><cellXfs><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="164"/><xf numFmtId="20"/></cellXfs>"#;
    let sheet = concat!(
        r#"<row><c t="s"><v>0</v></c><c t="s"><v>1</v></c><c t="inlineStr"><is><r><t>in</t></r><r><t>line</t></r><rPh><t>ruby</t></rPh></is></c></row>"#,
        r#"<row><c t="b"><v>1</v></c><c t="b"><v>0</v></c><c t="e"><v>#DIV/0!</v></c><c t="str"><f>A1&amp;B1</f><v>formula text</v></c><c><f>1+1</f><v>2</v></c></row>"#,
        r#"<row><c s="1"><v>0</v></c><c s="1"><v>1</v></c><c s="1"><v>60</v></c><c s="1"><v>61</v></c><c s="2"><v>46329</v></c><c s="3"><v>0.5</v></c></row>"#,
        r#"<row><c t="d"><v>2026-11-03T00:00:00</v></c><c><v>1.5E3</v></c><c t="n" s="1"><v>-1</v></c></row>"#,
    );
    let shared = [r#"<r><t>Ri</t></r><r><rPr/><t>ch</t></r>"#, "<t>plain</t>"];
    let got = text(read(&Excel::default(), "v.xlsx", &sample::xlsx(&[("S", sheet)], &shared, styles, false)));
    eprintln!("1900: {got:?}");
    let got1904 = text(read(&Excel::default(), "v.xlsx", &sample::xlsx(&[("S", sheet)], &shared, styles, true)));
    eprintln!("1904: {got1904:?}");
    assert!(got.contains("Rich\tplain\tinline\n"));
    assert!(got.contains("TRUE\tFALSE\t#DIV/0!\tformula text\t2\n"));
    assert!(got.contains("2026-11-03"), "{got}");
    // Serial 0 is "1900-01-00" in Excel: shown as the number it is.
    assert!(got.contains("\n0\t1900-01-01\t1900-02-29\t1900-03-01\t"), "{got}");
    assert!(got1904.contains("1904-01-01\t1904-01-02"), "{got1904}");
}

#[test]
fn review_excel_hidden_and_chart_sheets() {
    let mut wb = format!(r#"<?xml version="1.0"?><workbook {S}><sheets>"#);
    wb.push_str(r#"<sheet name="Visible" sheetId="1" r:id="rId1"/><sheet name="Hidden" sheetId="2" state="hidden" r:id="rId2"/><sheet name="VeryHidden" sheetId="3" state="veryHidden" r:id="rId3"/>"#);
    wb.push_str("</sheets></workbook>");
    let rels = sample::rels(&[
        ("rId1", "worksheet", "worksheets/sheet1.xml"),
        ("rId2", "worksheet", "worksheets/sheet2.xml"),
        ("rId3", "worksheet", "worksheets/sheet3.xml"),
    ]);
    let root = sample::rels(&[("rId1", "officeDocument", "xl/workbook.xml")]);
    let sh = |t: &str| format!(r#"<worksheet {S}><sheetData><row><c t="inlineStr"><is><t>{t}</t></is></c></row></sheetData></worksheet>"#);
    let (a, b, c) = (sh("v"), sh("h"), sh("vh"));
    let bytes = sample::zip(&[
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", wb.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", a.as_bytes()),
        ("xl/worksheets/sheet2.xml", b.as_bytes()),
        ("xl/worksheets/sheet3.xml", c.as_bytes()),
    ]);
    let got = text(read(&Excel::default(), "h.xlsx", &bytes));
    eprintln!("{got:?}");
    // Decided: very hidden sheets (reachable only from macros) are left
    // out; hidden ones are read, labelled as hidden.
    assert_eq!(got, "[Visible]v\n[Hidden (hidden)]h\n");
}

// ---------- PowerPoint correctness ----------

#[test]
fn review_pptx_groups_tables_and_order() {
    let group = format!("<p:grpSp><p:nvGrpSpPr/>{}<p:grpSp>{}</p:grpSp></p:grpSp>", shape(&["outer"]), shape(&["inner"]));
    let table = r#"<p:graphicFrame><a:graphic><a:graphicData><a:tbl><a:tr><a:tc><a:txBody><a:p><a:r><a:t>c1</a:t></a:r></a:p><a:p><a:r><a:t>more</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>c2</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#;
    let s1 = format!("{group}{table}");
    let s2 = shape(&["second", "line<a:br/>broken"]);
    let got = text(read(&PowerPoint::default(), "g.pptx", &sample::pptx(&[(&s1, Some("speaker")), (&s2, None)])));
    eprintln!("{got:?}");
    assert_eq!(got, "[slide 1]outer\ninner\nc1 more\tc2\n[slide 1 notes]speaker\n[slide 2]second\nline\nbroken\n");
}

// ---------- Status ----------

#[test]
fn review_statuses() {
    // A zip whose only part is stored with an unsupported method (bzip2 = 12).
    let mut bytes = sample::docx(&paras(&["x"]));
    for i in 0..bytes.len() - 46 {
        if bytes[i..i + 4] == *b"PK\x01\x02" && bytes[i + 46..].starts_with(b"word/document.xml") {
            bytes[i + 10] = 12;
        }
        if bytes[i..i + 4] == *b"PK\x03\x04" && bytes[i + 30..].starts_with(b"word/document.xml") {
            bytes[i + 8] = 12;
        }
    }
    eprintln!("bzip2 part: {:?}", read(&Word::default(), "m.docx", &bytes).map(|_| ()));
    // A pptx named .docx.
    let pptx = sample::pptx(&[(&shape(&["a"]), None)]);
    eprintln!("pptx as docx: {:?}", read(&Word::default(), "p.docx", &pptx).map(|_| ()));
    // A workbook named .docx.
    let xlsx = sample::xlsx(&[("A", "<row><c><v>1</v></c></row>")], &[], "", false);
    eprintln!("xlsx as docx: {:?}", read(&Word::default(), "x.docx", &xlsx).map(|_| ()));
    eprintln!("docx as pptx: {:?}", read(&PowerPoint::default(), "x.pptx", &sample::docx("")).map(|_| ()));
    // An empty-but-valid zip (only end record).
    let empty = stored(&[]);
    eprintln!("empty zip: {:?}", read(&Word::default(), "e.docx", &empty).map(|_| ()));
    // Garbage after PK\x03\x04.
    eprintln!("PK garbage: {:?}", read(&Word::default(), "g.docx", b"PK\x03\x04garbage").map(|_| ()));
}
