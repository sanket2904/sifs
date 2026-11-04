use super::excel::{show_date, shown_by_code, shown_by_id, Shown};
use super::sample::{self, paras, shape, W};
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

fn parts(read: Read) -> Vec<(String, String)> {
    read.unwrap().0.into_iter().map(|p| (p.label, p.text)).collect()
}

fn fact<'a>(facts: &'a [(String, String)], key: &str) -> Option<&'a str> {
    facts.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn skipped(read: Read) -> String {
    match read {
        Err(ReadError::Unsupported(why)) => why,
        other => panic!("expected skipped, got {other:?}"),
    }
}

fn failed(read: Read) -> String {
    match read {
        Err(ReadError::Other(why)) => why,
        other => panic!("expected failed, got {other:?}"),
    }
}

// --- Word ---

#[test]
fn word_text_comes_in_order_with_tabs_breaks_and_tables() {
    let body = format!(
        "{}{}{}{}{}{}",
        r#"<w:p><w:pPr><w:tabs><w:tab w:val="left" w:pos="720"/></w:tabs></w:pPr><w:r><w:t>Rent</w:t></w:r><w:r><w:tab/><w:t xml:space="preserve">due </w:t></w:r><w:r><w:t>monthly</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>first line</w:t><w:br/><w:t>second line</w:t></w:r></w:p>"#,
        r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Month</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Amount</w:t></w:r></w:p><w:p><w:r><w:t>INR</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>March</w:t></w:r></w:p></w:tc><w:tc><w:p/></w:tc><w:tc><w:p><w:r><w:t>25000</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        r#"<w:p><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><w:txbxContent><w:p><w:r><w:t>harbour</w:t></w:r></w:p></w:txbxContent></w:drawing></mc:Choice><mc:Fallback><w:pict><w:txbxContent><w:p><w:r><w:t>harbour</w:t></w:r></w:p></w:txbxContent></w:pict></mc:Fallback></mc:AlternateContent></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>kept</w:t></w:r><w:del><w:r><w:delText>struck</w:delText></w:r></w:del><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>PAGE \* MERGEFORMAT</w:instrText></w:r><w:r><w:t xml:space="preserve"> and shown</w:t></w:r></w:p>"#,
        r#"<w:p><m:oMath><m:r><m:t>E=mc²</m:t></m:r></m:oMath><w:r><w:t>Tom &amp; Jerry &#8364;5 &lt;3</w:t></w:r></w:p>"#,
    );
    let got = parts(read(&Word::default(), "a.docx", &sample::docx(&body)));
    assert_eq!(
        got,
        vec![(
            "".to_string(),
            "Rent\tdue monthly\nfirst line\nsecond line\nMonth\tAmount INR\nMarch\t25000\nharbour\nkept and shown\nE=mc²Tom & Jerry €5 <3\n"
                .to_string()
        )]
    );
}

#[test]
fn word_pages_come_from_rendered_and_typed_breaks_and_a_doubled_break_counts_once() {
    let body = format!(
        "{}{}{}{}",
        paras(&["one"]),
        r#"<w:p><w:r><w:t>two</w:t></w:r><w:r><w:br w:type="page"/></w:r></w:p>"#,
        r#"<w:p><w:r><w:lastRenderedPageBreak/><w:t>three</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>four </w:t></w:r><w:r><w:lastRenderedPageBreak/><w:t>five</w:t></w:r></w:p>"#,
    );
    let got = parts(read(&Word::default(), "p.docx", &sample::docx(&body)));
    assert_eq!(
        got,
        vec![
            ("page 1".to_string(), "one\ntwo\n".to_string()),
            ("page 2".to_string(), "three\nfour \n".to_string()),
            ("page 3".to_string(), "five\n".to_string()),
        ]
    );
}

