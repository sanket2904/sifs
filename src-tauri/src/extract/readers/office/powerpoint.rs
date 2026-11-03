//! PowerPoint presentations: one part per slide, in the presentation's
//! order, each followed by its notes.

use super::{limits, rel_id, targets, Caps, Handler, Kind, Package, Prose, Root};
use crate::extract::{fact, Flow, Limits, ReadError, Reader, Sink};
use quick_xml::events::BytesStart;
use std::collections::HashSet;
use std::path::Path;

/// What an mc:AlternateContent repeats for older readers.
const SKIPPED: &[&[u8]] = &[b"Fallback"];

#[derive(Default)]
pub struct PowerPoint {
    pub caps: Caps,
}

impl Reader for PowerPoint {
    fn id(&self) -> &'static str {
        "pptx"
    }
    fn version(&self) -> u32 {
        1
    }
    fn exts(&self) -> &'static [&'static str] {
        &["pptx", "pptm", "potx", "potm", "ppsx", "ppsm"]
    }
    fn limits(&self) -> Limits {
        limits()
    }
    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError> {
        let mut package = Package::open(path, Kind::PowerPoint, self.caps)?;
        let main = package.main_part(sink)?;
        let mut presentation = Presentation { max: self.caps.listed, ..Presentation::default() };
        package.parse(&main, &mut presentation, sink)?;
        package.check_root(presentation.root.get())?;
        sink.fact(fact::SLIDES, presentation.total.to_string());
        if presentation.total > presentation.slides.len() {
            sink.note_truncated();
        }
        let rels = package.rels(&main, sink)?;
        let targets = targets(&rels);
        let mut read = HashSet::new();
        for (n, id) in presentation.slides.iter().enumerate() {
            sink.check()?;
            let Some(&slide) = targets.get(id.as_str()) else { continue };
            // A part named by two slides is read once.
            if !read.insert(slide) {
                continue;
            }
            sink.part(format!("slide {}", n + 1));
            let mut prose = Prose::new(SKIPPED, false);
            if package.parse(slide, &mut prose, sink)? == Some(Flow::Full) || prose.finish(sink)? == Flow::Full {
                return Ok(());
            }
            let notes = package.rels(slide, sink)?.into_iter().find(|r| r.kind == "notesSlide");
            if let Some(notes) = notes {
                sink.part(format!("slide {} notes", n + 1));
                let mut prose = Prose::new(SKIPPED, false);
                if package.parse(&notes.target, &mut prose, sink)? == Some(Flow::Full)
                    || prose.finish(sink)? == Flow::Full
                {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

/// presentation.xml: the slides in order, by relationship id, the first
/// max of them, and how many it lists.
#[derive(Default)]
struct Presentation {
    root: Root,
    slides: Vec<String>,
    total: usize,
    max: usize,
}

impl Handler for Presentation {
    fn open(&mut self, name: &[u8], e: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        self.root.see(name);
        if self.root.get() != Some(b"presentation") {
            return Ok(Flow::Full);
        }
        if name == b"sldId" {
            self.total += 1;
            if let Some(id) = rel_id(e).filter(|_| self.slides.len() < self.max) {
                self.slides.push(id);
            }
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
