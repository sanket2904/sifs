//! Word, Excel and PowerPoint files (Office Open XML): zips of XML parts.
//! Only the parts with text are unpacked, each through a counter, so a zip
//! bomb is caught by what it unpacks to, not by the sizes it declares. The
//! XML is read with quick-xml, which never reads a DTD, so no entity a file
//! declares is ever expanded.
//!
//! A password-protected Office file is not a zip but an OLE compound file
//! (cfb.rs); it is skipped with that reason.

mod cfb;
mod excel;
mod powerpoint;
mod word;

#[cfg(test)]
pub(crate) mod sample;
#[cfg(test)]
mod review;
#[cfg(test)]
mod review2;
#[cfg(test)]
mod review3;
#[cfg(test)]
mod review4;
#[cfg(test)]
mod review5;
#[cfg(test)]
mod tests;

pub use excel::Excel;
pub use powerpoint::PowerPoint;
pub use word::Word;

use crate::extract::{fact, Flow, Limits, ReadError, Sink, MB};
use quick_xml::events::{BytesStart, Event};
use std::cell::Cell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::rc::Rc;
use zip::ZipArchive;

/// What an Office reader may take on, on top of the worker's limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    /// Bytes read while opening the zip: its central directory, which is
    /// held in memory, one entry for every 46 bytes or more of it.
    pub directory_bytes: u64,
    /// Parts in the zip.
    pub entries: usize,
    /// A part that has unpacked more than ratio_after bytes, at more than
    /// ratio times its compressed size, is a zip bomb.
    pub ratio: u64,
    pub ratio_after: u64,
    /// Bytes unpacked over all parts; past it, reading stops and what was
    /// read is kept.
    pub inflated: u64,
    /// Cells looked at in a workbook, empty ones included.
    pub cells: usize,
    /// Characters of a workbook's shared strings kept.
    pub shared_chars: usize,
    /// Shared strings kept: past it, the rest of the list is not read.
    pub shared_strings: usize,
    /// Sheets or slides read: past it, the rest are left out.
    pub listed: usize,
    /// Bytes of one XML event (a text, tag, comment or CDATA section),
    /// which the XML reader holds whole: past it, reading stops.
    pub event: usize,
}

impl Default for Caps {
    fn default() -> Caps {
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
    }
}

/// Office files are opened up to this size: only their XML is unpacked,
/// and images and videos make them large.
pub const MAX_BYTES: u64 = 100 * MB;

fn limits() -> Limits {
    Limits { max_bytes: MAX_BYTES, ..Limits::default() }
}

pub(crate) const PASSWORD: &str = "password-protected";
pub(crate) const BOMB: &str = "looks like a zip bomb";

/// Which of the three a reader reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Word,
    Excel,
    PowerPoint,
}

impl Kind {
    /// Why a file that isn't one is skipped.
    pub(crate) fn not_one(self) -> &'static str {
        match self {
            Kind::Word => "not a Word document",
            Kind::Excel => "not an Excel workbook",
            Kind::PowerPoint => "not a PowerPoint presentation",
        }
    }

    /// Where the main part is when the package doesn't say.
    fn main_part(self) -> &'static str {
        match self {
            Kind::Word => "word/document.xml",
            Kind::Excel => "xl/workbook.xml",
            Kind::PowerPoint => "ppt/presentation.xml",
        }
    }

    /// The main part's root element.
    fn root(self) -> &'static [u8] {
        match self {
            Kind::Word => b"document",
            Kind::Excel => b"workbook",
            Kind::PowerPoint => b"presentation",
        }
    }
}

/// Why unpacking stopped, when a cap stopped it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Bomb,
    Inflated,
    /// One XML event grew past caps.event.
    Event,
}

/// The file, counting what the zip crate reads of it while budget lasts.
struct Budgeted {
    file: BufReader<File>,
    left: Rc<Cell<u64>>,
}

impl Read for Budgeted {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.left.get();
        if left == 0 {
            return Err(io::Error::other("over the budget for opening"));
        }
        let max = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
        let n = self.file.read(&mut buf[..max])?;
        self.left.set(left - n as u64);
        Ok(n)
    }
}

impl Seek for Budgeted {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

/// A relationship from one part to another, its target a path in the zip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rel {
    pub id: String,
    /// The last segment of its type: "officeDocument", "worksheet", ...
    pub kind: String,
    pub target: String,
}

