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

/// An open Office file.
pub(crate) struct Package {
    zip: ZipArchive<Budgeted>,
    /// Part names are case-insensitive: lowercase name to name.
    names: HashMap<String, String>,
    caps: Caps,
    inflated: Cell<u64>,
    stop: Cell<Option<Stop>>,
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
        Ok(Package { zip, names, caps, inflated: Cell::new(0), stop: Cell::new(None), kind })
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
        if self.stop.get() == Some(Stop::Inflated) {
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
        match walk(BufReader::new(counted), handler, sink) {
            Ok(flow) => Ok(Some(flow)),
            Err(Walked::Read(e)) => Err(e),
            Err(Walked::Xml(e)) => match self.stop.get() {
                Some(Stop::Bomb) => Err(ReadError::Unsupported(BOMB.into())),
                Some(Stop::Inflated) => Ok(Some(sink.cut())),
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
        let mut rels = Rels { dir: dir.to_string(), found: Vec::new() };
        self.parse(&name, &mut rels, sink)?;
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
}

/// The sink is asked about the time limit every this many events.
const CHECK_EVERY: u32 = 4096;

/// Feeds the XML's events to handler until its end, or a Full. Empty
/// elements come as an open and a close. Of entity references only XML's
/// own five and character references are decoded; others (declared in a
/// DTD, which is never read) are dropped.
fn walk(xml: impl BufRead, handler: &mut dyn Handler, sink: &mut Sink) -> Result<Flow, Walked> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut buf = Vec::new();
    let mut events = 0u32;
    loop {
        events = events.wrapping_add(1);
        if events % CHECK_EVERY == 0 {
            sink.check().map_err(Walked::Read)?;
        }
        buf.clear();
        let flow = match reader.read_event_into(&mut buf).map_err(Walked::Xml)? {
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
pub(crate) fn attr(e: &BytesStart, local: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref().as_bytes() == local)
        .map(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_default())
}

/// The relationship id of an element: the r:id attribute (any prefix, but
/// with one, since an element may have its own plain id too).
pub(crate) fn rel_id(e: &BytesStart) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.prefix().is_some() && a.key.local_name().as_ref() == "id")
        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
}

/// Reads a .rels part.
struct Rels {
    dir: String,
    found: Vec<Rel>,
}

impl Handler for Rels {
    fn open(&mut self, name: &[u8], e: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        if name == b"Relationship" && attr(e, b"TargetMode").as_deref() != Some("External") {
            let (Some(id), Some(kind), Some(target)) = (attr(e, b"Id"), attr(e, b"Type"), attr(e, b"Target")) else {
                return Ok(Flow::More);
            };
            let kind = kind.rsplit('/').next().unwrap_or_default().to_string();
            self.found.push(Rel { id, kind, target: resolve(&self.dir, &target) });
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
struct Props {
    keys: Vec<(&'static [u8], &'static str)>,
    current: Option<&'static str>,
    value: String,
}

impl Props {
    fn new(keys: &[(&'static [u8], &'static str)]) -> Props {
        Props { keys: keys.to_vec(), current: None, value: String::new() }
    }
}

impl Handler for Props {
    fn open(&mut self, name: &[u8], _: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        self.current = self.keys.iter().find(|(n, _)| *n == name).map(|(_, k)| *k);
        self.value.clear();
        Ok(Flow::More)
    }
    fn close(&mut self, _: &[u8], sink: &mut Sink) -> Result<Flow, ReadError> {
        if let Some(key) = self.current.take() {
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
/// line, br is a new line, tab in a run a tab. In a table a cell's
/// paragraphs are joined by spaces, its cells by tabs, its rows by lines.
/// Of an mc:AlternateContent only the Choice is read: the Fallback repeats
/// it. Pages are kept for a Word document's body when paged is set.
pub(crate) struct Prose {
    line: String,
    cell: String,
    row: Vec<String>,
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
    /// Text since the current page began, so a break with nothing before
    /// it (Word writes a rendered break after a typed one) counts once.
    text_on_page: bool,
    /// Whether anything was found at all.
    pub(crate) any: bool,
}

impl Prose {
    pub(crate) fn new(skipped: &'static [&'static [u8]], paged: bool) -> Prose {
        Prose {
            line: String::new(),
            cell: String::new(),
            row: Vec::new(),
            tables: 0,
            cells: 0,
            runs: 0,
            in_text: 0,
            skip: 0,
            skipped,
            paged,
            page: 1,
            text_on_page: false,
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

    fn page_break(&mut self, sink: &mut Sink) -> Result<Flow, ReadError> {
        if !self.paged || self.tables > 0 || !(self.text_on_page || !self.line.trim().is_empty()) {
            return Ok(Flow::More);
        }
        // The line goes on: its first half ends this page.
        if !self.line.trim().is_empty() {
            let line = std::mem::take(&mut self.line);
            if sink.text(&line)? == Flow::Full || sink.text("\n")? == Flow::Full {
                return Ok(Flow::Full);
            }
        }
        if self.page == 1 {
            sink.relabel("page 1");
        }
        self.page += 1;
        sink.part(format!("page {}", self.page));
        self.text_on_page = false;
        Ok(Flow::More)
    }

    /// Writes what is left, at the end of a part.
    pub(crate) fn finish(&mut self, sink: &mut Sink) -> Result<Flow, ReadError> {
        self.end_line(sink)
    }
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
            b"tab" if self.runs > 0 => self.push("\t"),
            b"br" | b"cr" => {
                if self.paged && name == b"br" && attr(e, b"type").as_deref() == Some("page") {
                    return self.page_break(sink);
                }
                if self.tables == 0 {
                    return self.end_line(sink);
                }
                self.push(" ");
            }
            b"lastRenderedPageBreak" => return self.page_break(sink),
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
                    let cell = std::mem::take(&mut self.cell);
                    self.row.push(cell.split_whitespace().collect::<Vec<_>>().join(" "));
                } else {
                    self.push(" ");
                }
            }
            b"tr" if self.tables == 1 => {
                let row = std::mem::take(&mut self.row);
                self.line = row.into_iter().filter(|c| !c.is_empty()).collect::<Vec<_>>().join("\t");
                return self.end_line(sink);
            }
            b"tbl" => self.tables = self.tables.saturating_sub(1),
            _ => {}
        }
        Ok(Flow::More)
    }

    fn text(&mut self, text: &str, sink: &mut Sink) -> Result<Flow, ReadError> {
        if self.skip == 0 && self.in_text > 0 {
            self.push(text);
            // A paragraph can be as long as the part: write it as it comes.
            if self.line.len() >= LONG_LINE {
                let line = std::mem::take(&mut self.line);
                return sink.text(&line);
            }
        }
        Ok(Flow::More)
    }
}

/// A line this long is written before it ends.
const LONG_LINE: usize = 64 * 1024;

impl Prose {
    fn push(&mut self, s: &str) {
        if !s.trim().is_empty() {
            self.any = true;
            self.text_on_page = true;
        }
        if self.cells > 0 {
            self.cell.push_str(s);
        } else {
            self.line.push_str(s);
        }
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
