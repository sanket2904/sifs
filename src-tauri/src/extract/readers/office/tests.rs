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
    assert_eq!(got.len(), 0, "the row in progress is not written: {got:?}");
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