/// Relationship ids to their targets, for looking up many.
pub(crate) fn targets(rels: &[Rel]) -> HashMap<&str, &str> {
    rels.iter().map(|r| (r.id.as_str(), r.target.as_str())).collect()
}

/// An open Office file.
pub(crate) struct Package {
    zip: ZipArchive<Budgeted>,
    /// Part names are case-insensitive: lowercase name to name.
    names: HashMap<String, String>,
    caps: Caps,
    inflated: Cell<u64>,
    stop: Cell<Option<Stop>>,
    /// XML events read, over every part: the sink is asked about the time
    /// every CHECK_EVERY of them, however small the parts.
    events: Cell<u32>,
    kind: Kind,
}

impl Package {
    pub(crate) fn open(path: &Path, kind: Kind, caps: Caps) -> Result<Package, ReadError> {
        let mut file = File::open(path)?;
        let mut magic = [0u8; 8];
        let n = read_up_to(&mut file, &mut magic)?;
        if n >= 4 && (magic[..4] == *b"PK\x03\x04" || magic[..4] == *b"PK\x05\x06") {
            // A zip.
        } else if n == 8 && magic == cfb::MAGIC {
            let why = match cfb::stream_names(&mut file) {
                Some(names) if names.iter().any(|n| n == "EncryptedPackage") => PASSWORD,
                Some(_) => "an older Word, Excel or PowerPoint format",
                None => "password-protected, or an older Word, Excel or PowerPoint format",
            };
            return Err(ReadError::Unsupported(why.into()));
        } else {
            return Err(ReadError::Unsupported(kind.not_one().into()));
        }
        // The zip crate reserves room for every entry the end record
        // declares before reading one: refuse a count past the cap first.
        if declared_entries(&mut file)?.is_some_and(|n| n > caps.entries as u64) {
            return Err(ReadError::Unsupported(format!("more than {} parts", caps.entries)));
        }
        file.seek(SeekFrom::Start(0))?;
        let left = Rc::new(Cell::new(caps.directory_bytes));
        let zip = match ZipArchive::new(Budgeted { file: BufReader::new(file), left: left.clone() }) {
            Ok(zip) => zip,
            Err(_) if left.get() == 0 => {
                let why = format!(
                    "damaged, or its list of parts is larger than {}",
                    crate::extract::describe_size(caps.directory_bytes)
                );
                return Err(ReadError::Other(why));
            }
            Err(e) => return Err(ReadError::Other(format!("damaged or incomplete: {e}"))),
        };
        left.set(u64::MAX);
        if zip.len() > caps.entries {
            return Err(ReadError::Unsupported(format!("more than {} parts", caps.entries)));
        }
        let names = zip.file_names().filter_map(|n| n.ok()).map(|n| (n.to_lowercase(), n.into_owned())).collect();
        Ok(Package {
            zip,
            names,
            caps,
            inflated: Cell::new(0),
            stop: Cell::new(None),
            events: Cell::new(0),
            kind,
        })
    }

    pub(crate) fn has(&self, name: &str) -> bool {
        self.names.contains_key(&name.to_lowercase())
    }

    /// Reads the part name with handler, unpacking it through the caps.
    /// None if there is no such part; Full if the handler or sink is full,
    /// or the cap on unpacking over all parts was reached (the sink is then
    /// cut).
    pub(crate) fn parse(
        &mut self,
        name: &str,
        handler: &mut dyn Handler,
        sink: &mut Sink,
    ) -> Result<Option<Flow>, ReadError> {
        let Some(real) = self.names.get(&name.to_lowercase()).cloned() else {
            return Ok(None);
        };
        if matches!(self.stop.get(), Some(Stop::Inflated | Stop::Event)) {
            return Ok(Some(sink.cut()));
        }
        let Some(index) = self.zip.index_for_name(&real) else {
            return Ok(None);
        };
        let entry = self.zip.by_index_data(index).map_err(|e| damaged(name, e))?;
        if entry.encrypted() {
            return Err(ReadError::Unsupported(PASSWORD.into()));
        }
        let compressed = entry.compressed_size();
        let file = self.zip.by_index(index).map_err(|e| damaged(name, e))?;
        let counted = Inflate {
            file,
            compressed,
            read: 0,
            total: &self.inflated,
            stop: &self.stop,
            caps: &self.caps,
        };
        let xml = Bounded { inner: BufReader::new(counted), consumed: 0, start: 0, max: self.caps.event as u64, over: false };
        match walk(xml, handler, sink, &self.events) {
            Ok(flow) => Ok(Some(flow)),
            Err(Walked::Read(e)) => Err(e),
            // Like the cap on unpacking: what was read is kept.
            Err(Walked::Long) => {
                self.stop.set(Some(Stop::Event));
                Ok(Some(sink.cut()))
            }
            Err(Walked::Xml(e)) => match self.stop.get() {
                Some(Stop::Bomb) => Err(ReadError::Unsupported(BOMB.into())),
                Some(Stop::Inflated | Stop::Event) => Ok(Some(sink.cut())),
                None => Err(damaged(name, e)),
            },
        }
    }

