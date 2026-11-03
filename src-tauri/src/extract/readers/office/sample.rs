//! Office files built for tests, from minimal XML, with the zip crate: no
//! binary samples are kept.

use std::io::Write;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

pub(crate) const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#;
pub(crate) const S: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
pub(crate) const P: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A zip of these parts, deflated.
pub(crate) fn zip(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, body) in parts {
        zip.start_file(*name, options).unwrap();
        zip.write_all(body).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// A .rels part: (id, type's last segment, target).
pub(crate) fn rels(rels: &[(&str, &str, &str)]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (id, kind, target) in rels {
        let base = if *kind == "core-properties" {
            "http://schemas.openxmlformats.org/package/2006/relationships/metadata"
        } else {
            REL
        };
        xml.push_str(&format!(r#"<Relationship Id="{id}" Type="{base}/{kind}" Target="{target}"/>"#));
    }
    xml.push_str("</Relationships>");
    xml
}

pub(crate) fn core(title: &str, author: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>{title}</dc:title><dc:creator>{author}</dc:creator><dcterms:created xsi:type="dcterms:W3CDTF">2026-11-01T09:15:00Z</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">2026-11-03T10:00:00Z</dcterms:modified></cp:coreProperties>"#
    )
}

/// A Word document whose body is body (the insides of w:body), with extra
/// parts and the document's own relationships.
pub(crate) fn docx_with(body: &str, extra: &[(&str, String)], doc_rels: &[(&str, &str, &str)]) -> Vec<u8> {
    let document = format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document {W}><w:body>{body}</w:body></w:document>"#);
    let package_rels = rels(&[
        ("rId1", "officeDocument", "word/document.xml"),
        ("rId2", "core-properties", "docProps/core.xml"),
        ("rId3", "extended-properties", "docProps/app.xml"),
    ]);
    let app = r#"<?xml version="1.0"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Pages>3</Pages></Properties>"#;
    let mut parts: Vec<(&str, Vec<u8>)> = vec![
        ("[Content_Types].xml", b"<Types/>".to_vec()),
        ("_rels/.rels", package_rels.into_bytes()),
        ("docProps/core.xml", core("Rent agreement", "Asha Rao").into_bytes()),
        ("docProps/app.xml", app.as_bytes().to_vec()),
        ("word/document.xml", document.into_bytes()),
        ("word/_rels/document.xml.rels", rels(doc_rels).into_bytes()),
    ];
    for (name, body) in extra {
        parts.push((name, body.as_bytes().to_vec()));
    }
    let parts: Vec<(&str, &[u8])> = parts.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    zip(&parts)
}

pub(crate) fn docx(body: &str) -> Vec<u8> {
    docx_with(body, &[], &[])
}

/// Paragraphs of plain text, for a w:body.
pub(crate) fn paras(lines: &[&str]) -> String {
    lines.iter().map(|l| format!("<w:p><w:r><w:t xml:space=\"preserve\">{l}</w:t></w:r></w:p>")).collect()
}

/// A workbook with these sheets: (name, sheetData insides). Sheets are
/// stored under names unlike their own, in reverse, so order comes from
/// the workbook.
pub(crate) fn xlsx(sheets: &[(&str, &str)], shared: &[&str], styles: &str, date1904: bool) -> Vec<u8> {
    let mut workbook = format!(r#"<?xml version="1.0"?><workbook {S}><workbookPr date1904="{}"/><sheets>"#, date1904 as u8);
    let mut wb_rels = vec![
        ("rIdS".to_string(), "sharedStrings".to_string(), "sharedStrings.xml".to_string()),
        ("rIdT".to_string(), "styles".to_string(), "styles.xml".to_string()),
    ];
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, (name, data)) in sheets.iter().enumerate() {
        let file = format!("worksheets/sheet{}.xml", sheets.len() - i);
        workbook.push_str(&format!(r#"<sheet name="{name}" sheetId="{}" r:id="rId{i}"/>"#, i + 1));
        wb_rels.push((format!("rId{i}"), "worksheet".into(), file.clone()));
        parts.push((
            format!("xl/{file}"),
            format!(r#"<?xml version="1.0"?><worksheet {S}><sheetData>{data}</sheetData></worksheet>"#).into_bytes(),
        ));
    }
    workbook.push_str("</sheets></workbook>");
    let sst: String = shared.iter().map(|s| format!("<si>{s}</si>")).collect();
    let wb_rels: Vec<(&str, &str, &str)> =
        wb_rels.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str())).collect();
    parts.extend([
        ("[Content_Types].xml".to_string(), b"<Types/>".to_vec()),
        (
            "_rels/.rels".to_string(),
            rels(&[("rId1", "officeDocument", "xl/workbook.xml"), ("rId2", "core-properties", "docProps/core.xml")])
                .into_bytes(),
        ),
        ("docProps/core.xml".to_string(), core("Budget", "Ravi").into_bytes()),
        ("xl/workbook.xml".to_string(), workbook.into_bytes()),
        ("xl/_rels/workbook.xml.rels".to_string(), rels(&wb_rels).into_bytes()),
        (
            "xl/sharedStrings.xml".to_string(),
            format!(r#"<?xml version="1.0"?><sst {S} count="{0}" uniqueCount="{0}">{sst}</sst>"#, shared.len())
                .into_bytes(),
        ),
        ("xl/styles.xml".to_string(), format!(r#"<?xml version="1.0"?><styleSheet {S}>{styles}</styleSheet>"#).into_bytes()),
    ]);
    let parts: Vec<(&str, &[u8])> = parts.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    zip(&parts)
}

/// A shape's text body of paragraphs, for a slide.
pub(crate) fn shape(paras: &[&str]) -> String {
    let ps: String = paras.iter().map(|p| format!("<a:p><a:r><a:t>{p}</a:t></a:r></a:p>")).collect();
    format!("<p:sp><p:txBody><a:bodyPr/>{ps}</p:txBody></p:sp>")
}

/// A presentation of these slides: (shapes, notes). Slides are stored
/// under names unlike their order.
pub(crate) fn pptx(slides: &[(&str, Option<&str>)]) -> Vec<u8> {
    let mut presentation = format!(r#"<?xml version="1.0"?><p:presentation {P}><p:sldIdLst>"#);
    let mut p_rels = Vec::new();
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, (shapes, notes)) in slides.iter().enumerate() {
        let n = slides.len() - i;
        presentation.push_str(&format!(r#"<p:sldId id="{}" r:id="rId{i}"/>"#, 256 + i));
        p_rels.push((format!("rId{i}"), "slide".to_string(), format!("slides/slide{n}.xml")));
        parts.push((
            format!("ppt/slides/slide{n}.xml"),
            format!(r#"<?xml version="1.0"?><p:sld {P}><p:cSld><p:spTree>{shapes}</p:spTree></p:cSld></p:sld>"#)
                .into_bytes(),
        ));
        let mut slide_rels = vec![("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml")];
        let notes_target = format!("../notesSlides/notesSlide{n}.xml");
        if let Some(notes) = notes {
            slide_rels.push(("rId2", "notesSlide", &notes_target));
            let body = format!(
                r#"<p:sp><p:txBody><a:p><a:fld id="{{1}}" type="slidenum"><a:t>{n}</a:t></a:fld></a:p></p:txBody></p:sp>{}"#,
                shape(&[notes])
            );
            parts.push((
                format!("ppt/notesSlides/notesSlide{n}.xml"),
                format!(r#"<?xml version="1.0"?><p:notes {P}><p:cSld><p:spTree>{body}</p:spTree></p:cSld></p:notes>"#)
                    .into_bytes(),
            ));
        }
        parts.push((format!("ppt/slides/_rels/slide{n}.xml.rels"), rels(&slide_rels).into_bytes()));
    }
    presentation.push_str("</p:sldIdLst></p:presentation>");
    let p_rels: Vec<(&str, &str, &str)> = p_rels.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str())).collect();
    parts.extend([
        ("[Content_Types].xml".to_string(), b"<Types/>".to_vec()),
        (
            "_rels/.rels".to_string(),
            rels(&[("rId1", "officeDocument", "ppt/presentation.xml"), ("rId2", "core-properties", "docProps/core.xml")])
                .into_bytes(),
        ),
        ("docProps/core.xml".to_string(), core("Pitch", "Mei").into_bytes()),
        ("ppt/presentation.xml".to_string(), presentation.into_bytes()),
        ("ppt/_rels/presentation.xml.rels".to_string(), rels(&p_rels).into_bytes()),
    ]);
    let parts: Vec<(&str, &[u8])> = parts.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    zip(&parts)
}

/// A compound file, as Office wraps a password-protected file, with these
/// streams in its directory (one sector of 512 bytes: up to three names).
pub(crate) fn compound(streams: &[&str]) -> Vec<u8> {
    let mut header = vec![0u8; 512];
    header[..8].copy_from_slice(&super::cfb::MAGIC);
    header[0x1A] = 0x3E; // minor and major version 3
    header[0x1C..0x1E].copy_from_slice(&0xFFFEu16.to_le_bytes());
    header[0x1E..0x20].copy_from_slice(&9u16.to_le_bytes()); // 512-byte sectors
    header[0x20..0x22].copy_from_slice(&6u16.to_le_bytes());
    header[0x2C..0x30].copy_from_slice(&1u32.to_le_bytes()); // one FAT sector
    header[0x30..0x34].copy_from_slice(&1u32.to_le_bytes()); // directory at sector 1
    for i in 0..109 {
        let v: u32 = if i == 0 { 0 } else { 0xFFFF_FFFF };
        header[0x4C + 4 * i..0x50 + 4 * i].copy_from_slice(&v.to_le_bytes());
    }
    // Sector 0, the FAT: itself, then the directory's one-sector chain.
    let mut fat = vec![0xFFu8; 512];
    fat[0..4].copy_from_slice(&0xFFFF_FFFDu32.to_le_bytes());
    fat[4..8].copy_from_slice(&0xFFFF_FFFEu32.to_le_bytes());
    // Sector 1, the directory.
    let mut dir = vec![0u8; 512];
    for (i, name) in std::iter::once("Root Entry").chain(streams.iter().copied()).take(4).enumerate() {
        let entry = &mut dir[i * 128..(i + 1) * 128];
        let units: Vec<u16> = name.encode_utf16().collect();
        for (j, u) in units.iter().enumerate() {
            entry[2 * j..2 * j + 2].copy_from_slice(&u.to_le_bytes());
        }
        entry[0x40..0x42].copy_from_slice(&((units.len() as u16 + 1) * 2).to_le_bytes());
        entry[0x42] = if i == 0 { 5 } else { 2 };
    }
    [header, fat, dir].concat()
}

/// Counts the bytes each thread has allocated and not yet freed, so a test
/// can see how much a read held at most, whatever other tests do at the
/// same time (unlike the process's peak memory).
pub(crate) mod held {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    struct Counting;

    thread_local! {
        static NOW: Cell<isize> = const { Cell::new(0) };
        static PEAK: Cell<isize> = const { Cell::new(0) };
    }

    fn add(n: usize) {
        let _ = NOW.try_with(|now| {
            let v = now.get() + n as isize;
            now.set(v);
            let _ = PEAK.try_with(|peak| peak.set(peak.get().max(v)));
        });
    }

    fn sub(n: usize) {
        let _ = NOW.try_with(|now| now.set(now.get() - n as isize));
    }

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let p = System.alloc(layout);
            if !p.is_null() {
                add(layout.size());
            }
            p
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let p = System.alloc_zeroed(layout);
            if !p.is_null() {
                add(layout.size());
            }
            p
        }
        unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
            System.dealloc(p, layout);
            sub(layout.size());
        }
        unsafe fn realloc(&self, p: *mut u8, layout: Layout, new: usize) -> *mut u8 {
            let q = System.realloc(p, layout, new);
            if !q.is_null() {
                add(new);
                sub(layout.size());
            }
            q
        }
    }

    #[global_allocator]
    static COUNTING: Counting = Counting;

    /// What f returns, and the most bytes this thread held during it, over
    /// what it held before.
    pub(crate) fn peak<T>(f: impl FnOnce() -> T) -> (T, usize) {
        let start = NOW.with(Cell::get);
        PEAK.with(|p| p.set(start));
        let out = f();
        (out, (PEAK.with(Cell::get) - start).max(0) as usize)
    }
}
