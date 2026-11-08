use super::frame::{Frame, MARKER};
use crate::extract::{fact, platform, MAX_FACT_CHARS};
use objc2::rc::autoreleasepool;
use objc2::AllocAnyThread;
use objc2_foundation::{NSString, NSURL};
use objc2_pdf_kit::{PDFDocument, PDFDocumentAuthorAttribute, PDFDocumentTitleAttribute};
use std::ffi::OsString;
use std::io::Write;
use std::time::{Duration, Instant};

pub(super) const PATH: &str = "SIFS_PDF_HELPER";
pub(super) const PAGES: &str = "SIFS_PDF_PAGES";
pub(super) const CHARS: &str = "SIFS_PDF_CHARS";
pub(super) const MILLIS: &str = "SIFS_PDF_MILLIS";

pub(super) struct Request {
    pub path: OsString,
    pub pages: usize,
    pub chars: usize,
    pub millis: u64,
}

impl Request {
    fn from_env() -> Option<Request> {
        let number = |key| std::env::var(key).ok()?.parse().ok();
        Some(Request {
            path: std::env::var_os(PATH)?,
            pages: number(PAGES)?,
            chars: number(CHARS)?,
            millis: number(MILLIS)? as u64,
        })
    }
}

/// If this process was started as the helper, reads the PDF and exits;
/// otherwise returns at once.
pub fn serve_if_asked() {
    if std::env::var_os(PATH).is_none() {
        return;
    }
    platform::lower_thread_priority();
    serve_and_exit();
}

/// The helper's work, at the caller's priority: the tests' helper keeps
/// theirs, so it isn't starved while every core is busy.
pub(super) fn serve_and_exit() -> ! {
    let code = match Request::from_env() {
        Some(request) => {
            exit_when_sifs_closes_stdin_or_after(Instant::now() + Duration::from_millis(request.millis));
            let mut out = std::io::stdout().lock();
            match serve(&request, &mut out) {
                Ok(()) => 0,
                Err(_) => 1,
            }
        }
        None => 2,
    };
    // Straight out: nothing PDFKit or the test harness registered to run at
    // exit gets the chance to hang or print.
    unsafe { libc::_exit(code) }
}

pub(super) fn exit_when_sifs_closes_stdin_or_after(deadline: Instant) {
    std::thread::spawn(move || {
        platform::lower_thread_priority();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                unsafe { libc::_exit(3) }
            }
            let mut fd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
            let ms = left.as_millis().min(1000) as libc::c_int;
            if unsafe { libc::poll(&mut fd, 1, ms) } > 0 {
                let mut byte = 0u8;
                if unsafe { libc::read(0, (&mut byte as *mut u8).cast(), 1) } <= 0 {
                    unsafe { libc::_exit(4) }
                }
            }
        }
    });
}

pub(super) fn serve(request: &Request, out: &mut impl Write) -> std::io::Result<()> {
    out.write_all(MARKER)?;
    let mut send = |frame: Frame| -> std::io::Result<()> {
        out.write_all(&frame.encode())?;
        out.flush()
    };
    autoreleasepool(|_| {
        let path = NSString::from_str(&request.path.to_string_lossy());
        let url = NSURL::fileURLWithPath(&path);
        let Some(doc) = (unsafe { PDFDocument::initWithURL(PDFDocument::alloc(), &url) }) else {
            return send(Frame::Unopenable);
        };
        if unsafe { doc.isLocked() } {
            return send(Frame::Locked);
        }
        let count = unsafe { doc.pageCount() };
        send(Frame::Pages(count.min(u32::MAX as usize) as u32))?;
        for (key, value) in attributes(&doc) {
            send(Frame::Fact(key.to_string(), value))?;
        }
        let mut left = request.chars;
        let mut cut = count > request.pages;
        for index in 0..count.min(request.pages) {
            let text = autoreleasepool(|_| {
                let page = unsafe { doc.pageAtIndex(index) }?;
                unsafe { page.string() }.map(|s| s.to_string())
            });
            let text = text.unwrap_or_default();
            if text.trim().is_empty() {
                send(Frame::Textless(index as u32))?;
                continue;
            }
            let kept = match text.char_indices().nth(left) {
                Some((at, _)) => &text[..at],
                None => &text,
            };
            left -= kept.chars().count();
            let whole = kept.len() == text.len();
            send(Frame::Page(index as u32, kept.to_string()))?;
            if left == 0 {
                cut |= !whole || index + 1 < count;
                break;
            }
        }
        if cut {
            send(Frame::Cut)?;
        }
        send(Frame::End)
    })
}

fn attributes(doc: &PDFDocument) -> Vec<(&'static str, String)> {
    let Some(attributes) = (unsafe { doc.documentAttributes() }) else {
        return Vec::new();
    };
    let keys = unsafe { [(fact::TITLE, PDFDocumentTitleAttribute), (fact::AUTHOR, PDFDocumentAuthorAttribute)] };
    keys.into_iter()
        .filter_map(|(key, attribute)| {
            let value = attributes.objectForKey(attribute)?;
            let value = value.downcast::<NSString>().ok()?.to_string();
            Some((key, value.chars().take(MAX_FACT_CHARS).collect()))
        })
        .collect()
}