    /// The relationships of part (of the package itself for ""), in order,
    /// leaving out those to outside the file.
    pub(crate) fn rels(&mut self, part: &str, sink: &mut Sink) -> Result<Vec<Rel>, ReadError> {
        let (dir, file) = match part.rsplit_once('/') {
            Some((dir, file)) => (dir, file),
            None => ("", part),
        };
        let name = if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
        let mut rels = Rels { dir: dir.to_string(), found: Vec::new(), bytes: 0, over: false };
        self.parse(&name, &mut rels, sink)?;
        if rels.over {
            sink.note_truncated();
        }
        Ok(rels.found)
    }

    /// The main part, found through the package's relationships, else where
    /// it usually is; an error if there is none.
    pub(crate) fn main_part(&mut self, sink: &mut Sink) -> Result<String, ReadError> {
        let rels = self.rels("", sink)?;
        let main = rels
            .iter()
            .find(|r| r.kind == "officeDocument" && self.has(&r.target))
            .map(|r| r.target.clone())
            .unwrap_or_else(|| self.kind.main_part().to_string());
        if !self.has(&main) {
            return Err(ReadError::Unsupported(self.kind.not_one().into()));
        }
        // Facts first: they are small, and kept even if the text is cut.
        let core = rels.iter().find(|r| r.kind == "core-properties").map(|r| r.target.clone());
        let mut core_props = Props::new(&[
            (b"title", fact::TITLE),
            (b"creator", fact::AUTHOR),
            (b"created", fact::CREATED),
            (b"modified", fact::MODIFIED),
        ]);
        self.parse(core.as_deref().unwrap_or("docProps/core.xml"), &mut core_props, sink)?;
        if self.kind == Kind::Word {
            let app = rels.iter().find(|r| r.kind == "extended-properties").map(|r| r.target.clone());
            let mut app_props = Props::new(&[(b"Pages", fact::PAGES)]);
            self.parse(app.as_deref().unwrap_or("docProps/app.xml"), &mut app_props, sink)?;
        }
        Ok(main)
    }

    /// Checks that the main part's root is what this reader reads.
    pub(crate) fn check_root(&self, root: Option<&[u8]>) -> Result<(), ReadError> {
        if root == Some(self.kind.root()) {
            Ok(())
        } else {
            Err(ReadError::Unsupported(self.kind.not_one().into()))
        }
    }
}

/// How many entries the zip's end record declares (its zip64 record's
/// count, when there is one and it is larger), or None if no end record is
/// found. Read from at most the last 64 KB and the 56 bytes of a zip64
/// record.
fn declared_entries(file: &mut File) -> io::Result<Option<u64>> {
    const END: &[u8] = b"PK\x05\x06";
    const LOCATOR: &[u8] = b"PK\x06\x07";
    const END64: &[u8] = b"PK\x06\x06";
    let len = file.seek(SeekFrom::End(0))?;
    let tail_len = len.min(22 + 65_535);
    let mut tail = vec![0u8; tail_len as usize];
    file.seek(SeekFrom::Start(len - tail_len))?;
    file.read_exact(&mut tail)?;
    let Some(at) = tail.windows(4).rposition(|w| w == END).filter(|at| at + 22 <= tail.len()) else {
        return Ok(None);
    };
    let mut entries = u64::from(u16::from_le_bytes([tail[at + 10], tail[at + 11]]));
    if at >= 20 && &tail[at - 20..at - 16] == LOCATOR {
        let offset = u64::from_le_bytes(tail[at - 12..at - 4].try_into().unwrap());
        if offset.checked_add(56).is_some_and(|end| end <= len) {
            let mut record = [0u8; 56];
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(&mut record)?;
            if &record[..4] == END64 {
                entries = entries.max(u64::from_le_bytes(record[32..40].try_into().unwrap()));
            }
        }
    }
    Ok(Some(entries))
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match file.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

fn damaged(part: &str, e: impl std::fmt::Display) -> ReadError {
    ReadError::Other(format!("damaged or incomplete: {part}: {e}"))
}

/// A part being unpacked, counted against the caps.
struct Inflate<'a, R: Read> {
    file: R,
    compressed: u64,
    read: u64,
    total: &'a Cell<u64>,
    stop: &'a Cell<Option<Stop>>,
    caps: &'a Caps,
}

