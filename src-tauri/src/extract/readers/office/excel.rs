//! Excel workbooks: one part per sheet, labelled with its name, each row a
//! line of its non-empty cells joined by tabs.
//!
//! Cells are shown as stored, except dates: a number in a cell whose style
//! has a date or time format is shown as 2026-11-03, 2026-11-03 14:30 or
//! 14:30:00. Other number formats (thousands separators, currencies,
//! decimals) are not applied: the digits are what a search needs.

use super::{attr, limits, rel_id, Caps, Handler, Kind, Package, Root};
use crate::extract::{fact, Flow, Limits, ReadError, Reader, Sink};
use quick_xml::events::BytesStart;
use std::collections::HashMap;
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
        let mut workbook = Workbook::default();
        package.parse(&main, &mut workbook, sink)?;
        package.check_root(workbook.root.get())?;
        let rels = package.rels(&main, sink)?;
        let target_of = |kind: &str| rels.iter().find(|r| r.kind == kind).map(|r| r.target.clone());
        let names: Vec<&str> = workbook.sheets.iter().map(|(name, _)| name.as_str()).collect();
        sink.fact(fact::SHEETS, names.join(", "));

        let mut shared = Shared { max_chars: self.caps.shared_chars, ..Shared::default() };
        if let Some(target) = target_of("sharedStrings") {
            if package.parse(&target, &mut shared, sink)? == Some(Flow::Full) {
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
            row: Vec::new(),
            cell: None,
            in_value: false,
            in_inline: 0,
            skip: 0,
        };
        for (name, id) in &workbook.sheets {
            let Some(rel) = rels.iter().find(|r| &r.id == id) else { continue };
            sink.part(name.clone());
            let flow = package.parse(&rel.target, &mut sheet, sink)?;
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

/// workbook.xml: the sheets in order, and the date system.
#[derive(Default)]
struct Workbook {
    root: Root,
    /// (name, relationship id)
    sheets: Vec<(String, String)>,
    date1904: bool,
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
                if let (Some(sheet), Some(id)) = (attr(e, b"name"), rel_id(e)) {
                    self.sheets.push((sheet, id));
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
/// (rPh) left out. Past max_chars in all, later strings are empty.
#[derive(Default)]
struct Shared {
    strings: Vec<String>,
    current: String,
    in_text: u32,
    in_phonetic: u32,
    chars: usize,
    max_chars: usize,
    cut: bool,
}

impl Handler for Shared {
    fn open(&mut self, name: &[u8], _: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        match name {
            b"si" => self.current.clear(),
            b"t" => self.in_text += 1,
            b"rPh" => self.in_phonetic += 1,
            _ => {}
        }
        Ok(Flow::More)
    }
    fn close(&mut self, name: &[u8], _: &mut Sink) -> Result<Flow, ReadError> {
        match name {
            b"si" => {
                let s = std::mem::take(&mut self.current);
                let n = s.chars().count();
                if self.chars + n > self.max_chars {
                    self.cut = true;
                    self.chars = self.max_chars;
                    self.strings.push(String::new());
                } else {
                    self.chars += n;
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
        // A string past the cap is dropped whole, so stop growing it.
        if self.in_text > 0 && self.in_phonetic == 0 && self.chars + self.current.len() <= self.max_chars {
            self.current.push_str(text);
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

/// styles.xml: the number format of each cell style (cellXfs).
#[derive(Default)]
struct Styles {
    formats: HashMap<u32, String>,
    xfs: Vec<u32>,
    in_cell_xfs: bool,
}

impl Styles {
    fn kinds(&self) -> Vec<Shown> {
        self.xfs
            .iter()
            .map(|id| match self.formats.get(id) {
                Some(code) => shown_by_code(code),
                None => shown_by_id(*id),
            })
            .collect()
    }
}

impl Handler for Styles {
    fn open(&mut self, name: &[u8], e: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        match name {
            b"numFmt" => {
                if let (Some(id), Some(code)) = (attr(e, b"numFmtId"), attr(e, b"formatCode")) {
                    if let Ok(id) = id.parse() {
                        self.formats.insert(id, code);
                    }
                }
            }
            b"cellXfs" => self.in_cell_xfs = true,
            b"xf" if self.in_cell_xfs => {
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
}

/// One sheet's cells as rows.
struct Sheet {
    shared: Vec<String>,
    dates: Vec<Shown>,
    date1904: bool,
    /// Cells looked at so far, over every sheet.
    cells: usize,
    max_cells: usize,
    row: Vec<String>,
    cell: Option<Cell>,
    in_value: bool,
    /// Inside an inline string's t.
    in_inline: u32,
    /// Inside a phonetic guide.
    skip: u32,
}

impl Sheet {
    fn shown(&self, cell: Cell) -> String {
        let value = cell.value;
        match cell.kind.as_str() {
            "s" => value.trim().parse::<usize>().ok().and_then(|i| self.shared.get(i)).cloned().unwrap_or_default(),
            "b" => match value.trim() {
                "1" => "TRUE".into(),
                "0" => "FALSE".into(),
                other => other.into(),
            },
            "inlineStr" | "str" | "e" | "d" => value,
            _ => {
                let shown = self.dates.get(cell.style).copied().unwrap_or(Shown::Number);
                match value.trim().parse::<f64>() {
                    Ok(serial) if shown != Shown::Number => {
                        show_date(serial, shown, self.date1904).unwrap_or_else(|| value.trim().to_string())
                    }
                    _ => value.trim().to_string(),
                }
            }
        }
    }
}

impl Handler for Sheet {
    fn open(&mut self, name: &[u8], e: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        match name {
            b"row" => self.row.clear(),
            b"c" => {
                self.cells += 1;
                if self.cells > self.max_cells {
                    return Ok(Flow::Full);
                }
                self.cell = Some(Cell {
                    kind: attr(e, b"t").unwrap_or_default(),
                    style: attr(e, b"s").and_then(|s| s.parse().ok()).unwrap_or(0),
                    value: String::new(),
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
                if let Some(cell) = self.cell.take() {
                    let shown = self.shown(cell);
                    let shown = shown.trim();
                    if !shown.is_empty() {
                        self.row.push(shown.replace(['\t', '\n', '\r'], " "));
                    }
                }
            }
            b"row" if !self.row.is_empty() => {
                let mut line = std::mem::take(&mut self.row).join("\t");
                line.push('\n');
                return sink.text(&line);
            }
            _ => {}
        }
        Ok(Flow::More)
    }
    fn text(&mut self, text: &str, _: &mut Sink) -> Result<Flow, ReadError> {
        if let Some(cell) = &mut self.cell {
            let inline = cell.kind == "inlineStr" && self.in_inline > 0 && self.skip == 0;
            if (self.in_value || inline) && cell.value.len() < 1_000_000 {
                cell.value.push_str(text);
            }
        }
        Ok(Flow::More)
    }
}