#[test]
fn word_headers_footers_notes_and_comments_are_parts_of_their_own_and_the_facts_are_kept() {
    let part = |root: &str, inner: &str| format!(r#"<?xml version="1.0"?><w:{root} {W}>{inner}</w:{root}>"#);
    let extra = [
        ("word/header1.xml", part("hdr", &paras(&["Confidential"]))),
        ("word/footer1.xml", part("ftr", &paras(&["Page footer"]))),
        (
            "word/footnotes.xml",
            part(
                "footnotes",
                &format!(r#"<w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:id="1">{}</w:footnote>"#, paras(&["See clause nine"])),
            ),
        ),
        ("word/comments.xml", part("comments", &format!(r#"<w:comment w:id="0" w:author="Ravi">{}</w:comment>"#, paras(&["Check the deposit"])))),
    ];
    let rels = [
        ("rId1", "footer", "footer1.xml"),
        ("rId2", "header", "header1.xml"),
        ("rId3", "footnotes", "footnotes.xml"),
        ("rId4", "comments", "comments.xml"),
        ("rId5", "hyperlink", "https://example.com"),
    ];
    let (got, facts) = read(&Word::default(), "h.docx", &sample::docx_with(&paras(&["Body"]), &extra, &rels)).unwrap();
    let got: Vec<(String, String)> = got.into_iter().map(|p| (p.label, p.text)).collect();
    assert_eq!(
        got,
        vec![
            ("".to_string(), "Body\n".to_string()),
            ("headers and footers".to_string(), "Confidential\nPage footer\n".to_string()),
            ("footnotes".to_string(), "See clause nine\n".to_string()),
            ("comments".to_string(), "Check the deposit\n".to_string()),
        ]
    );
    assert_eq!(fact(&facts, "title"), Some("Rent agreement"));
    assert_eq!(fact(&facts, "author"), Some("Asha Rao"));
    assert_eq!(fact(&facts, "created"), Some("2026-11-01T09:15:00Z"));
    assert_eq!(fact(&facts, "modified"), Some("2026-11-03T10:00:00Z"));
    assert_eq!(fact(&facts, "pages"), Some("3"));
}

#[test]
fn a_doctype_entity_is_never_expanded() {
    let lol = r#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;"><!ENTITY lol3 "&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;">]>"#;
    let document = format!(r#"{lol}<w:document {W}><w:body><w:p><w:r><w:t>before &lol3; after</w:t></w:r></w:p></w:body></w:document>"#);
    let package_rels = sample::rels(&[("rId1", "officeDocument", "word/document.xml")]);
    let bytes =
        sample::zip(&[("_rels/.rels", package_rels.as_bytes()), ("word/document.xml", document.as_bytes())]);
    assert_eq!(parts(read(&Word::default(), "lol.docx", &bytes)), vec![("".into(), "before  after\n".into())]);
}

// --- Excel ---

const STYLES: &str = r#"<numFmts count="2"><numFmt numFmtId="164" formatCode="yyyy\-mm\-dd hh:mm"/><numFmt numFmtId="165" formatCode="&quot;Rs&quot; #,##0.00"/></numFmts><cellStyleXfs count="1"><xf numFmtId="14"/></cellStyleXfs><cellXfs count="5"><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="164"/><xf numFmtId="20"/><xf numFmtId="165"/></cellXfs>"#;

#[test]
fn excel_sheets_are_parts_in_the_workbooks_order_with_cells_as_rows() {
    let first = concat!(
        r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="inlineStr"><is><t>Inline note</t></is></c></row>"#,
        r#"<row r="2"><c r="A2" t="s"><v>2</v></c><c r="C2"><v>25000.5</v></c><c r="D2" t="b"><v>1</v></c><c r="E2" t="e"><v>#DIV/0!</v></c></row>"#,
        r#"<row r="3"><c r="A3" s="1"><v>46329</v></c><c r="B3" s="2"><v>46329.6041666667</v></c><c r="C3" s="3"><v>0.6045138888889</v></c><c r="D3" s="4"><v>1234.5</v></c></row>"#,
        r#"<row r="4"><c r="A4" t="str"><f>CONCAT("a","b")</f><v>ab</v></c><c r="B4"><f>SUM(C2:C2)</f><v>25000.5</v></c><c r="C4" s="1"/></row>"#,
    );
    let second = r#"<row r="1"><c r="A1" t="s"><v>3</v></c></row>"#;
    let shared = [
        "<t>Month</t>",
        "<r><t>Rent </t></r><r><rPr><b/></rPr><t>INR</t></r>",
        "<t>March</t>",
        "<t>東京</t><rPh sb=\"0\" eb=\"2\"><t>トウキョウ</t></rPh>",
    ];
    let bytes = sample::xlsx(&[("Q2", first), ("Notes", second)], &shared, STYLES, false);
    let (got, facts) = read(&Excel::default(), "b.xlsx", &bytes).unwrap();
    let got: Vec<(String, String)> = got.into_iter().map(|p| (p.label, p.text)).collect();
    assert_eq!(
        got,
        vec![
            (
                "Q2".to_string(),
                "Month\tRent INR\tInline note\nMarch\t25000.5\tTRUE\t#DIV/0!\n2026-11-03\t2026-11-03 14:30\t14:30:30\t1234.5\nab\t25000.5\n"
                    .to_string()
            ),
            ("Notes".to_string(), "東京\n".to_string()),
        ]
    );
    assert_eq!(fact(&facts, "sheets"), Some("Q2, Notes"));
    assert_eq!(fact(&facts, "title"), Some("Budget"));
    assert_eq!(fact(&facts, "author"), Some("Ravi"));
}

#[test]
fn excel_dates_in_the_1904_system_are_shown_as_their_day() {
    let bytes = sample::xlsx(&[("S", r#"<row><c s="1"><v>44137</v></c></row>"#)], &[], STYLES, true);
    assert_eq!(parts(read(&Excel::default(), "m.xlsx", &bytes)), vec![("S".into(), "2024-11-03\n".into())]);
}

#[test]
fn date_formats_and_serials_are_recognised() {
    assert_eq!(shown_by_id(14), Shown::Date);
    assert_eq!(shown_by_id(22), Shown::DateTime);
    assert_eq!(shown_by_id(20), Shown::Time);
    assert_eq!(shown_by_id(4), Shown::Number);
    assert_eq!(shown_by_code("dd/mm/yyyy"), Shown::Date);
    assert_eq!(shown_by_code("mmm yy"), Shown::Date);
    assert_eq!(shown_by_code("[h]:mm:ss"), Shown::Time);
    assert_eq!(shown_by_code("mm:ss"), Shown::Time);
    assert_eq!(shown_by_code("[$-409]d-mmm-yy h:mm AM/PM"), Shown::DateTime);
    assert_eq!(shown_by_code("[Red]#,##0.00"), Shown::Number);
    assert_eq!(shown_by_code("\"days\" 0"), Shown::Number);
    assert_eq!(shown_by_code("0.00\\h"), Shown::Number);
    assert_eq!(shown_by_code("General"), Shown::Number);
    let date = |s: f64| show_date(s, Shown::Date, false).unwrap();
    assert_eq!(date(1.0), "1900-01-01");
    assert_eq!(date(59.0), "1900-02-28");
    assert_eq!(date(60.0), "1900-02-29");
    assert_eq!(date(61.0), "1900-03-01");
    assert_eq!(date(45599.0), "2024-11-03");
    assert_eq!(show_date(0.0, Shown::Date, true).unwrap(), "1904-01-01");
    assert_eq!(show_date(45599.99999999, Shown::DateTime, false).unwrap(), "2024-11-04 00:00");
    assert_eq!(show_date(-1.0, Shown::Date, false), None);
}

// --- PowerPoint ---

#[test]
fn slides_come_in_the_presentations_order_each_followed_by_its_notes() {
    let table = r#"<p:graphicFrame><a:graphic><a:graphicData><a:tbl><a:tr><a:tc><a:txBody><a:p><a:r><a:t>Year</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>Users</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>"#;
    let first = format!("{}{}", shape(&["Pitch deck", "Seed round"]), r#"<p:sp><p:txBody><a:p><a:r><a:t>line</a:t></a:r><a:br/><a:r><a:t>break</a:t></a:r></a:p></p:txBody></p:sp>"#);
    let second = format!("{}{}", shape(&["Traction"]), table);
    let bytes = sample::pptx(&[(&first, Some("Say hello")), (&second, None), (&shape(&["Thanks"]), Some("Questions"))]);
    let (got, facts) = read(&PowerPoint::default(), "c.pptx", &bytes).unwrap();
    let got: Vec<(String, String)> = got.into_iter().map(|p| (p.label, p.text)).collect();
    assert_eq!(
        got,
        vec![
            ("slide 1".to_string(), "Pitch deck\nSeed round\nline\nbreak\n".to_string()),
            ("slide 1 notes".to_string(), "Say hello\n".to_string()),
            ("slide 2".to_string(), "Traction\nYear\tUsers\n".to_string()),
            ("slide 3".to_string(), "Thanks\n".to_string()),
            ("slide 3 notes".to_string(), "Questions\n".to_string()),
        ]
    );
    assert_eq!(fact(&facts, "slides"), Some("3"));
    assert_eq!(fact(&facts, "title"), Some("Pitch"));
}

// --- Unicode ---

#[test]
fn hindi_japanese_and_emoji_come_through_in_every_kind() {
    let words = "किराया 東京の家賃 I❤️you 🏠";
    let docx = sample::docx(&paras(&[words]));
    assert_eq!(parts(read(&Word::default(), "u.docx", &docx)), vec![("".into(), format!("{words}\n"))]);
    let xlsx = sample::xlsx(&[("सूची", r#"<row><c t="s"><v>0</v></c></row>"#)], &[&format!("<t>{words}</t>")], "", false);
    assert_eq!(parts(read(&Excel::default(), "u.xlsx", &xlsx)), vec![("सूची".into(), format!("{words}\n"))]);
    let pptx = sample::pptx(&[(&shape(&[words]), Some(words))]);
    assert_eq!(
        parts(read(&PowerPoint::default(), "u.pptx", &pptx)),
        vec![("slide 1".into(), format!("{words}\n")), ("slide 1 notes".into(), format!("{words}\n"))]
    );
}

// --- Safety ---

#[test]
fn a_zip_bomb_is_skipped_by_what_it_unpacks_to() {
    // 2 MB of spaces deflate to about 2 KB: a ratio of a thousand.
    let body = format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", " ".repeat(2 * MB as usize));
    assert_eq!(skipped(read(&Word::default(), "bomb.docx", &sample::docx(&body))), BOMB);
}

#[test]
fn past_the_cap_on_unpacking_reading_stops_and_keeps_what_it_read() {
    // Words that barely compress, so no ratio check fires.
    let mut seed = 7u64;
    let mut word = || {
        (0..8)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (b'a' + (seed >> 59) as u8 % 26) as char
            })
            .collect::<String>()
    };
    let lines: Vec<String> = (0..4000).map(|_| (0..8).map(|_| word()).collect::<Vec<_>>().join(" ")).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let bytes = sample::docx(&paras(&lines));
    let word = Word { caps: Caps { inflated: 100_000, ..Caps::default() } };
    let (got, facts) = read(&word, "big.docx", &bytes).unwrap();
    let text: String = got.into_iter().map(|p| p.text).collect();
    assert!(text.starts_with(lines[0]) && text.len() < 100_000, "{}", text.len());
    assert_eq!(fact(&facts, "truncated"), Some("1"));
    let (got, facts) = read(&Word::default(), "big.docx", &bytes).unwrap();
    assert!(got[0].text.contains(lines[3999]));
    assert_eq!(fact(&facts, "truncated"), None);
}

#[test]
fn a_zip_with_too_many_parts_is_skipped() {
    let names: Vec<String> = (0..60).map(|i| format!("media/{i}.bin")).collect();
    let mut parts: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    let rels = sample::rels(&[("rId1", "officeDocument", "word/document.xml")]);
    let doc = format!("<w:document {W}><w:body>{}</w:body></w:document>", paras(&["hi"]));
    parts.push(("_rels/.rels", rels.as_bytes()));
    parts.push(("word/document.xml", doc.as_bytes()));
    let bytes = sample::zip(&parts);
    let word = Word { caps: Caps { entries: 50, ..Caps::default() } };
    assert_eq!(skipped(read(&word, "many.docx", &bytes)), "more than 50 parts");
    assert_eq!(parts_of(read(&Word::default(), "many.docx", &bytes)), "hi\n");
}

fn parts_of(read: Read) -> String {
    read.unwrap().0.into_iter().map(|p| p.text).collect()
}

#[test]
fn a_zip_whose_list_of_parts_is_too_large_is_never_taken_in() {
    // 1,000 parts with 100-character names: a 150 KB central directory.
    let names: Vec<String> = (0..1000).map(|i| format!("media/{i:0>94}.bin")).collect();
    let mut parts: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    let rels = sample::rels(&[("rId1", "officeDocument", "word/document.xml")]);
    let doc = format!("<w:document {W}><w:body>{}</w:body></w:document>", paras(&["hi"]));
    parts.push(("_rels/.rels", rels.as_bytes()));
    parts.push(("word/document.xml", doc.as_bytes()));
    let bytes = sample::zip(&parts);
    let word = Word { caps: Caps { directory_bytes: 64 * 1024, ..Caps::default() } };
    assert_eq!(failed(read(&word, "dir.docx", &bytes)), "damaged, or its list of parts is larger than 64 KB");
    assert_eq!(parts_of(read(&Word::default(), "dir.docx", &bytes)), "hi\n");
}

#[test]
fn a_password_protected_file_is_skipped_as_one_and_an_old_format_as_another() {
    let encrypted = sample::compound(&["EncryptionInfo", "EncryptedPackage"]);
    for reader in [&Word::default() as &dyn Reader, &Excel::default(), &PowerPoint::default()] {
        let name = format!("secret.{}", reader.exts()[0]);
        assert_eq!(skipped(read(reader, &name, &encrypted)), PASSWORD);
    }
    let old = sample::compound(&["WordDocument", "1Table"]);
    assert_eq!(skipped(read(&Word::default(), "old.docx", &old)), "an older Word, Excel or PowerPoint format");
    // A compound file whose directory can't be followed.
    let mut broken = encrypted.clone();
    broken.truncate(600);
    assert_eq!(
        skipped(read(&Word::default(), "broken.docx", &broken)),
        "password-protected, or an older Word, Excel or PowerPoint format"
    );
}

#[test]
fn a_zip_with_encrypted_parts_is_skipped_as_password_protected() {
    let mut bytes = sample::docx(&paras(&["secret"]));
    // Set the encrypted flag on the document's entry in the list of parts.
    let name = b"word/document.xml";
    let at = (0..bytes.len() - 46)
        .find(|&i| bytes[i..i + 4] == *b"PK\x01\x02" && bytes[i + 46..].starts_with(name))
        .unwrap();
    bytes[at + 8] |= 1;
    assert_eq!(skipped(read(&Word::default(), "zipcrypto.docx", &bytes)), PASSWORD);
}

#[test]
fn a_truncated_zip_is_recorded_damaged() {
    let bytes = sample::docx(&paras(&["hello"]));
    let why = failed(read(&Word::default(), "cut.docx", &bytes[..bytes.len() / 2]));
    assert!(why.starts_with("damaged or incomplete"), "{why}");
    // Its parts' data cut short, its list of parts intact: still damaged.
    let mut bytes = bytes.clone();
    let at = bytes.windows(17).position(|w| w == b"word/document.xml").unwrap() + 40;
    for b in &mut bytes[at..at + 30] {
        *b ^= 0x55;
    }
    let why = failed(read(&Word::default(), "flipped.docx", &bytes));
    assert!(why.starts_with("damaged or incomplete"), "{why}");
}

#[test]
fn a_file_that_is_not_the_kind_its_name_says_is_skipped() {
    let zip = sample::zip(&[("readme.txt", b"just a zip")]);
    assert_eq!(skipped(read(&Word::default(), "fake.docx", &zip)), "not a Word document");
    assert_eq!(skipped(read(&Excel::default(), "fake.xlsx", &zip)), "not an Excel workbook");
    assert_eq!(skipped(read(&PowerPoint::default(), "fake.pptx", &zip)), "not a PowerPoint presentation");
    assert_eq!(skipped(read(&Word::default(), "text.docx", b"plain words")), "not a Word document");
    assert_eq!(skipped(read(&Word::default(), "empty.docx", b"")), "not a Word document");
    // A Word document named as a workbook.
    let docx = sample::docx(&paras(&["hi"]));
    assert_eq!(skipped(read(&Excel::default(), "doc.xlsx", &docx)), "not an Excel workbook");
}

#[test]
fn a_sheet_past_the_cell_cap_is_cut_and_recorded() {
    let row: String = (0..30).map(|i| format!("<c><v>{i}</v></c>")).collect();
    let bytes = sample::xlsx(&[("S", &format!("<row>{row}</row>")), ("T", "<row><c><v>99</v></c></row>")], &[], "", false);
    let excel = Excel { caps: Caps { cells: 10, ..Caps::default() } };
    let (got, facts) = read(&excel, "wide.xlsx", &bytes).unwrap();
    // Cells are written as they end, so the first ten are kept.
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].text, "0\t1\t2\t3\t4\t5\t6\t7\t8\t9");
    assert_eq!(fact(&facts, "truncated"), Some("1"));
    let rows: String = (0..30).map(|i| format!("<row><c><v>{i}</v></c></row>")).collect();
    let bytes = sample::xlsx(&[("S", &rows), ("T", "<row><c><v>99</v></c></row>")], &[], "", false);
    let (got, facts) = read(&excel, "tall.xlsx", &bytes).unwrap();
    assert_eq!(got[0].text, "0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n");
    assert_eq!(got.len(), 1);
    assert_eq!(fact(&facts, "truncated"), Some("1"));
    let (got, facts) = read(&Excel::default(), "tall.xlsx", &bytes).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(fact(&facts, "truncated"), None);
}

#[test]
fn shared_strings_past_their_cap_read_as_empty_and_are_recorded() {
    let sheet = r#"<row><c t="s"><v>0</v></c><c t="s"><v>1</v></c><c t="s"><v>2</v></c></row>"#;
    let bytes = sample::xlsx(&[("S", sheet)], &["<t>short</t>", "<t>longer string</t>", "<t>end</t>"], "", false);
    let excel = Excel { caps: Caps { shared_chars: 10, ..Caps::default() } };
    let (got, facts) = read(&excel, "s.xlsx", &bytes).unwrap();
    assert_eq!(got[0].text, "short\n");
    assert_eq!(fact(&facts, "truncated"), Some("1"));
    let (got, _) = read(&Excel::default(), "s.xlsx", &bytes).unwrap();
    assert_eq!(got[0].text, "short\tlonger string\tend\n");
}

#[test]
fn a_reader_past_its_time_stops_itself_even_on_parts_without_text() {
    let row: String = (0..10_000).map(|_| "<c/>").collect();
    let bytes = sample::xlsx(&[("S", &format!("<row>{row}</row>"))], &[], "", false);
    let now = Instant::now();
    assert!(matches!(read_at(&Excel::default(), "slow.xlsx", &bytes, now), Err(ReadError::Timeout)));
    assert!(read(&Excel::default(), "slow.xlsx", &bytes).is_ok());
}

#[test]
fn the_caps_and_limits_are_the_planned_ones() {
    assert_eq!(
        Caps::default(),
        Caps {
            directory_bytes: 4 * MB,
            entries: 10_000,
            ratio: 100,
            ratio_after: MB,
            inflated: 128 * MB,
            cells: 1_000_000,
            shared_chars: 4_000_000,
            shared_strings: 1_000_000,
            listed: 10_000,
            event: 4 * MB as usize,
        }
    );
    for reader in [&Word::default() as &dyn Reader, &Excel::default(), &PowerPoint::default()] {
        let limits = reader.limits();
        assert_eq!(limits.max_bytes, 100 * MB, "{}", reader.id());
        assert_eq!(limits.max_chars, 1_000_000);
        assert_eq!(limits.time, Duration::from_secs(10));
    }
}

#[test]
fn relationship_targets_resolve_within_the_package() {
    assert_eq!(resolve("word", "header1.xml"), "word/header1.xml");
    assert_eq!(resolve("ppt/slides", "../notesSlides/notesSlide1.xml"), "ppt/notesSlides/notesSlide1.xml");
    assert_eq!(resolve("xl", "/xl/worksheets/sheet1.xml"), "xl/worksheets/sheet1.xml");
    assert_eq!(resolve("", "word/document.xml"), "word/document.xml");
    assert_eq!(resolve("a", "../../../x.xml"), "x.xml");
}

// --- From the step 2 review: each test fails without its rule ---

/// Letters that don't compress much, so a test file is no zip bomb.
fn letters(n: usize, seed: &mut u64) -> String {
    (0..n)
        .map(|_| {
            *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (b'a' + ((*seed >> 33) % 26) as u8) as char
        })
        .collect()
}

/// A workbook listing these (name, relationship id) sheets, each id
/// pointing at a sheet with one inline string: (id, text).
fn workbook(listed: &[(&str, &str)], sheets: &[(&str, &str)]) -> Vec<u8> {
    let mut wb = format!(r#"<?xml version="1.0"?><workbook {}><sheets>"#, sample::S);
    for (name, id) in listed {
        wb.push_str(&format!(r#"<sheet name="{name}" r:id="{id}"/>"#));
    }
    wb.push_str("</sheets></workbook>");
    let targets: Vec<(String, String)> =
        sheets.iter().map(|(id, _)| (id.to_string(), format!("worksheets/{id}.xml"))).collect();
    let rels: Vec<(&str, &str, &str)> = targets.iter().map(|(id, t)| (id.as_str(), "worksheet", t.as_str())).collect();
    let mut parts: Vec<(String, Vec<u8>)> = vec![
        ("_rels/.rels".into(), sample::rels(&[("rId1", "officeDocument", "xl/workbook.xml")]).into_bytes()),
        ("xl/workbook.xml".into(), wb.into_bytes()),
        ("xl/_rels/workbook.xml.rels".into(), sample::rels(&rels).into_bytes()),
    ];
    for (id, text) in sheets {
        let sheet = format!(
            r#"<worksheet {}><sheetData><row><c t="inlineStr"><is><t>{text}</t></is></c></row></sheetData></worksheet>"#,
            sample::S
        );
        parts.push((format!("xl/worksheets/{id}.xml"), sheet.into_bytes()));
    }
    let parts: Vec<(&str, &[u8])> = parts.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    sample::zip(&parts)
}

#[test]
fn past_the_cap_on_sheets_and_slides_the_rest_are_left_out_and_recorded() {
    let two = Caps { listed: 2, ..Caps::default() };
    let bytes = sample::xlsx(&[("A", "<row><c><v>1</v></c></row>"), ("B", "<row><c><v>2</v></c></row>"), ("C", "<row><c><v>3</v></c></row>")], &[], "", false);
    let (got, facts) = read(&Excel { caps: two }, "s.xlsx", &bytes).unwrap();
    let labels: Vec<&str> = got.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["A", "B"]);
    assert_eq!(fact(&facts, "sheets"), Some("A, B"));
    assert_eq!(fact(&facts, "truncated"), Some("1"));

    let bytes = sample::pptx(&[(&shape(&["one"]), None), (&shape(&["two"]), None), (&shape(&["three"]), None)]);
    let (got, facts) = read(&PowerPoint { caps: two }, "s.pptx", &bytes).unwrap();
    let labels: Vec<&str> = got.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["slide 1", "slide 2"]);
    assert_eq!(fact(&facts, "slides"), Some("3"), "the count is the presentation's");
    assert_eq!(fact(&facts, "truncated"), Some("1"));
    // At the cap exactly, nothing is left out.
    let (_, facts) = read(&PowerPoint { caps: Caps { listed: 3, ..Caps::default() } }, "s.pptx", &bytes).unwrap();
    assert_eq!(fact(&facts, "truncated"), None);
}

#[test]
fn a_sheet_or_slide_listed_twice_is_read_once() {
    let bytes = workbook(&[("A", "rId1"), ("B", "rId1"), ("C", "rId2")], &[("rId1", "first"), ("rId2", "second")]);
    assert_eq!(
        parts(read(&Excel::default(), "twice.xlsx", &bytes)),
        [("A".to_string(), "first\n".to_string()), ("C".to_string(), "second\n".to_string())]
    );
    // Both slide entries name the same relationship.
    let pres = format!(
        r#"<?xml version="1.0"?><p:presentation {}><p:sldIdLst><p:sldId id="256" r:id="rId0"/><p:sldId id="257" r:id="rId0"/></p:sldIdLst></p:presentation>"#,
        sample::P
    );
    let slide = format!(r#"<p:sld {}><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sld>"#, sample::P, shape(&["only"]));
    let bytes = sample::zip(&[
        ("_rels/.rels", sample::rels(&[("rId1", "officeDocument", "ppt/presentation.xml")]).as_bytes()),
        ("ppt/presentation.xml", pres.as_bytes()),
        ("ppt/_rels/presentation.xml.rels", sample::rels(&[("rId0", "slide", "slides/slide1.xml")]).as_bytes()),
        ("ppt/slides/slide1.xml", slide.as_bytes()),
    ]);
    assert_eq!(parts(read(&PowerPoint::default(), "twice.pptx", &bytes)), [("slide 1".to_string(), "only\n".to_string())]);
}

#[test]
fn past_the_cap_on_shared_strings_the_rest_of_the_list_is_not_read() {
    let sheet = r#"<row><c t="s"><v>0</v></c><c t="s"><v>1</v></c><c t="s"><v>2</v></c></row>"#;
    let bytes = sample::xlsx(&[("S", sheet)], &["<t>a</t>", "<t>b</t>", "<t>c</t>"], "", false);
    let (got, facts) = read(&Excel { caps: Caps { shared_strings: 2, ..Caps::default() } }, "n.xlsx", &bytes).unwrap();
    assert_eq!(got[0].text, "a\tb\n", "the sheets are still read");
    assert_eq!(fact(&facts, "truncated"), Some("1"));
    let (got, facts) = read(&Excel { caps: Caps { shared_strings: 3, ..Caps::default() } }, "n.xlsx", &bytes).unwrap();
    assert_eq!(got[0].text, "a\tb\tc\n");
    assert_eq!(fact(&facts, "truncated"), None);
}

#[test]
fn a_cells_value_is_kept_up_to_excels_own_limit_in_characters() {
    let max = super::excel::MAX_CELL_CHARS;
    for letter in ["a", "é"] {
        for (n, kept, cut) in [(max, max, false), (max + 1, max, true)] {
            let sheet = format!(r#"<row><c t="inlineStr"><is><t>{}</t></is></c></row>"#, letter.repeat(n));
            let bytes = sample::xlsx(&[("S", &sheet)], &[], "", false);
            let (got, facts) = read(&Excel::default(), "long.xlsx", &bytes).unwrap();
            assert_eq!(got[0].text, format!("{}\n", letter.repeat(kept)), "{n} of {letter}");
            assert_eq!(fact(&facts, "truncated").is_some(), cut, "{n} of {letter}");
        }
    }
}

#[test]
fn page_breaks_inside_a_table_move_the_rows_after_them() {
    let row = |cells: &[&str]| -> String {
        let cells: String = cells.iter().map(|c| format!("<w:tc>{c}</w:tc>")).collect();
        format!("<w:tr>{cells}</w:tr>")
    };
    let p = |t: &str| format!("<w:p><w:r><w:t>{t}</w:t></w:r></w:p>");
    // Word marks the first run of a row that starts a new page.
    let body = format!(
        "{}<w:tbl>{}{}</w:tbl>{}",
        p("before"),
        row(&[&p("r1a"), &p("r1b")]),
        row(&["<w:p><w:r><w:lastRenderedPageBreak/><w:t>r2a</w:t></w:r></w:p>", &p("r2b")]),
        p("after"),
    );
    assert_eq!(
        parts(read(&Word::default(), "t.docx", &sample::docx(&body))),
        [("page 1".to_string(), "before\nr1a\tr1b\n".to_string()), ("page 2".to_string(), "r2a\tr2b\nafter\n".to_string())]
    );
    // A break typed in a row's second cell: the next row starts the page.
    let body = format!(
        "{}<w:tbl>{}{}</w:tbl>",
        p("before"),
        row(&[&p("x"), &format!("{}<w:p><w:r><w:br w:type=\"page\"/></w:r><w:r><w:t>z</w:t></w:r></w:p>", p("y"))]),
        row(&[&p("next")]),
    );
    assert_eq!(
        parts(read(&Word::default(), "t.docx", &sample::docx(&body))),
        [("page 1".to_string(), "before\nx\ty z\n".to_string()), ("page 2".to_string(), "next\n".to_string())]
    );
}

#[test]
fn a_compound_file_whose_directory_chain_loops_still_ends() {
    let mut bytes = sample::compound(&["EncryptionInfo", "EncryptedPackage"]);
    // The FAT entry of the directory's sector (1) points at itself.
    bytes[512 + 4..512 + 8].copy_from_slice(&1u32.to_le_bytes());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(read(&Word::default(), "loop.docx", &bytes).map(|_| ()));
    });
    let got = rx.recv_timeout(Duration::from_secs(10)).expect("reading a looping directory chain never ended");
    assert!(matches!(&got, Err(ReadError::Unsupported(why)) if why == "password-protected"), "{got:?}");
}

#[test]
fn a_long_paragraph_or_table_cell_is_written_as_it_comes() {
    // 8 MB in 1 KB runs: held whole, the line or cell would grow to 8 MB
    // before the sink (1M characters) cut it.
    let mut seed = 7;
    let runs: String = (0..8 * 1024).map(|_| format!("<w:r><w:t>{}</w:t></w:r>", letters(1024, &mut seed))).collect();
    for (what, body) in [
        ("paragraph", format!("<w:p>{runs}</w:p>")),
        ("cell", format!("<w:tbl><w:tr><w:tc><w:p>{runs}</w:p></w:tc></w:tr></w:tbl>")),
        (
            "nested table's cell",
            format!("<w:tbl><w:tr><w:tc><w:tbl><w:tr><w:tc><w:p>{runs}</w:p></w:tc></w:tr></w:tbl></w:tc></w:tr></w:tbl>"),
        ),
    ] {
        let bytes = sample::docx(&body);
        let (got, held) = sample::held::peak(|| read(&Word::default(), "long.docx", &bytes));
        let (parts, facts) = got.unwrap();
        assert_eq!(parts.iter().map(|p| p.text.chars().count()).sum::<usize>(), 1_000_000, "{what}");
        assert_eq!(fact(&facts, "truncated"), Some("1"));
        assert!(held < 6 * MB as usize, "a long {what} held {} KB", held / 1024);
    }
}

#[test]
fn a_documents_property_is_held_only_up_to_what_a_fact_keeps() {
    // A 6 MB title in many pieces (each &amp; splits the text).
    let mut seed = 9;
    let title: String = (0..600_000).map(|_| format!("{}&amp;", letters(5, &mut seed))).collect();
    let core = sample::core(&title, "Asha Rao");
    let bytes = sample::zip(&[
        (
            "_rels/.rels",
            sample::rels(&[("rId1", "officeDocument", "word/document.xml"), ("rId2", "core-properties", "docProps/core.xml")])
                .as_bytes(),
        ),
        ("docProps/core.xml", core.as_bytes()),
        ("word/document.xml", format!(r#"<w:document {W}><w:body>{}</w:body></w:document>"#, paras(&["x"])).as_bytes()),
    ]);
    let (got, held) = sample::held::peak(|| read(&Word::default(), "props.docx", &bytes));
    let (_, facts) = got.unwrap();
    assert_eq!(fact(&facts, "title").unwrap().chars().count(), crate::extract::MAX_FACT_CHARS);
    assert!(held < 2 * MB as usize, "the title held {} KB", held / 1024);
}

#[test]
fn an_external_relationship_is_never_read_as_a_part() {
    let footnotes = format!(r#"<w:footnotes {W}>{}</w:footnotes>"#, paras(&["note text"]));
    let doc_rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml" TargetMode="External"/></Relationships>"#;
    let bytes = sample::zip(&[
        ("_rels/.rels", sample::rels(&[("rId1", "officeDocument", "word/document.xml")]).as_bytes()),
        ("word/document.xml", format!(r#"<w:document {W}><w:body>{}</w:body></w:document>"#, paras(&["body"])).as_bytes()),
        ("word/_rels/document.xml.rels", doc_rels.as_bytes()),
        ("word/footnotes.xml", footnotes.as_bytes()),
    ]);
    assert_eq!(parts(read(&Word::default(), "ext.docx", &bytes)), [("".to_string(), "body\n".to_string())]);
}

#[test]
fn a_slides_fallback_content_is_left_out() {
    let alt = r#"<mc:AlternateContent><mc:Choice Requires="p14"><p:sp><p:txBody><a:p><a:r><a:t>chosen</a:t></a:r></a:p></p:txBody></p:sp></mc:Choice><mc:Fallback><p:sp><p:txBody><a:p><a:r><a:t>fallback</a:t></a:r></a:p></p:txBody></p:sp></mc:Fallback></mc:AlternateContent>"#;
    assert_eq!(parts(read(&PowerPoint::default(), "alt.pptx", &sample::pptx(&[(alt, None)]))), [("slide 1".to_string(), "chosen\n".to_string())]);
}

#[test]
fn word_hyphens_and_symbols_come_through_as_shown() {
    let body = concat!(
        r#"<w:p><w:r><w:t>co</w:t></w:r><w:r><w:softHyphen/></w:r><w:r><w:t>operate</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>well</w:t></w:r><w:r><w:noBreakHyphen/></w:r><w:r><w:t>known</w:t></w:r></w:p>"#,
        r#"<w:p><w:r><w:t>done </w:t></w:r><w:r><w:sym w:font="Segoe UI Symbol" w:char="2713"/></w:r><w:r><w:sym w:font="Wingdings" w:char="F04A"/></w:r></w:p>"#,
    );
    assert_eq!(parts(read(&Word::default(), "h.docx", &sample::docx(body))), [("".to_string(), "cooperate\nwell-known\ndone ✓\n".to_string())]);
}

#[test]
fn the_time_limit_is_asked_between_sheets_slides_and_parts() {
    // Parts with no text (writing text asks the sink itself), far from the
    // XML loop's every-256-events check, with the time already up.
    let past = |reader: &dyn Reader, name: &str, bytes: &[u8]| read_at(reader, name, bytes, Instant::now());
    let xlsx = sample::xlsx(&[("A", ""), ("B", "")], &[], "", false);
    assert!(matches!(past(&Excel::default(), "t.xlsx", &xlsx), Err(ReadError::Timeout)));
    let pptx = sample::pptx(&[("", None), ("", None)]);
    assert!(matches!(past(&PowerPoint::default(), "t.pptx", &pptx), Err(ReadError::Timeout)));
    let header = format!(r#"<w:hdr {W}/>"#);
    let docx = sample::docx_with("", &[("word/header1.xml", header)], &[("rId7", "header", "header1.xml")]);
    assert!(matches!(past(&Word::default(), "t.docx", &docx), Err(ReadError::Timeout)));
    // With time to spare, they read.
    assert!(read(&Excel::default(), "t.xlsx", &xlsx).is_ok());
    assert!(read(&PowerPoint::default(), "t.pptx", &pptx).is_ok());
    assert!(read(&Word::default(), "t.docx", &docx).is_ok());
}

#[test]
fn sheets_and_slides_are_found_among_many_relationships_at_once() {
    // 100,000 listed (the cap raised for the test) against 100,000
    // relationships, none matching: looked up one by one that is 10
    // billion comparisons, far past the reader's 10 s; at once, about a
    // second in a debug build.
    let n = 100_000;
    let caps = Caps { listed: n, ..Caps::default() };
    let rels: String =
        (0..n).map(|j| format!(r#"<Relationship Id="y{j}" Type="t/worksheet" Target="s.xml"/>"#)).collect();
    let rels = format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#);
    let sheets: String = (0..n).map(|i| format!(r#"<sheet name="s{i}" r:id="x{i}"/>"#)).collect();
    let wb = format!(r#"<workbook {}><sheets>{sheets}</sheets></workbook>"#, sample::S);
    let xlsx = sample::zip(&[
        ("_rels/.rels", sample::rels(&[("rId1", "officeDocument", "xl/workbook.xml")]).as_bytes()),
        ("xl/workbook.xml", wb.as_bytes()),
        ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
    ]);
    let slides: String = (0..n).map(|i| format!(r#"<p:sldId r:id="x{i}"/>"#)).collect();
    let pres = format!(r#"<p:presentation {}><p:sldIdLst>{slides}</p:sldIdLst></p:presentation>"#, sample::P);
    let pptx = sample::zip(&[
        ("_rels/.rels", sample::rels(&[("rId1", "officeDocument", "ppt/presentation.xml")]).as_bytes()),
        ("ppt/presentation.xml", pres.as_bytes()),
        ("ppt/_rels/presentation.xml.rels", rels.as_bytes()),
    ]);
    for (reader, name, bytes) in [(&Excel { caps } as &dyn Reader, "many.xlsx", &xlsx), (&PowerPoint { caps }, "many.pptx", &pptx)] {
        let got = read(reader, name, bytes);
        assert!(got.is_ok(), "{name}: {:?}", got.map(|_| ()));
    }
}

// --- From the second review ---

#[test]
fn one_xml_event_past_its_cap_stops_reading_and_keeps_what_was_read() {
    let caps = Caps { event: 1000, ..Caps::default() };
    let mut seed = 21;
    let text_of = |l: &str| paras(&[l]);
    let comment = |l: &str| format!("<!--{l}-->");
    let tag = |l: &str| format!(r#"<w:p w:x="{l}"/>"#);
    type Shape<'a> = (&'a str, &'a dyn Fn(&str) -> String);
    let shapes: [Shape; 3] = [("text", &text_of), ("comment", &comment), ("tag", &tag)];
    for (what, middle) in shapes {
        for (n, cut) in [(900, false), (1100, true)] {
            let body = format!("{}{}{}", paras(&["kept"]), middle(&letters(n, &mut seed)), paras(&["later"]));
            let (got, facts) = read(&Word { caps }, "e.docx", &sample::docx(&body)).unwrap();
            let text: String = got.iter().map(|p| p.text.as_str()).collect();
            assert!(text.starts_with("kept\n"), "{what} {n}");
            assert_eq!(text.ends_with("later\n"), !cut, "{what} {n}: {text:?}");
            assert_eq!(fact(&facts, "truncated").is_some(), cut, "{what} {n}");
        }
    }
    // At the default cap, an 80 MB text node is never held: reading stops
    // at 4 MB of it.
    let mut doc = String::from("<w:p><w:r><w:t>");
    for _ in 0..80 {
        doc.push_str(&letters(20_000, &mut seed));
        doc.push_str(&"x".repeat(MB as usize - 20_001));
        doc.push('\r');
    }
    doc.push_str("</w:t></w:r></w:p>");
    let bytes = sample::docx(&format!("{}{doc}", paras(&["kept"])));
    let (got, held) = sample::held::peak(|| read(&Word::default(), "big.docx", &bytes));
    let (parts, facts) = got.unwrap();
    assert_eq!(parts[0].text, "kept\n");
    assert_eq!(fact(&facts, "truncated"), Some("1"));
    assert!(held < 32 * MB as usize, "held {} MB", held / MB as usize);
}

#[test]
fn a_long_attribute_reads_as_absent() {
    // A header's relationship with a target past 4 KB is not followed.
    let header = format!(r#"<w:hdr {W}>{}</w:hdr>"#, paras(&["head"]));
    for (target, found) in [("header1.xml".to_string(), true), (format!("{}header1.xml", "./".repeat(2100)), false)] {
        let docx = sample::docx_with(&paras(&["body"]), &[("word/header1.xml", header.clone())], &[("rId7", "header", &target)]);
        let got = parts(read(&Word::default(), "a.docx", &docx));
        assert_eq!(got.iter().any(|(_, t)| t == "head\n"), found, "{} bytes", target.len());
    }
}

#[test]
fn past_the_cap_on_relationships_the_rest_are_left_out_and_recorded() {
    let header = format!(r#"<w:hdr {W}>{}</w:hdr>"#, paras(&["head"]));
    for (before, found) in [(49_999, true), (50_000, false)] {
        let mut rels: Vec<(String, &str, &str)> = (0..before).map(|i| (format!("i{i}"), "image", "media/x.png")).collect();
        rels.push(("rH".to_string(), "header", "header1.xml"));
        let rels: Vec<(&str, &str, &str)> = rels.iter().map(|(a, b, c)| (a.as_str(), *b, *c)).collect();
        let docx = sample::docx_with(&paras(&["body"]), &[("word/header1.xml", header.clone())], &rels);
        let (got, facts) = read(&Word::default(), "r.docx", &docx).unwrap();
        assert_eq!(got.iter().any(|p| p.text == "head\n"), found, "{before} before it");
        assert_eq!(fact(&facts, "truncated").is_some(), !found);
    }
}

#[test]
fn blank_pages_cost_nothing_and_parts_are_capped() {
    // A thousand typed breaks: two parts, labelled where the text is.
    let body = format!("{}<w:p><w:r>{}</w:r></w:p>{}", paras(&["start"]), r#"<w:br w:type="page"/>"#.repeat(1000), paras(&["end"]));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blank.docx");
    std::fs::write(&path, sample::docx(&body)).unwrap();
    let mut sink = Sink::new(1_000_000, Instant::now() + Duration::from_secs(10));
    Word::default().read(&path, &mut sink).unwrap();
    assert_eq!(sink.parts.len(), 2, "blank pages kept no part");
    let (got, _) = sink.finish();
    let labels: Vec<&str> = got.iter().map(|p| p.label.as_str()).collect();
    assert_eq!(labels, ["page 1", "page 1001"]);
    // One part past the cap, every page with text: the last is left out.
    let max = crate::extract::MAX_PARTS;
    for (pages, cut) in [(max, false), (max + 1, true)] {
        let body: String = (0..pages).map(|i| format!(r#"<w:p><w:r><w:t>p{i}</w:t><w:br w:type="page"/></w:r></w:p>"#)).collect();
        let (got, facts) = read(&Word::default(), "many.docx", &sample::docx(&body)).unwrap();
        assert_eq!(got.len(), max, "{pages} pages");
        assert_eq!(fact(&facts, "truncated").is_some(), cut, "{pages} pages");
    }
}

#[test]
fn styles_past_excels_own_limit_are_left_out() {
    // 65,536 styles are kept; a cell naming one past them shows its number.
    let mut seed = 31;
    let xfs = |n: usize, seed: &mut u64| -> String {
        (0..n).map(|_| format!(r#"<xf numFmtId="0" x="{}"/>"#, letters(6, seed))).collect()
    };
    for (index, date) in [(65_535, true), (65_536, false)] {
        let styles = format!(r#"<cellXfs>{}<xf numFmtId="14"/></cellXfs>"#, xfs(index, &mut seed));
        let sheet = format!(r#"<row><c s="{index}"><v>46329</v></c></row>"#);
        let (got, _) = read(&Excel::default(), "st.xlsx", &sample::xlsx(&[("S", &sheet)], &[], &styles, false)).unwrap();
        assert_eq!(got[0].text, if date { "2026-11-03\n" } else { "46329\n" }, "style {index}");
    }
    // Custom formats past the same limit are left out too.
    for (count, date) in [(65_535, true), (65_536, false)] {
        let mut formats: String = (0..count).map(|i| format!(r#"<numFmt numFmtId="{}" formatCode="0.{}"/>"#, 1000 + i, letters(3, &mut seed))).collect();
        formats.push_str(r#"<numFmt numFmtId="164" formatCode="yyyy-mm-dd"/>"#);
        let styles = format!(r#"<numFmts>{formats}</numFmts><cellXfs><xf numFmtId="164"/></cellXfs>"#);
        let sheet = r#"<row><c s="0"><v>46329</v></c></row>"#;
        let (got, _) = read(&Excel::default(), "nf.xlsx", &sample::xlsx(&[("S", sheet)], &[], &styles, false)).unwrap();
        assert_eq!(got[0].text, if date { "2026-11-03\n" } else { "46329\n" }, "{count} formats before it");
    }
}

// --- From the third review ---

#[test]
fn nesting_past_its_cap_stops_reading_and_keeps_what_was_read() {
    for (depth, cut) in [(10_000, false), (10_001, true)] {
        // The document, body and paragraph are three levels already.
        let body = format!("{}<w:p>{}{}</w:p>{}", paras(&["kept"]), "<w:x>".repeat(depth - 3), "</w:x>".repeat(depth - 3), paras(&["later"]));
        let (got, facts) = read(&Word::default(), "deep.docx", &sample::docx(&body)).unwrap();
        let text: String = got.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(text, if cut { "kept\n" } else { "kept\nlater\n" }, "{depth}");
        assert_eq!(fact(&facts, "truncated").is_some(), cut, "{depth}");
    }
}

#[test]
fn a_property_keeps_its_first_value_and_facts_are_capped() {
    let core = r#"<cp:coreProperties xmlns:cp="c" xmlns:dc="d"><dc:title>First</dc:title><dc:title>Second</dc:title></cp:coreProperties>"#;
    let bytes = sample::zip(&[
        ("_rels/.rels", sample::rels(&[("rId1", "officeDocument", "word/document.xml"), ("rId2", "core-properties", "docProps/core.xml")]).as_bytes()),
        ("docProps/core.xml", core.as_bytes()),
        ("word/document.xml", format!(r#"<w:document {W}><w:body>{}</w:body></w:document>"#, paras(&["x"])).as_bytes()),
    ]);
    let (_, facts) = read(&Word::default(), "p.docx", &bytes).unwrap();
    let titles: Vec<&str> = facts.iter().filter(|(k, _)| k == "title").map(|(_, v)| v.as_str()).collect();
    assert_eq!(titles, ["First"]);
    let max = crate::extract::MAX_FACTS;
    let mut sink = Sink::new(10, Instant::now() + Duration::from_secs(10));
    for i in 0..max + 1 {
        sink.fact("k", format!("{i}"));
    }
    let (_, facts) = sink.finish();
    assert_eq!(facts.len(), max);
    assert_eq!(facts.last().unwrap().1, format!("{}", max - 1));
}

// --- From the fourth review ---

#[test]
fn a_relationship_resolving_past_4_kb_is_left_out() {
    // The main document sits in a 3,000-byte folder; its header's target,
    // joined to that folder, comes to 4,096 bytes or one more.
    let folder = "f".repeat(3000);
    for (name_len, found) in [(4096 - 3001, true), (4097 - 3001, false)] {
        let header_name = format!("{}.xml", "h".repeat(name_len - 4));
        let header_path = format!("{folder}/{header_name}");
        let header = format!(r#"<w:hdr {W}>{}</w:hdr>"#, paras(&["head"]));
        let document = format!(r#"<w:document {W}><w:body>{}</w:body></w:document>"#, paras(&["body"]));
        let root = sample::rels(&[("rId1", "officeDocument", &format!("{folder}/document.xml"))]);
        let doc_rels = sample::rels(&[("rH", "header", &header_name)]);
        let bytes = sample::zip(&[
            ("_rels/.rels", root.as_bytes()),
            (&format!("{folder}/document.xml"), document.as_bytes()),
            (&format!("{folder}/_rels/document.xml.rels"), doc_rels.as_bytes()),
            (&header_path, header.as_bytes()),
        ]);
        let got = parts(read(&Word::default(), "deep.docx", &bytes));
        assert_eq!(got.iter().any(|(_, t)| t == "head\n"), found, "{} bytes", header_path.len());
    }
}

#[test]
fn an_element_name_past_256_bytes_stops_reading() {
    for (len, cut) in [(256, false), (257, true)] {
        let name = format!("w:{}", "x".repeat(len - 2));
        let body = format!("{}<{name}/>{}", paras(&["kept"]), paras(&["later"]));
        let (got, facts) = read(&Word::default(), "n.docx", &sample::docx(&body)).unwrap();
        let text: String = got.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(text, if cut { "kept\n" } else { "kept\nlater\n" }, "{len}");
        assert_eq!(fact(&facts, "truncated").is_some(), cut, "{len}");
    }
}

// --- From the fifth review ---

#[test]
fn a_zip_declaring_more_parts_than_the_cap_is_skipped_before_it_is_read() {
    let caps = Caps { entries: 3, ..Caps::default() };
    // A real zip of 3 parts, then the same with its end record's count
    // raised to 4: refused by what it declares, not what it holds.
    let bytes = sample::docx(&paras(&["x"]));
    let mut few = sample::zip(&[("a", b"1"), ("b", b"2"), ("c", b"3")]);
    assert!(!matches!(read(&Word { caps }, "z.docx", &few), Err(ReadError::Unsupported(ref why)) if why.starts_with("more than")));
    let end = few.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
    few[end + 8..end + 12].copy_from_slice(&[4, 0, 4, 0]);
    assert_eq!(skipped(read(&Word { caps }, "z.docx", &few)), "more than 3 parts");
    // A zip64 end record declaring a million entries, ahead of the plain
    // one: refused the same way.
    let mut big = bytes.clone();
    let end = big.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
    let plain = big.split_off(end);
    let offset = big.len() as u64;
    let mut end64 = b"PK\x06\x06".to_vec();
    end64.extend_from_slice(&44u64.to_le_bytes()); // size of the rest
    end64.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    end64.extend_from_slice(&1_000_000u64.to_le_bytes()); // entries here
    end64.extend_from_slice(&1_000_000u64.to_le_bytes()); // entries in all
    end64.extend_from_slice(&[0; 16]); // directory size and offset
    let mut locator = b"PK\x06\x07".to_vec();
    locator.extend_from_slice(&0u32.to_le_bytes());
    locator.extend_from_slice(&offset.to_le_bytes());
    locator.extend_from_slice(&1u32.to_le_bytes());
    big.extend_from_slice(&end64);
    big.extend_from_slice(&locator);
    big.extend_from_slice(&plain);
    assert_eq!(skipped(read(&Word::default(), "z64.docx", &big)), "more than 10000 parts");
}

#[test]
fn shared_strings_are_kept_trimmed_and_the_check_interval_is_short() {
    // A blank string reads as empty, a padded one as its text: as a cell
    // shows them, stored so (a cell naming a long blank string costs
    // nothing).
    let sheet = r#"<row><c t="s"><v>0</v></c><c t="s"><v>1</v></c></row>"#;
    let bytes = sample::xlsx(&[("S", sheet)], &[&format!("<t>{}</t>", " ".repeat(1000)), "<t>  padded  </t>"], "", false);
    assert_eq!(parts(read(&Excel::default(), "t.xlsx", &bytes)), [("S".to_string(), "padded\n".to_string())]);
    assert_eq!(super::CHECK_EVERY, 256);
}