impl<R: Read> Read for Inflate<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.stop.get().is_some() {
            return Err(io::Error::other("unpacking stopped"));
        }
        let n = self.file.read(buf)?;
        self.read += n as u64;
        self.total.set(self.total.get() + n as u64);
        if self.read > self.caps.ratio_after && self.read > self.caps.ratio.saturating_mul(self.compressed.max(1)) {
            self.stop.set(Some(Stop::Bomb));
            return Err(io::Error::other(BOMB));
        }
        if self.total.get() > self.caps.inflated {
            self.stop.set(Some(Stop::Inflated));
            return Err(io::Error::other("unpacked too much"));
        }
        Ok(n)
    }
}

/// What reads one part's XML. Elements are named by their local name: the
/// prefixes Office uses are fixed by convention, not by the namespaces.
pub(crate) trait Handler {
    fn open(&mut self, name: &[u8], e: &BytesStart, sink: &mut Sink) -> Result<Flow, ReadError>;
    fn close(&mut self, name: &[u8], sink: &mut Sink) -> Result<Flow, ReadError>;
    fn text(&mut self, text: &str, sink: &mut Sink) -> Result<Flow, ReadError>;
}

enum Walked {
    Read(ReadError),
    Xml(quick_xml::Error),
    /// One event grew past its cap.
    Long,
}

/// A part's XML as the XML reader takes it, refusing to let one event grow
/// past max bytes: the reader holds an event whole (a text node, a tag with
/// its attributes, a comment, a CDATA section), however long. It takes the
/// bytes of an event as it scans it, so what it has taken since the last
/// event ended is the event so far.
struct Bounded<R> {
    inner: R,
    consumed: u64,
    /// Where the current event began.
    start: u64,
    max: u64,
    over: bool,
}

impl<R: BufRead> Read for Bounded<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = {
            let have = self.fill_buf()?;
            let n = have.len().min(out.len());
            out[..n].copy_from_slice(&have[..n]);
            n
        };
        self.consume(n);
        Ok(n)
    }
}

impl<R: BufRead> BufRead for Bounded<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        // A byte-order mark is not part of the first event.
        if self.consumed == 0 && self.start == 0 && self.inner.fill_buf()?.starts_with(b"\xEF\xBB\xBF") {
            self.start = 3;
        }
        let used = self.consumed.saturating_sub(self.start);
        if used > self.max {
            self.over = true;
            return Err(io::Error::other("an XML event is too long"));
        }
        // No more than one byte past the cap at a time: an event longer
        // than that has to ask again, and is refused.
        let room = (self.max - used + 1).min(usize::MAX as u64) as usize;
        let have = self.inner.fill_buf()?;
        Ok(&have[..have.len().min(room)])
    }
    fn consume(&mut self, n: usize) {
        self.consumed += n as u64;
        self.inner.consume(n);
    }
}

/// Elements open at once: real Office files nest a few dozen deep.
const MAX_DEPTH: usize = 10_000;

/// Bytes of an element's name: Office's are under 64. With MAX_DEPTH, this
/// bounds the open names the XML reader keeps (2.5 MB).
const MAX_NAME: usize = 256;

/// The sink is asked about the time limit every this many events: often
/// enough that events made costly by what earlier parts left behind can't
/// run on long (a clock read is some tens of nanoseconds).
pub(crate) const CHECK_EVERY: u32 = 256;

