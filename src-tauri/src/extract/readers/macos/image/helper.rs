//! The helper's side: it reads requests from its stdin, one image at a
//! time, and sends each image's facts, text and end on its stdout. It ends
//! when its stdin closes, after a while with no request, or a second past a
//! request's deadline should SIFS not have killed it.

use super::facts::{self, Properties};
use super::frame::{requests, Frame, Outcome, Request, MARKER};
use crate::extract::platform;
use crate::extract::readers::macos::supervise::HELPER_SLACK;
use objc2::rc::{autoreleasepool, Retained};
use objc2::AllocAnyThread;
use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CFURL};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSArray, NSDictionary};
use objc2_image_io::*;
use objc2_vision::{VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Set to the idle time in milliseconds, it makes SIFS's program the image
/// helper.
pub(super) const HELPER: &str = "SIFS_IMAGE_HELPER";

/// A side under this many pixels holds no letter: no OCR.
pub(super) const SMALL: f64 = 8.0;

/// Vision reads a copy at most this many pixels on its longest side.
const MAX_SIDE: i64 = 4096;

/// How the helper ends, as its exit code. A crash ends it on a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Exit {
    Output = 1,
    BadRequest = 2,
    Late = 3,
    StdinClosed = 4,
    Idle = 5,
}

pub(super) fn exit(how: Exit) -> ! {
    // Straight out: nothing ImageIO, Vision or the test harness registered
    // to run at exit gets the chance to hang or print.
    unsafe { libc::_exit(how as i32) }
}

/// The idle time the helper was started with, if it was started as one.
pub(super) fn idle_from_env() -> Option<Duration> {
    let millis = std::env::var_os(HELPER)?;
    let millis = millis.to_str().and_then(|m| m.parse().ok()).unwrap_or(30_000);
    Some(Duration::from_millis(millis))
}

/// If this process was started as the image helper, serves images and
/// exits; otherwise returns at once.
pub fn serve_if_asked() {
    let Some(idle) = idle_from_env() else {
        return;
    };
    platform::lower_thread_priority();
    serve_and_exit(idle, read_image);
}

/// Where a read sends its frames.
pub(super) type Send<'a> = dyn FnMut(Frame) -> std::io::Result<()> + 'a;

/// Serves requests with read, at the caller's priority: the tests' helper
/// keeps theirs, so it isn't starved while every core is busy.
pub(super) fn serve_and_exit(idle: Duration, mut read: impl FnMut(&Request, &mut Send) -> std::io::Result<()>) -> ! {
    let deadline = Arc::new(Deadline::new());
    let requests = requests_from_stdin(deadline.clone());
    let mut out = std::io::stdout().lock();
    if out.write_all(MARKER).and_then(|_| out.flush()).is_err() {
        exit(Exit::Output);
    }
    loop {
        let request = match requests.recv_timeout(idle) {
            Ok(request) => request,
            Err(RecvTimeoutError::Timeout) => exit(Exit::Idle),
            Err(RecvTimeoutError::Disconnected) => exit(Exit::StdinClosed),
        };
        let mut send = |frame: Frame| -> std::io::Result<()> {
            out.write_all(&frame.encode())?;
            out.flush()
        };
        if read(&request, &mut send).is_err() {
            exit(Exit::Output);
        }
        deadline.clear();
    }
}

/// When the request being read must be done by, a second past SIFS's own
/// deadline. Shared with the thread that watches stdin.
struct Deadline {
    start: Instant,
    /// Milliseconds from start; u64::MAX when no request is being read.
    at: AtomicU64,
}

impl Deadline {
    fn new() -> Deadline {
        Deadline { start: Instant::now(), at: AtomicU64::new(u64::MAX) }
    }

    fn set(&self, left: Duration) {
        let at = (self.start.elapsed() + left + HELPER_SLACK).as_millis();
        self.at.store(at.min(u64::MAX as u128 - 1) as u64, Ordering::SeqCst);
    }

