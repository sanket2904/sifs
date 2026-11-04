//! Excel workbooks: one part per sheet, labelled with its name, each row a
//! line of its non-empty cells joined by tabs.
//!
//! Cells are shown as stored, except dates: a number in a cell whose style
//! has a date or time format is shown as 2026-11-03, 2026-11-03 14:30 or
//! 14:30:00. Other number formats (thousands separators, currencies,
//! decimals) are not applied: the digits are what a search needs.

use super::{attr, limits, rel_id, targets, Caps, Handler, Kind, Package, Root};
use crate::extract::{fact, Flow, Limits, ReadError, Reader, Sink};
use quick_xml::events::BytesStart;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Default)]
pub struct Excel {
    pub caps: Caps,
}

impl Reader for Excel {
    fn id(&self) -> &'static str {
        "xlsx"
    }
    fn version(&self) -> u32 {
        1
    }
    fn exts(&self) -> &'static [&'static str] {
        &["xlsx", "xlsm", "xltx", "xltm"]
    }
    fn limits(&self) -> Limits {
        limits()
    }
    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError> {
        let mut package = Package::open(path, Kind::Excel, self.caps)?;
        let main = package.main_part(sink)?;
        let mut workbook = Workbook { max: self.caps.listed, ..Workbook::default() };
        package.parse(&main, &mut workbook, sink)?;
        package.check_root(workbook.root.get())?;
        if workbook.over {
            sink.note_truncated();
        }
        let rels = package.rels(&main, sink)?;
        let target_of = |kind: &str| rels.iter().find(|r| r.kind == kind).map(|r| r.target.clone());
        let names: Vec<&str> = workbook.sheets.iter().map(|s| s.name.as_str()).collect();
        sink.fact(fact::SHEETS, names.join(", "));

        let mut shared =
            Shared { max_chars: self.caps.shared_chars, max_strings: self.caps.shared_strings, ..Shared::default() };
        if let Some(target) = target_of("sharedStrings") {
            // Full from the list itself (past its count) still reads the sheets.
            if package.parse(&target, &mut shared, sink)? == Some(Flow::Full) && !shared.cut {
                return Ok(());
            }
        }
        if shared.cut {
            sink.note_truncated();
        }
        let mut styles = Styles::default();
        if let Some(target) = target_of("styles") {
            if package.parse(&target, &mut styles, sink)? == Some(Flow::Full) {
                return Ok(());
            }
        }
        let mut sheet = Sheet {
            shared: shared.strings,
            dates: styles.kinds(),
            date1904: workbook.date1904,
            cells: 0,
            max_cells: self.caps.cells,
            row_cells: 0,
            cell: None,
            in_value: false,
            in_inline: 0,
            skip: 0,
        };
        let targets = targets(&rels);
        let mut read = HashSet::new();
        for listed in &workbook.sheets {
            sink.check()?;
            let Some(&target) = targets.get(listed.id.as_str()) else { continue };
            // A part named by two sheets is read once.
            if !read.insert(target) {
                continue;
            }
            sink.part(if listed.hidden { format!("{} (hidden)", listed.name) } else { listed.name.clone() });
            let flow = package.parse(target, &mut sheet, sink)?;
            if sheet.cells > sheet.max_cells {
                sink.cut();
                return Ok(());
            }
            if flow == Some(Flow::Full) {
                return Ok(());
            }
        }
        Ok(())
    }
}

/// A sheet as the workbook lists it.
struct Listed {
    name: String,
    /// Its relationship id.
    id: String,
    hidden: bool,
}

/// workbook.xml: the sheets in order, and the date system. Very hidden
/// sheets (shown only to macros) are left out; at most max are listed.
#[derive(Default)]
struct Workbook {
    root: Root,
    sheets: Vec<Listed>,
    date1904: bool,
    max: usize,
    over: bool,
}