/// Feeds the XML's events to handler until its end, or a Full. Empty
/// elements come as an open and a close. Of entity references only XML's
/// own five and character references are decoded; others (declared in a
/// DTD, which is never read) are dropped.
fn walk<R: BufRead>(
    xml: Bounded<R>,
    handler: &mut dyn Handler,
    sink: &mut Sink,
    events: &Cell<u32>,
) -> Result<Flow, Walked> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut buf = Vec::new();
    let mut depth = 0usize;
    loop {
        events.set(events.get().wrapping_add(1));
        if events.get().is_multiple_of(CHECK_EVERY) {
            sink.check().map_err(Walked::Read)?;
        }
        buf.clear();
        let event = match reader.read_event_into(&mut buf) {
            Ok(event) => event,
            Err(_) if reader.get_ref().over => return Err(Walked::Long),
            Err(e) => return Err(Walked::Xml(e)),
        };
        let bounded = reader.get_mut();
        bounded.start = bounded.consumed;
        // The XML reader keeps every open element's name: deep nesting is
        // stopped like a long event.
        match &event {
            Event::Start(e) => {
                depth += 1;
                if depth > MAX_DEPTH || e.name().as_ref().len() > MAX_NAME {
                    return Err(Walked::Long);
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        let flow = match event {
            Event::Start(e) => handler.open(e.local_name().as_ref().as_bytes(), &e, sink),
            Event::End(e) => handler.close(e.local_name().as_ref().as_bytes(), sink),
            Event::Text(t) => handler.text(&t.xml10_content(), sink),
            Event::CData(t) => handler.text(&t, sink),
            Event::GeneralRef(r) => {
                let mut ch = [0u8; 4];
                let s = match r.resolve_char_ref() {
                    Ok(Some(c)) => Some(&*c.encode_utf8(&mut ch)),
                    Ok(None) => quick_xml::escape::resolve_predefined_entity(&r),
                    Err(_) => None,
                };
                match s {
                    Some(s) => handler.text(s, sink),
                    None => Ok(Flow::More),
                }
            }
            Event::Eof => return Ok(Flow::More),
            _ => Ok(Flow::More),
        }
        .map_err(Walked::Read)?;
        if flow == Flow::Full {
            return Ok(Flow::Full);
        }
    }
}

/// The value of the attribute with this local name, entities decoded.
/// Attributes are read without quick-xml's check for repeated names: that
/// check scans every earlier name for each repeat, all within one event
/// where the time limit is never asked, and Office files don't repeat them.
/// An attribute value read is at most this long: every one the readers use
/// (ids, types, paths, names, format codes) is far shorter. A longer one
/// reads as absent.
const MAX_ATTR: usize = 4096;

/// An attribute's value, by its local name (any prefix).
pub(crate) fn attr(e: &BytesStart, local: &[u8]) -> Option<String> {
    e.attributes()
        .with_checks(false)
        .flatten()
        .find(|a| a.key.local_name().as_ref().as_bytes() == local)
        .filter(|a| a.value.len() <= MAX_ATTR)
        .map(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_default())
}

/// The relationship id of an element: the r:id attribute (any prefix, but
/// with one, since an element may have its own plain id too).
pub(crate) fn rel_id(e: &BytesStart) -> Option<String> {
    e.attributes()
        .with_checks(false)
        .flatten()
        .find(|a| a.key.prefix().is_some() && a.key.local_name().as_ref() == "id")
        .filter(|a| a.value.len() <= MAX_ATTR)
        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
}

/// Relationships kept from one .rels part: past it, the rest are left out.
const MAX_RELS: usize = 50_000;

/// Bytes of resolved targets kept from one .rels part: a target is joined
/// to its part's folder, itself perhaps long, so the count alone doesn't
/// bound them.
const MAX_RELS_BYTES: usize = 4 * MB as usize;

/// Reads a .rels part, its first MAX_RELS relationships. A target that
/// resolves to a path longer than MAX_ATTR is left out (no part of an
/// Office file has such a name).
struct Rels {
    dir: String,
    found: Vec<Rel>,
    bytes: usize,
    over: bool,
}

impl Handler for Rels {
    fn open(&mut self, name: &[u8], e: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        if name == b"Relationship" && attr(e, b"TargetMode").as_deref() != Some("External") {
            if self.found.len() >= MAX_RELS {
                self.over = true;
                return Ok(Flow::Full);
            }
            let (Some(id), Some(kind), Some(target)) = (attr(e, b"Id"), attr(e, b"Type"), attr(e, b"Target")) else {
                return Ok(Flow::More);
            };
            let kind = kind.rsplit('/').next().unwrap_or_default().to_string();
            let target = resolve(&self.dir, &target);
            if target.len() > MAX_ATTR {
                return Ok(Flow::More);
            }
            self.bytes += id.len() + kind.len() + target.len();
            if self.bytes > MAX_RELS_BYTES {
                self.over = true;
                return Ok(Flow::Full);
            }
            self.found.push(Rel { id, kind, target });
        }
        Ok(Flow::More)
    }
    fn close(&mut self, _: &[u8], _: &mut Sink) -> Result<Flow, ReadError> {
        Ok(Flow::More)
    }
    fn text(&mut self, _: &str, _: &mut Sink) -> Result<Flow, ReadError> {
        Ok(Flow::More)
    }
}

/// A relationship's target as a path in the zip: relative to the folder of
/// the part it is from, or to the package's root when it starts with "/".
pub(crate) fn resolve(dir: &str, target: &str) -> String {
    let mut out: Vec<&str> = if target.starts_with('/') { Vec::new() } else { dir.split('/').collect() };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            seg => out.push(seg),
        }
    }
    out.retain(|s| !s.is_empty());
    out.join("/")
}

