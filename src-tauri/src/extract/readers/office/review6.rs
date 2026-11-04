//! The step 2 reviewer's sixth-round tests, kept (see "Reviews" in plans/phase3-reading-inside-files.md). Its two
//! release-only timing measurements were left out (the review copy in the
//! git history before "Remove the handover folder" has them).
use super::sample;
use super::*;
use crate::extract::{Part, Reader};
use std::io::Write as _;
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

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn crc(data: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(data);
    c.sum()
}

/// A zip written by hand: (name, method, data as stored, crc, uncompressed size).
fn raw_zip(entries: &[(String, u16, Vec<u8>, u32, u64)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, method, data, crc, size) in entries {
        let offset = out.len() as u32;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0x21u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(*size as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0x21u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(*size as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// `blocks` empty fixed-Huffman deflate blocks (10 bits each: BFINAL 0,
/// BTYPE 01, end-of-block 0000000), a multiple of 4 so they end on a byte.
fn empty_fixed_blocks(blocks: usize) -> Vec<u8> {
    assert_eq!(blocks % 4, 0);
    // Four blocks: bit 1, 11, 21, 31 set (LSB first) = 0x02, 0x08, 0x20, 0x80, 0x00.
    let mut v = Vec::with_capacity(blocks / 4 * 5);
    for _ in 0..blocks / 4 {
        v.extend_from_slice(&[0x02, 0x08, 0x20, 0x80, 0x00]);
    }
    v
}

/// A real Word document whose package relationships (_rels/.rels, the
/// first part read) are deflated behind `pad` bytes of empty blocks: they
/// unpack to nothing, so no cap on unpacking counts them.
fn docx_with_empty_blocks(pad: usize) -> Vec<u8> {
    let plain = sample::docx(&sample::paras(&["hello"]));
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(plain)).unwrap();
    let mut entries = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let name = f.name().unwrap().to_string();
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut data).unwrap();
        let mut stored = if name == "_rels/.rels" { empty_fixed_blocks(pad / 5 * 4) } else { Vec::new() };
        stored.extend_from_slice(&deflate(&data));
        entries.push((name, 8u16, stored, crc(&data), data.len() as u64));
    }
    raw_zip(&entries)
}

/// The zip64 records of round 5's test (a file of zeros claiming len/47
/// entries), with two ways round the new pre-check.
fn zip64_claiming(len: u64, locator_off_by_one: bool, stray_end_after: bool, trailing: usize) -> Vec<u8> {
    let n = len / 47;
    let mut f = vec![0u8; len as usize];
    f[..4].copy_from_slice(b"PK\x03\x04");
    let eocd64_at = len;
    f.extend_from_slice(&0x0606_4b50u32.to_le_bytes());
    f.extend_from_slice(&44u64.to_le_bytes());
    f.extend_from_slice(&45u16.to_le_bytes());
    f.extend_from_slice(&45u16.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&n.to_le_bytes());
    f.extend_from_slice(&n.to_le_bytes());
    f.extend_from_slice(&(n * 46).to_le_bytes());
    f.extend_from_slice(&n.to_le_bytes());
    // Locator. Off by one, it points at a zero byte just before the zip64
    // record: declared_entries finds no record there and ignores it; the
    // zip crate scans forward from there and finds it.
    f.extend_from_slice(&0x0706_4b50u32.to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&(eocd64_at - u64::from(locator_off_by_one)).to_le_bytes());
    f.extend_from_slice(&1u32.to_le_bytes());
    // End record. With the locator off by one its counts are 0 (only the
    // offset defers to zip64), so it declares nothing itself.
    let count: u16 = if locator_off_by_one { 0 } else { 0xFFFF };
    f.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    f.extend_from_slice(&0u16.to_le_bytes());
    f.extend_from_slice(&0u16.to_le_bytes());
    f.extend_from_slice(&count.to_le_bytes());
    f.extend_from_slice(&count.to_le_bytes());
    f.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    f.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    f.extend_from_slice(&0u16.to_le_bytes());
    if trailing > 0 {
        // Bytes after the end record (allowed: "garbage after comment"):
        // past 65,535 of them, declared_entries' 22 + 65,535-byte tail no
        // longer holds the record, while the zip crate's window (22 + 4 +
        // 65,536 bytes) still does.
        f.extend_from_slice(&vec![0u8; trailing]);
    }
    if stray_end_after {
        // A second end record after the real one, declaring 0 entries and
        // a comment running past the file: declared_entries reads only this
        // last one; the zip crate fails to parse it and falls back to the
        // real one.
        f.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        f.extend_from_slice(&[0; 16]);
        f.extend_from_slice(&0xFFFFu16.to_le_bytes());
    }
    f
}

fn zip64_peak(bytes: Vec<u8>) -> (String, usize) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("z.docx");
    std::fs::write(&path, &bytes).unwrap();
    drop(bytes);
    let (r, peak) = sample::held::peak(|| {
        let mut sink = Sink::new(1_000_000, Instant::now() + Duration::from_secs(10));
        Word::default().read(&path, &mut sink).map_err(|e| format!("{e:?}"))
    });
    (format!("{r:?}"), peak)
}