impl Handler for Workbook {
    fn open(&mut self, name: &[u8], e: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        self.root.see(name);
        if self.root.get() != Some(b"workbook") {
            return Ok(Flow::Full);
        }
        match name {
            b"workbookPr" => self.date1904 = matches!(attr(e, b"date1904").as_deref(), Some("1" | "true")),
            b"sheet" => {
                let state = attr(e, b"state");
                if state.as_deref() == Some("veryHidden") {
                    return Ok(Flow::More);
                }
                if self.sheets.len() >= self.max {
                    self.over = true;
                    return Ok(Flow::Full);
                }
                if let (Some(name), Some(id)) = (attr(e, b"name"), rel_id(e)) {
                    self.sheets.push(Listed { name, id, hidden: state.as_deref() == Some("hidden") });
                }
            }
            _ => {}
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

/// sharedStrings.xml: each si's text, rich runs joined, phonetic guides
/// (rPh) left out. Past max_chars in all, later strings are empty; past
/// max_strings, the rest of the list is not read.
#[derive(Default)]
struct Shared {
    strings: Vec<String>,
    current: String,
    /// Characters of current, and whether it has outgrown the cap.
    current_chars: usize,
    current_over: bool,
    in_text: u32,
    in_phonetic: u32,
    chars: usize,
    max_chars: usize,
    max_strings: usize,
    cut: bool,
}

impl Handler for Shared {
    fn open(&mut self, name: &[u8], _: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        match name {
            b"si" => {
                self.current.clear();
                self.current_chars = 0;
                self.current_over = false;
            }
            b"t" => self.in_text += 1,
            b"rPh" => self.in_phonetic += 1,
            _ => {}
        }
        Ok(Flow::More)
    }
    fn close(&mut self, name: &[u8], _: &mut Sink) -> Result<Flow, ReadError> {
        match name {
            b"si" => {
                if self.strings.len() >= self.max_strings {
                    self.cut = true;
                    return Ok(Flow::Full);
                }
                // Kept as every cell shows it, trimmed: a blank string is
                // empty, so a cell naming it costs nothing to show.
                let mut s = std::mem::take(&mut self.current);
                let trimmed = s.trim();
                if trimmed.len() < s.len() {
                    s = trimmed.to_string();
                }
                if self.current_over {
                    self.cut = true;
                    self.chars = self.max_chars;
                    self.strings.push(String::new());
                } else {
                    self.chars += self.current_chars;
                    self.strings.push(s);
                }
            }
            b"t" => self.in_text = self.in_text.saturating_sub(1),
            b"rPh" => self.in_phonetic = self.in_phonetic.saturating_sub(1),
            _ => {}
        }
        Ok(Flow::More)
    }
    fn text(&mut self, text: &str, _: &mut Sink) -> Result<Flow, ReadError> {
        // A string past the cap is dropped whole, so stop growing it. Its
        // characters are counted however its runs split it.
        if self.in_text > 0 && self.in_phonetic == 0 && !self.current_over {
            let n = text.chars().count();
            if self.chars + self.current_chars + n > self.max_chars {
                self.current_over = true;
                self.current.clear();
            } else {
                self.current_chars += n;
                self.current.push_str(text);
            }
        }
        Ok(Flow::More)
    }
}

/// How a cell style shows a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shown {
    Number,
    Date,
    DateTime,
    Time,
}

/// Custom number formats and cell styles kept: Excel allows about 250 of
/// the one and 64,000 of the other.
const MAX_STYLES: usize = 65_536;

/// styles.xml: the number format of each cell style (cellXfs). Each custom
/// format is classified once, as it is read: a style naming it only looks
/// it up.
#[derive(Default)]
struct Styles {
    formats: HashMap<u32, Shown>,
    xfs: Vec<u32>,
    in_cell_xfs: bool,
}

impl Styles {
    fn kinds(&self) -> Vec<Shown> {
        self.xfs.iter().map(|id| self.formats.get(id).copied().unwrap_or_else(|| shown_by_id(*id))).collect()
    }
}

impl Handler for Styles {
    fn open(&mut self, name: &[u8], e: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        match name {
            b"numFmt" => {
                if let (Some(id), Some(code)) = (attr(e, b"numFmtId"), attr(e, b"formatCode")) {
                    if let Ok(id) = id.parse() {
                        if self.formats.len() < MAX_STYLES || self.formats.contains_key(&id) {
                            self.formats.insert(id, shown_by_code(&code));
                        }
                    }
                }
            }
            b"cellXfs" => self.in_cell_xfs = true,
            b"xf" if self.in_cell_xfs && self.xfs.len() < MAX_STYLES => {
                self.xfs.push(attr(e, b"numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0));
            }
            _ => {}
        }
        Ok(Flow::More)
    }
    fn close(&mut self, name: &[u8], _: &mut Sink) -> Result<Flow, ReadError> {
        if name == b"cellXfs" {
            self.in_cell_xfs = false;
        }
        Ok(Flow::More)
    }
    fn text(&mut self, _: &str, _: &mut Sink) -> Result<Flow, ReadError> {
        Ok(Flow::More)
    }
}

/// Excel's built-in number formats that show dates and times.
pub(crate) fn shown_by_id(id: u32) -> Shown {
    match id {
        14..=17 | 27..=31 | 34..=36 | 50..=58 => Shown::Date,
        18..=21 | 32 | 33 | 45..=47 => Shown::Time,
        22 => Shown::DateTime,
        _ => Shown::Number,
    }
}

/// Whether a custom format code shows a date or time: d, m, y, h or s
/// outside quotes, brackets and escapes, in its first section. An m alone
/// is a month; with h or s, minutes.
pub(crate) fn shown_by_code(code: &str) -> Shown {
    let section = code.split(';').next().unwrap_or_default();
    let mut plain = String::new();
    let mut chars = section.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                }
            }
            '[' => {
                // [h], [mm], [ss] are elapsed time; [Red] or [$-409] are not.
                let inside: String = chars.by_ref().take_while(|c| *c != ']').collect();
                if inside.chars().all(|c| matches!(c.to_ascii_lowercase(), 'h' | 'm' | 's')) {
                    plain.push_str(&inside);
                }
            }
            '\\' | '_' | '*' => {
                chars.next();
            }
            c => plain.push(c.to_ascii_lowercase()),
        }
    }
    if plain.contains("general") {
        return Shown::Number;
    }
    let time = plain.contains('h') || plain.contains('s');
    let date = plain.contains('d') || plain.contains('y') || (plain.contains('m') && !time);
    match (date, time) {
        (true, true) => Shown::DateTime,
        (true, false) => Shown::Date,
        (false, true) => Shown::Time,
        (false, false) => Shown::Number,
    }
}