/// Reads elements' text into facts: docProps/core.xml and app.xml.
/// Each key's first value is kept: a file repeating an element millions of
/// times makes one fact, not millions.
struct Props {
    keys: Vec<(&'static [u8], &'static str)>,
    current: Option<&'static str>,
    value: String,
    found: Vec<&'static str>,
}

impl Props {
    fn new(keys: &[(&'static [u8], &'static str)]) -> Props {
        Props { keys: keys.to_vec(), current: None, value: String::new(), found: Vec::new() }
    }
}

impl Handler for Props {
    fn open(&mut self, name: &[u8], _: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        self.current = self.keys.iter().find(|(n, _)| *n == name).map(|(_, k)| *k).filter(|k| !self.found.contains(k));
        self.value.clear();
        Ok(Flow::More)
    }
    fn close(&mut self, _: &[u8], sink: &mut Sink) -> Result<Flow, ReadError> {
        if let Some(key) = self.current.take() {
            self.found.push(key);
            sink.fact(key, std::mem::take(&mut self.value));
        }
        Ok(Flow::More)
    }
    fn text(&mut self, text: &str, _: &mut Sink) -> Result<Flow, ReadError> {
        if self.current.is_some() && self.value.len() < 4 * crate::extract::MAX_FACT_CHARS {
            self.value.push_str(text);
        }
        Ok(Flow::More)
    }
}

/// Text in paragraphs, runs and tables, as Word (w:) and DrawingML (a:,
/// for slides) both write it: text is inside t, a paragraph (p) ends a
/// line, br is a new line, tab in a run a tab. A text box's paragraphs
/// start on a line of their own. In a table a cell's paragraphs are joined
/// by spaces, its cells by tabs, its rows by lines; each cell is written as
/// it ends (or as it comes, when long), so no row is held whole. Of an
/// mc:AlternateContent only the Choice is read: the Fallback repeats it.
/// Pages are kept for a Word document's body when paged is set.
pub(crate) struct Prose {
    line: String,
    cell: String,
    /// The current cell has been written in part already (it was long).
    cell_started: bool,
    /// The current cell has had text other than white space.
    cell_text: bool,
    /// Cells of the current row written.
    row_cells: usize,
    /// Tables, cells and runs open around the current position.
    tables: u32,
    cells: u32,
    runs: u32,
    /// Inside t, and inside something skipped whole.
    in_text: u32,
    skip: u32,
    /// Elements whose insides are skipped whole.
    skipped: &'static [&'static [u8]],
    paged: bool,
    page: u32,
    /// Text since the current page began, so a rendered break right after
    /// another break (Word writes one after a typed break) counts once.
    text_on_page: bool,
    /// Breaks inside a table row already under way: the next row begins
    /// the page after them (two typed breaks leave a blank page).
    pending_breaks: u32,
    /// Whether anything was found at all.
    pub(crate) any: bool,
}