    fn clear(&self) {
        self.at.store(u64::MAX, Ordering::SeqCst);
    }

    /// The time left; None with no request.
    fn left(&self) -> Option<Duration> {
        let at = self.at.load(Ordering::SeqCst);
        (at != u64::MAX).then(|| Duration::from_millis(at).saturating_sub(self.start.elapsed()))
    }
}

/// Requests as they come on stdin. The thread reading them ends the helper
/// when stdin closes, or when a request runs past its deadline.
fn requests_from_stdin(deadline: Arc<Deadline>) -> Receiver<Request> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut decoder = requests();
        let mut buf = [0u8; 4096];
        loop {
            let left = deadline.left();
            if left == Some(Duration::ZERO) {
                exit(Exit::Late);
            }
            let ms = left.unwrap_or(Duration::from_secs(1)).as_millis().clamp(1, 1000) as libc::c_int;
            let mut fd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
            if unsafe { libc::poll(&mut fd, 1, ms) } <= 0 {
                continue;
            }
            let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
            if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            if n <= 0 {
                exit(Exit::StdinClosed);
            }
            decoder.push(&buf[..n as usize]);
            loop {
                match decoder.next_frame() {
                    Ok(Some(request)) => {
                        deadline.set(Duration::from_millis(request.deadline_ms));
                        let _ = tx.send(request);
                    }
                    Ok(None) => break,
                    Err(_) => exit(Exit::BadRequest),
                }
            }
        }
    });
    rx
}

/// Reads one image: its facts first, from its properties, then its text.
pub(super) fn read_image(request: &Request, send: &mut Send) -> std::io::Result<()> {
    let outcome = autoreleasepool(|_| read(request, send))?;
    send(Frame::End { id: request.id, outcome })
}

fn read(request: &Request, send: &mut Send) -> std::io::Result<Outcome> {
    let id = request.id;
    let Some(url) = CFURL::from_file_path(&request.path) else {
        return Ok(Outcome::CantOpen);
    };
    let no_cache = dictionary(&[(unsafe { kCGImageSourceShouldCache }, CFBoolean::new(false).as_ref())]);
    let Some(source) = (unsafe { CGImageSource::with_url(&url, Some(no_cache.as_opaque())) }) else {
        return Ok(Outcome::CantOpen);
    };
    if unsafe { source.count() } == 0 {
        return Ok(Outcome::CantOpen);
    }
    let index = unsafe { source.primary_image_index() };
    let Some(found) = (unsafe { source.properties_at_index(index, Some(no_cache.as_opaque())) }) else {
        return Ok(Outcome::CantOpen);
    };
    let properties = properties(&typed(found));
    for (key, value) in facts::facts(&properties) {
        send(Frame::Fact { id, key, value })?;
    }
    let (Some(width), Some(height)) = (properties.width, properties.height) else {
        return Ok(Outcome::CantOpen);
    };
    if width * height > request.max_pixels as f64 {
        return Ok(Outcome::TooManyPixels);
    }
    if width.min(height) < SMALL {
        return Ok(Outcome::Small);
    }
    let yes = CFBoolean::new(true).as_ref();
    let side = CFNumber::new_i64(MAX_SIDE);
    let options = dictionary(&[
        (unsafe { kCGImageSourceCreateThumbnailFromImageAlways }, yes),
        (unsafe { kCGImageSourceThumbnailMaxPixelSize }, side.as_ref()),
        (unsafe { kCGImageSourceCreateThumbnailWithTransform }, yes),
        (unsafe { kCGImageSourceShouldCacheImmediately }, yes),
    ]);
    let Some(image) = (unsafe { source.thumbnail_at_index(index, Some(options.as_opaque())) }) else {
        return Ok(Outcome::CantOpen);
    };
    let lines = match recognize(&image) {
        Ok(lines) => lines,
        Err(why) => return Ok(Outcome::RecognitionFailed(why)),
    };
    let text = lines.join("\n");
    let kept = match text.char_indices().nth(request.max_chars) {
        Some((at, _)) => &text[..at],
        None => &text[..],
    };
    if !kept.trim().is_empty() {
        send(Frame::Text { id, text: kept.to_string(), cut: kept.len() < text.len() })?;
    }
    Ok(Outcome::Read)
}