/// FINDING 2 (minor). declared_entries reads one end record, the last, and
/// only the zip64 record the locator points at exactly; the zip crate tries
/// earlier end records when the last fails, and scans for the zip64 record.
#[test]
fn r6_declared_entries_can_be_got_round() {
    let mut over = Vec::new();
    for (name, off, stray, trailing) in [
        ("plain (round 5's file)", false, false, 0),
        ("locator off by one", true, false, 0),
        ("stray end record after", false, true, 0),
        ("end record 65,560 bytes from the end", false, false, 65_538),
    ] {
        let (r, peak) = zip64_peak(zip64_claiming(99 * MB, off, stray, trailing));
        eprintln!("declared entries, 99 MB, {name}: peak={} MiB result={r}", peak / MB as usize);
        if peak >= 256 * MB as usize {
            over.push(name);
        }
    }
    assert!(over.is_empty(), "reserved over 256 MiB: {over:?}");
}

/// A zip whose entries, under every name given, are one and the same
/// stored data (central records may point at one local header; the zip
/// crate checks overlaps only when asked).
fn shared_zip(names: &[&str], data: &[u8], crc: u32, size: u64) -> Vec<u8> {
    let one = raw_zip(&[(names[0].to_string(), 8, data.to_vec(), crc, size)]);
    let cd_at = one.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    let record_end = one.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
    let mut out = one[..cd_at].to_vec();
    let mut central = Vec::new();
    for name in names {
        let mut rec = one[cd_at..cd_at + 46].to_vec();
        rec[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
        rec.extend_from_slice(name.as_bytes());
        central.extend_from_slice(&rec);
    }
    let _ = record_end;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(names.len() as u16).to_le_bytes());
    out.extend_from_slice(&(names.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&(cd_at as u32).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// A workbook whose six parts read before the first sheet (package
/// relationships, core properties, workbook, its relationships, shared
/// strings, styles) are one XML text, deflated behind `pad` bytes of empty
/// blocks, stored once.
fn xlsx_six_stalls(pad: usize) -> Vec<u8> {
    let xml = r#"<workbook><Relationship Id="r1" Type="a/officeDocument" Target="/xl/workbook.xml"/><Relationship Id="r2" Type="a/sharedStrings" Target="/xl/sharedStrings.xml"/><Relationship Id="r3" Type="a/styles" Target="/xl/styles.xml"/><Relationship Id="r4" Type="a/core-properties" Target="/docProps/core.xml"/></workbook>"#;
    let mut data = empty_fixed_blocks(pad / 5 * 4);
    data.extend_from_slice(&deflate(xml.as_bytes()));
    let names = ["_rels/.rels", "docProps/core.xml", "xl/workbook.xml", "xl/_rels/workbook.xml.rels", "xl/sharedStrings.xml", "xl/styles.xml"];
    shared_zip(&names, &data, crc(xml.as_bytes()), xml.len() as u64)
}

/// FINDING 1, as a test that fails without a fix and is quick with one: the time is
/// already up, and a 256 KB file's six parts each unpack 256 KB of empty
/// blocks before the sink is first asked.
#[test]
fn r6_empty_blocks_ask_the_sink() {
    let bytes = xlsx_six_stalls(256 * 1024);
    let ok = read_at(&Excel::default(), "s.xlsx", &bytes, Instant::now() + Duration::from_secs(600));
    eprintln!("six stalls: the workbook reads: {:?}", ok.as_ref().map(|o| o.1.clone()).map_err(|e| format!("{e:?}")));
    let start = Instant::now();
    let r = read_at(&Excel::default(), "s.xlsx", &bytes, Instant::now());
    let elapsed = start.elapsed();
    eprintln!("six stalls: file={} bytes, time already up: elapsed={elapsed:?} result={:?}", bytes.len(), r.as_ref().map(|o| o.0.len()).map_err(|e| format!("{e:?}")));
    assert!(elapsed < Duration::from_secs(1), "the sink was first asked after {elapsed:?}");
}

/// Not the reviewer's: a deadline that passes while empty blocks are being
/// decoded stops the read within a moment of it (a 10 MB file takes many
/// seconds to decode whole in a debug build).
#[test]
fn empty_blocks_stop_near_the_deadline() {
    let bytes = docx_with_empty_blocks(10 * MB as usize);
    let start = Instant::now();
    let got = read_at(&Word::default(), "e.docx", &bytes, start + Duration::from_millis(300));
    let took = start.elapsed();
    assert!(matches!(got, Err(ReadError::Timeout)), "{:?}", got.map(|o| o.0.len()));
    assert!(took < Duration::from_secs(5), "stopped {took:?} after starting, the deadline 300 ms");
}