/// A line, or a table cell, this long is written before it ends.
const LONG_LINE: usize = 64 * 1024;

impl Prose {
    pub(crate) fn new(skipped: &'static [&'static [u8]], paged: bool) -> Prose {
        Prose {
            line: String::new(),
            cell: String::new(),
            cell_started: false,
            cell_text: false,
            row_cells: 0,
            tables: 0,
            cells: 0,
            runs: 0,
            in_text: 0,
            skip: 0,
            skipped,
            paged,
            page: 1,
            text_on_page: false,
            pending_breaks: 0,
            any: false,
        }
    }

    /// Writes the line so far, with a line end.
    fn end_line(&mut self, sink: &mut Sink) -> Result<Flow, ReadError> {
        if self.line.trim().is_empty() {
            self.line.clear();
            return Ok(Flow::More);
        }
        self.line.push('\n');
        let flow = sink.text(&self.line)?;
        self.line.clear();
        Ok(flow)
    }

    /// Writes the cell so far: all of it when it ends (last), else what has
    /// come of a long one. Cells are joined by tabs.
    fn write_cell(&mut self, sink: &mut Sink, last: bool) -> Result<Flow, ReadError> {
        let cell = std::mem::take(&mut self.cell);
        if last {
            self.cell_text = false;
        }
        let text = match (self.cell_started, last) {
            (false, true) => cell.trim(),
            (false, false) => cell.trim_start(),
            (true, true) => cell.trim_end(),
            (true, false) => cell.as_str(),
        };
        if text.is_empty() {
            if last {
                self.cell_started = false;
            }
            return Ok(Flow::More);
        }
        if !self.cell_started && self.row_cells > 0 && sink.text("\t")? == Flow::Full {
            return Ok(Flow::Full);
        }
        let flow = sink.text(text)?;
        if !self.cell_started {
            self.row_cells += 1;
        }
        self.cell_started = !last;
        Ok(flow)
    }

    fn page_break(&mut self, sink: &mut Sink, typed: bool) -> Result<Flow, ReadError> {
        if !self.paged {
            return Ok(Flow::More);
        }
        // Text arriving sets text_on_page, and a break clears it: so a
        // rendered break with nothing since the last break is the one Word
        // writes after a typed break, and is dropped.
        if !typed && !self.text_on_page {
            return Ok(Flow::More);
        }
        if self.tables > 0 {
            // At a row's start (where Word marks a row that begins a page)
            // the row goes on the new page; else the next row does.
            if self.row_cells == 0 && !self.cell_started && !self.cell_text {
                return self.new_page(sink);
            }
            self.pending_breaks = self.pending_breaks.saturating_add(1);
            self.text_on_page = false;
            return Ok(Flow::More);
        }
        self.new_page(sink)
    }

    fn new_page(&mut self, sink: &mut Sink) -> Result<Flow, ReadError> {
        self.new_pages(sink, 1)
    }

    /// Starts the page n pages on (more than one leaves blank pages).
    fn new_pages(&mut self, sink: &mut Sink, n: u32) -> Result<Flow, ReadError> {
        // The line goes on: its first half ends this page.
        if !self.line.trim().is_empty() {
            let line = std::mem::take(&mut self.line);
            if sink.text(&line)? == Flow::Full || sink.text("\n")? == Flow::Full {
                return Ok(Flow::Full);
            }
        }
        self.line.clear();
        if self.page == 1 {
            sink.relabel("page 1");
        }
        self.page = self.page.saturating_add(n);
        sink.part(format!("page {}", self.page));
        self.text_on_page = false;
        self.pending_breaks = 0;
        Ok(Flow::More)
    }

    /// Writes what is left, at the end of a part.
    pub(crate) fn finish(&mut self, sink: &mut Sink) -> Result<Flow, ReadError> {
        self.end_line(sink)
    }

    fn push(&mut self, s: &str) {
        if !s.trim().is_empty() {
            self.any = true;
            self.text_on_page = true;
            if self.cells > 0 {
                self.cell_text = true;
            }
        }
        if self.cells > 0 {
            self.cell.push_str(s);
        } else {
            self.line.push_str(s);
        }
    }

    /// A line or cell grown long is written as it comes.
    fn flush_long(&mut self, sink: &mut Sink) -> Result<Flow, ReadError> {
        if self.line.len() >= LONG_LINE {
            let line = std::mem::take(&mut self.line);
            return sink.text(&line);
        }
        // A nested table's text is part of its outer cell.
        if self.cell.len() >= LONG_LINE && self.cells > 0 {
            return self.write_cell(sink, false);
        }
        Ok(Flow::More)
    }
}