/// The lines Vision finds, in its order.
fn recognize(image: &CGImage) -> Result<Vec<String>, String> {
    let handler = unsafe { VNImageRequestHandler::initWithCGImage_options(VNImageRequestHandler::alloc(), image, &NSDictionary::new()) };
    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(true);
    // Vision picks the language; no list is fixed, so none is preferred.
    request.setAutomaticallyDetectsLanguage(true);
    let requests: Retained<NSArray<VNRequest>> =
        NSArray::from_retained_slice(&[Retained::into_super(Retained::into_super(request.clone()))]);
    handler.performRequests_error(&requests).map_err(|e| e.localizedDescription().to_string())?;
    let Some(results) = request.results() else {
        return Ok(Vec::new());
    };
    Ok(results
        .iter()
        .filter_map(|observation| {
            let best = observation.topCandidates(1).firstObject()?;
            Some(best.string().to_string())
        })
        .collect())
}

type Dictionary = CFDictionary<CFString, CFType>;

fn dictionary(pairs: &[(&CFString, &CFType)]) -> CFRetained<Dictionary> {
    let (keys, values): (Vec<&CFString>, Vec<&CFType>) = pairs.iter().copied().unzip();
    Dictionary::from_slices(&keys, &values)
}

fn typed(dictionary: CFRetained<CFDictionary>) -> CFRetained<Dictionary> {
    // ImageIO's property dictionaries are keyed by strings.
    unsafe { CFRetained::cast_unchecked(dictionary) }
}

fn properties(all: &Dictionary) -> Properties {
    let section = |key: &CFString| all.get(key)?.downcast::<CFDictionary>().ok().map(typed);
    let string = |d: &Dictionary, key: &CFString| Some(d.get(key)?.downcast::<CFString>().ok()?.to_string());
    let number = |d: &Dictionary, key: &CFString| d.get(key)?.downcast::<CFNumber>().ok()?.as_f64();
    let mut p = Properties {
        orientation: number(all, unsafe { kCGImagePropertyOrientation }),
        width: number(all, unsafe { kCGImagePropertyPixelWidth }),
        height: number(all, unsafe { kCGImagePropertyPixelHeight }),
        ..Properties::default()
    };
    if let Some(exif) = section(unsafe { kCGImagePropertyExifDictionary }) {
        p.original = string(&exif, unsafe { kCGImagePropertyExifDateTimeOriginal });
        p.original_offset = string(&exif, unsafe { kCGImagePropertyExifOffsetTimeOriginal });
        p.digitized = string(&exif, unsafe { kCGImagePropertyExifDateTimeDigitized });
        p.digitized_offset = string(&exif, unsafe { kCGImagePropertyExifOffsetTimeDigitized });
    }
    if let Some(tiff) = section(unsafe { kCGImagePropertyTIFFDictionary }) {
        p.make = string(&tiff, unsafe { kCGImagePropertyTIFFMake });
        p.model = string(&tiff, unsafe { kCGImagePropertyTIFFModel });
    }
    if let Some(gps) = section(unsafe { kCGImagePropertyGPSDictionary }) {
        p.latitude = number(&gps, unsafe { kCGImagePropertyGPSLatitude });
        p.latitude_ref = string(&gps, unsafe { kCGImagePropertyGPSLatitudeRef });
        p.longitude = number(&gps, unsafe { kCGImagePropertyGPSLongitude });
        p.longitude_ref = string(&gps, unsafe { kCGImagePropertyGPSLongitudeRef });
    }
    p
}