/// A serial date shown in ISO form, or None if it is out of range.
pub(crate) fn show_date(serial: f64, shown: Shown, date1904: bool) -> Option<String> {
    if !serial.is_finite() || !(0.0..3_000_000.0).contains(&serial) {
        return None;
    }
    // Before day 1 of the 1900 system Excel shows "1900-01-00": shown as
    // the number it is.
    if !date1904 && serial < 1.0 && shown != Shown::Time {
        return None;
    }
    let mut days = serial.floor() as i64;
    let mut seconds = ((serial - serial.floor()) * 86_400.0).round() as i64;
    if seconds >= 86_400 {
        days += 1;
        seconds -= 86_400;
    }
    // Days from 1970-01-01. The 1900 system counts 1900-02-29, which never
    // was (as Lotus 1-2-3 did): serial 60 is shown as that day.
    let (y, m, d) = if date1904 {
        civil(days - 24_107)
    } else if days == 60 {
        (1900, 2, 29)
    } else if days < 60 {
        civil(days - 25_568)
    } else {
        civil(days - 25_569)
    };
    let (h, min, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    Some(match shown {
        Shown::Date => format!("{y:04}-{m:02}-{d:02}"),
        Shown::DateTime if s == 0 => format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}"),
        Shown::DateTime => format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}"),
        Shown::Time => format!("{h:02}:{min:02}:{s:02}"),
        Shown::Number => return None,
    })
}

/// (year, month, day) of a day counted from 1970-01-01 (Howard Hinnant's
/// civil_from_days).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// The cell being read: its type and style, and its value so far.
struct Cell {
    kind: String,
    style: usize,
    value: String,
    /// Characters of value.
    chars: usize,
}

/// A cell's value is kept up to Excel's own limit, in characters; the rest
/// is dropped and recorded.
pub(crate) const MAX_CELL_CHARS: usize = 32_767;

/// One sheet's cells as rows, each cell written as it ends: tabs between
/// cells, a line end after each row. Nothing of a row is held.
struct Sheet {
    shared: Vec<String>,
    dates: Vec<Shown>,
    date1904: bool,
    /// Cells looked at so far, over every sheet.
    cells: usize,
    max_cells: usize,
    /// Cells of the current row written.
    row_cells: usize,
    cell: Option<Cell>,
    in_value: bool,
    /// Inside an inline string's t.
    in_inline: u32,
    /// Inside a phonetic guide.
    skip: u32,
}