/// The character of a w:sym, when it is a plain Unicode character, not one
/// of a symbol font's private-use code points.
fn symbol(e: &BytesStart) -> Option<char> {
    let code = u32::from_str_radix(attr(e, b"char")?.trim(), 16).ok()?;
    let c = char::from_u32(code)?;
    (!c.is_control() && !(0xE000..=0xF8FF).contains(&code)).then_some(c)
}

impl Handler for Prose {
    fn open(&mut self, name: &[u8], e: &BytesStart, sink: &mut Sink) -> Result<Flow, ReadError> {
        if self.skip > 0 || self.skipped.contains(&name) || (name == b"fld" && attr(e, b"type").as_deref() == Some("slidenum"))
        {
            self.skip += 1;
            return Ok(Flow::More);
        }
        match name {
            b"t" => self.in_text += 1,
            b"r" => self.runs += 1,
            b"tbl" => {
                if self.tables == 0 {
                    // A table starts on a line of its own.
                    if self.end_line(sink)? == Flow::Full {
                        return Ok(Flow::Full);
                    }
                }
                self.tables += 1;
            }
            b"tc" => self.cells += 1,
            b"tab" if self.runs > 0 => self.push(if self.cells > 0 { " " } else { "\t" }),
            b"noBreakHyphen" if self.runs > 0 => self.push("-"),
            b"sym" if self.runs > 0 => {
                if let Some(c) = symbol(e) {
                    self.push(c.encode_utf8(&mut [0u8; 4]));
                }
            }
            b"br" | b"cr" => {
                if self.paged && name == b"br" && attr(e, b"type").as_deref() == Some("page") {
                    return self.page_break(sink, true);
                }
                if self.cells > 0 {
                    self.push(" ");
                } else {
                    return self.end_line(sink);
                }
            }
            b"txbxContent" => {
                if self.cells > 0 {
                    self.push(" ");
                } else {
                    return self.end_line(sink);
                }
            }
            b"lastRenderedPageBreak" => return self.page_break(sink, false),
            _ => {}
        }
        Ok(Flow::More)
    }

    fn close(&mut self, name: &[u8], sink: &mut Sink) -> Result<Flow, ReadError> {
        if self.skip > 0 {
            self.skip -= 1;
            return Ok(Flow::More);
        }
        match name {
            b"t" => self.in_text = self.in_text.saturating_sub(1),
            b"r" => self.runs = self.runs.saturating_sub(1),
            b"p" if self.cells > 0 => self.push(" "),
            b"p" => return self.end_line(sink),
            b"tc" => {
                self.cells = self.cells.saturating_sub(1);
                if self.tables == 1 {
                    return self.write_cell(sink, true);
                }
                self.push(" ");
            }
            b"tr" if self.tables == 1 => {
                if self.row_cells > 0 {
                    self.row_cells = 0;
                    if sink.text("\n")? == Flow::Full {
                        return Ok(Flow::Full);
                    }
                }
                if self.pending_breaks > 0 {
                    let n = self.pending_breaks;
                    return self.new_pages(sink, n);
                }
            }
            b"tbl" => self.tables = self.tables.saturating_sub(1),
            _ => {}
        }
        Ok(Flow::More)
    }

    fn text(&mut self, text: &str, sink: &mut Sink) -> Result<Flow, ReadError> {
        if self.skip == 0 && self.in_text > 0 {
            self.push(text);
            return self.flush_long(sink);
        }
        Ok(Flow::More)
    }
}

/// Notes the main part's root element, then stops caring.
#[derive(Default)]
pub(crate) struct Root(pub Option<Vec<u8>>);

impl Root {
    pub(crate) fn see(&mut self, name: &[u8]) {
        if self.0.is_none() {
            self.0 = Some(name.to_vec());
        }
    }

    pub(crate) fn get(&self) -> Option<&[u8]> {
        self.0.as_deref()
    }
}
