//! PDFs for the tests, written byte by byte. Text is shown in a font with
//! no glyphs but a ToUnicode map, which is all PDFKit needs to read it, so
//! any Unicode text can go on a page.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AllocAnyThread;
use objc2_foundation::{NSDictionary, NSString, NSURL};
use objc2_pdf_kit::{PDFDocument, PDFDocumentOwnerPasswordOption, PDFDocumentUserPasswordOption};
use std::fmt::Write;
use std::path::Path;

pub(crate) enum Page<'a> {
    Text(&'a str),
    /// ASCII text in Helvetica: PDFKit keeps it when it saves the file
    /// again (encrypt), which it doesn't for the font without glyphs.
    Helvetica(&'a str),
    Image,
    Blank,
    /// Form XObjects ten deep, each drawing the next ten times, ending in
    /// nothing: PDFKit works through 10^9 empty forms, using no memory.
    Hang,
    /// The same, ending in text: PDFKit keeps every character it draws,
    /// gigabytes a minute.
    Hog,
}

const CATALOG: usize = 1;
const PAGES: usize = 2;
const FONT: usize = 3;
const CID_FONT: usize = 4;
const DESCRIPTOR: usize = 5;
const TO_UNICODE: usize = 6;
const IMAGE: usize = 7;
const HELVETICA: usize = HOG + DEPTH;
const DEPTH: usize = 10;
const HANG: usize = 8;
const HOG: usize = HANG + DEPTH;
const FIRST_PAGE: usize = HELVETICA + 1;

pub(crate) fn pdf(pages: &[Page]) -> Vec<u8> {
    pdf_with(pages, "")
}

pub(crate) fn pdf_with(pages: &[Page], trailer: &str) -> Vec<u8> {
    let mut chars: Vec<char> = vec!['x'];
    for page in pages {
        if let Page::Text(text) = page {
            for c in text.chars() {
                if !chars.contains(&c) {
                    chars.push(c);
                }
            }
        }
    }
    let show = |text: &str| {
        let hex: String =
            text.chars().map(|c| format!("{:04X}", chars.iter().position(|&k| k == c).unwrap() + 1)).collect();
        format!("BT /F1 12 Tf 72 720 Td <{hex}> Tj ET")
    };
    let mut objs: Vec<Vec<u8>> = vec![Vec::new(); FIRST_PAGE - 1 + 2 * pages.len()];
    let mut set = |n: usize, body: Vec<u8>| objs[n - 1] = body;
    set(CATALOG, format!("<< /Type /Catalog /Pages {PAGES} 0 R >>").into());
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", FIRST_PAGE + 2 * i)).collect();
    set(PAGES, format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into());
    set(
        FONT,
        format!("<< /Type /Font /Subtype /Type0 /BaseFont /T /Encoding /Identity-H /DescendantFonts [{CID_FONT} 0 R] /ToUnicode {TO_UNICODE} 0 R >>")
            .into(),
    );
    set(
        CID_FONT,
        format!("<< /Type /Font /Subtype /CIDFontType2 /BaseFont /T /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {DESCRIPTOR} 0 R /DW 600 >>")
            .into(),
    );
    set(
        DESCRIPTOR,
        b"<< /Type /FontDescriptor /FontName /T /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >>".to_vec(),
    );
    let mut cmap = String::from("/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange\n");
    for (block, group) in chars.chunks(100).enumerate() {
        writeln!(cmap, "{} beginbfchar", group.len()).unwrap();
        for (i, c) in group.iter().enumerate() {
            let utf16: String = c.encode_utf16(&mut [0; 2]).iter().map(|u| format!("{u:04X}")).collect();
            writeln!(cmap, "<{:04X}> <{utf16}>", block * 100 + i + 1).unwrap();
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap CMapName currentdict /CMap defineresource pop end end");
    set(TO_UNICODE, stream(cmap.as_bytes(), ""));
    let pixels: Vec<u8> = (0..64).map(|i| if i % 2 == 0 { 0 } else { 255 }).collect();
    set(HELVETICA, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    set(IMAGE, stream(&pixels, "/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceGray /BitsPerComponent 8"));
    for (first, leaf) in [(HANG, "q Q".to_string()), (HOG, show("x"))] {
        for level in 0..DEPTH {
            let n = first + level;
            let form = "/Type /XObject /Subtype /Form /BBox [0 0 612 792]";
            set(
                n,
                if level + 1 < DEPTH {
                    stream("/X Do ".repeat(10).as_bytes(), &format!("{form} /Resources << /XObject << /X {} 0 R >> >>", n + 1))
                } else {
                    stream(leaf.as_bytes(), &format!("{form} /Resources << /Font << /F1 {FONT} 0 R >> >>"))
                },
            );
        }
    }
    for (i, page) in pages.iter().enumerate() {
        let n = FIRST_PAGE + 2 * i;
        let content = match page {
            Page::Text(text) => show(text),
            Page::Helvetica(text) => format!("BT /F2 12 Tf 72 720 Td ({text}) Tj ET"),
            Page::Image => "q 200 0 0 200 72 400 cm /I Do Q".to_string(),
            Page::Blank => String::new(),
            Page::Hang | Page::Hog => "/X Do".to_string(),
        };
        let form = match page {
            Page::Hog => HOG,
            _ => HANG,
        };
        set(
            n,
            format!("<< /Type /Page /Parent {PAGES} 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 {FONT} 0 R /F2 {HELVETICA} 0 R >> /XObject << /I {IMAGE} 0 R /X {form} 0 R >> >> /Contents {} 0 R >>", n + 1)
                .into(),
        );
        set(n + 1, stream(content.as_bytes(), ""));
    }
    let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1);
    for offset in offsets {
        writeln!(table, "{offset:010} 00000 n ").unwrap();
    }
    write!(table, "trailer\n<< /Size {} /Root {CATALOG} 0 R {trailer} >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).unwrap();
    out.extend_from_slice(table.as_bytes());
    out
}

fn stream(data: &[u8], dict: &str) -> Vec<u8> {
    let mut out = format!("<< /Length {} {dict} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// Saves the PDF at from to to, encrypted by PDFKit: with a password to
/// open it (user), and one to change it (owner).
pub(crate) fn encrypt(from: &Path, to: &Path, user: Option<&str>, owner: &str) {
    let url = |p: &Path| NSURL::fileURLWithPath(&NSString::from_str(p.to_str().unwrap()));
    let doc = unsafe { PDFDocument::initWithURL(PDFDocument::alloc(), &url(from)) }.expect("a PDF to encrypt");
    let owner = NSString::from_str(owner);
    let user = user.map(NSString::from_str);
    let mut keys: Vec<&NSString> = vec![unsafe { PDFDocumentOwnerPasswordOption }];
    let mut values: Vec<&AnyObject> = vec![&owner];
    if let Some(user) = &user {
        keys.push(unsafe { PDFDocumentUserPasswordOption });
        values.push(user);
    }
    let options: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(&keys, &values);
    assert!(unsafe { doc.writeToURL_withOptions(&url(to), Some(&options)) }, "PDFKit saved the encrypted PDF");
}
