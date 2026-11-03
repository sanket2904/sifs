//! Word documents: the body, by page where the file marks pages, then the
//! headers and footers, footnotes, endnotes and comments.

use super::{limits, Caps, Handler, Kind, Package, Prose, Root};
use crate::extract::{Flow, Limits, ReadError, Reader, Sink};
use quick_xml::events::BytesStart;
use std::collections::HashSet;
use std::path::Path;

/// Left out of Word text: what an mc:AlternateContent repeats for older
/// readers, and text a tracked change deleted or moved away.
const SKIPPED: &[&[u8]] = &[b"Fallback", b"del", b"moveFrom"];

#[derive(Default)]
pub struct Word {
    pub caps: Caps,
}

impl Reader for Word {
    fn id(&self) -> &'static str {
        "docx"
    }
    fn version(&self) -> u32 {
        1
    }
    fn exts(&self) -> &'static [&'static str] {
        &["docx", "docm", "dotx", "dotm"]
    }
    fn limits(&self) -> Limits {
        limits()
    }
    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError> {
        let mut package = Package::open(path, Kind::Word, self.caps)?;
        let main = package.main_part(sink)?;
        sink.part("");
        let mut body = Body { prose: Prose::new(SKIPPED, true), root: Root(None) };
        let flow = package.parse(&main, &mut body, sink)?;
        package.check_root(body.root.get())?;
        if flow == Some(Flow::Full) || body.prose.finish(sink)? == Flow::Full {
            return Ok(());
        }
        let rels = package.rels(&main, sink)?;
        let mut sections: Vec<(&str, Vec<&str>)> = vec![
            ("headers and footers", Vec::new()),
            ("footnotes", Vec::new()),
            ("endnotes", Vec::new()),
            ("comments", Vec::new()),
        ];
        // Each part once, however many relationships name it.
        let mut seen = HashSet::new();
        for rel in &rels {
            if !seen.insert(rel.target.as_str()) {
                continue;
            }
            let at = match rel.kind.as_str() {
                "header" | "footer" => 0,
                "footnotes" => 1,
                "endnotes" => 2,
                "comments" => 3,
                _ => continue,
            };
            sections[at].1.push(&rel.target);
        }
        // Headers before footers, each in the order of its file's name.
        sections[0].1.sort_by_key(|t| (t.contains("footer"), t.to_string()));
        for (label, targets) in sections {
            if targets.is_empty() {
                continue;
            }
            sink.part(label);
            for target in targets {
                sink.check()?;
                let mut prose = Prose::new(SKIPPED, false);
                if package.parse(target, &mut prose, sink)? == Some(Flow::Full) || prose.finish(sink)? == Flow::Full {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

/// The body: prose, noting the root element.
struct Body {
    prose: Prose,
    root: Root,
}

impl Handler for Body {
    fn open(&mut self, name: &[u8], e: &BytesStart, sink: &mut Sink) -> Result<Flow, ReadError> {
        self.root.see(name);
        if self.root.get() != Some(b"document") {
            return Ok(Flow::Full);
        }
        self.prose.open(name, e, sink)
    }
    fn close(&mut self, name: &[u8], sink: &mut Sink) -> Result<Flow, ReadError> {
        self.prose.close(name, sink)
    }
    fn text(&mut self, text: &str, sink: &mut Sink) -> Result<Flow, ReadError> {
        self.prose.text(text, sink)
    }
}
