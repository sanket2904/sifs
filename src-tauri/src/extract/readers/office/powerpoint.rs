//! PowerPoint presentations: one part per slide, in the presentation's
//! order, each followed by its notes.

use super::{limits, rel_id, Caps, Handler, Kind, Package, Prose, Root};
use crate::extract::{fact, Flow, Limits, ReadError, Reader, Sink};
use quick_xml::events::BytesStart;
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
        let mut presentation = Presentation::default();
        package.parse(&main, &mut presentation, sink)?;
        package.check_root(presentation.root.get())?;
        sink.fact(fact::SLIDES, presentation.slides.len().to_string());
        let rels = package.rels(&main, sink)?;
        for (n, id) in presentation.slides.iter().enumerate() {
            let Some(slide) = rels.iter().find(|r| &r.id == id).map(|r| r.target.clone()) else { continue };
            sink.part(format!("slide {}", n + 1));
            let mut prose = Prose::new(SKIPPED, false);
            if package.parse(&slide, &mut prose, sink)? == Some(Flow::Full) || prose.finish(sink)? == Flow::Full {
                return Ok(());
            }
            let notes = package.rels(&slide, sink)?.into_iter().find(|r| r.kind == "notesSlide");
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

/// presentation.xml: the slides in order, by relationship id.
#[derive(Default)]
struct Presentation {
    root: Root,
    slides: Vec<String>,
}

impl Handler for Presentation {
    fn open(&mut self, name: &[u8], e: &BytesStart, _: &mut Sink) -> Result<Flow, ReadError> {
        self.root.see(name);
        if self.root.get() != Some(b"presentation") {
            return Ok(Flow::Full);
        }
        if name == b"sldId" {
            if let Some(id) = rel_id(e) {
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