/// What a cell shows, borrowed where it can be (a shared string).
fn shown<'a>(cell: &'a Cell, shared: &'a [String], dates: &[Shown], date1904: bool) -> Cow<'a, str> {
    let value = cell.value.trim();
    match cell.kind.as_str() {
        "s" => Cow::Borrowed(value.parse::<usize>().ok().and_then(|i| shared.get(i)).map_or("", |s| s.as_str())),
        "b" => Cow::Borrowed(match value {
            "1" => "TRUE",
            "0" => "FALSE",
            other => other,
        }),
        "inlineStr" | "str" | "e" | "d" => Cow::Borrowed(&cell.value),
        _ => {
            let kind = dates.get(cell.style).copied().unwrap_or(Shown::Number);
            match value.parse::<f64>() {
                Ok(serial) if kind != Shown::Number => {
                    show_date(serial, kind, date1904).map_or(Cow::Borrowed(value), Cow::Owned)
                }
                _ => Cow::Borrowed(value),
            }
        }
    }
}

impl Handler for Sheet {
    fn open(&mut self, name: &[u8], e: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        match name {
            b"row" => self.row_cells = 0,
            b"c" => {
                self.cells += 1;
                if self.cells > self.max_cells {
                    return Ok(Flow::Full);
                }
                self.cell = Some(Cell {
                    kind: attr(e, b"t").unwrap_or_default(),
                    style: attr(e, b"s").and_then(|s| s.parse().ok()).unwrap_or(0),
                    value: String::new(),
                    chars: 0,
                });
            }
            b"v" => self.in_value = true,
            b"t" => self.in_inline += 1,
            b"rPh" => self.skip += 1,
            _ => {}
        }
        Ok(Flow::More)
    }
    fn close(&mut self, name: &[u8], sink: &mut Sink) -> Result<Flow, ReadError> {
        match name {
            b"v" => self.in_value = false,
            b"t" => self.in_inline = self.in_inline.saturating_sub(1),
            b"rPh" => self.skip = self.skip.saturating_sub(1),
            b"c" => {
                let Some(cell) = self.cell.take() else { return Ok(Flow::More) };
                let shown = shown(&cell, &self.shared, &self.dates, self.date1904);
                let shown = shown.trim();
                if shown.is_empty() {
                    return Ok(Flow::More);
                }
                if self.row_cells > 0 && sink.text("\t")? == Flow::Full {
                    return Ok(Flow::Full);
                }
                self.row_cells += 1;
                return if shown.contains(['\t', '\n', '\r']) {
                    sink.text(&shown.replace(['\t', '\n', '\r'], " "))
                } else {
                    sink.text(shown)
                };
            }
            b"row" if self.row_cells > 0 => {
                self.row_cells = 0;
                return sink.text("\n");
            }
            _ => {}
        }
        Ok(Flow::More)
    }
    fn text(&mut self, text: &str, sink: &mut Sink) -> Result<Flow, ReadError> {
        if let Some(cell) = &mut self.cell {
            let inline = cell.kind == "inlineStr" && self.in_inline > 0 && self.skip == 0;
            if self.in_value || inline {
                let room = MAX_CELL_CHARS - cell.chars;
                let kept = match text.char_indices().nth(room) {
                    Some((cut, _)) => {
                        sink.note_truncated();
                        &text[..cut]
                    }
                    None => text,
                };
                cell.chars += kept.chars().count();
                cell.value.push_str(kept);
            }
        }
        Ok(Flow::More)
    }
}

#[cfg(test)]
mod shared_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn shared_strings_are_stored_trimmed() {
        let mut sink = Sink::new(100, Instant::now() + Duration::from_secs(10));
        let mut shared = Shared { max_chars: 1_000_000, max_strings: 10, ..Shared::default() };
        let e = BytesStart::new("x");
        for text in ["  padded  ", "    ", "plain"] {
            shared.open(b"si", &e, &mut sink).unwrap();
            shared.open(b"t", &e, &mut sink).unwrap();
            shared.text(text, &mut sink).unwrap();
            shared.close(b"t", &mut sink).unwrap();
            shared.close(b"si", &mut sink).unwrap();
        }
        assert_eq!(shared.strings, ["padded", "", "plain"]);
    }
}
