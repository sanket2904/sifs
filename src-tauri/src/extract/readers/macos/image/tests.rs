use super::draw::{self, Format, Meta};
use super::frame::{FactKey, MARKER};
use super::helper::{read_image, serve_and_exit, Send};
use crate::extract::readers::macos::supervise::{Taken, HELPER_SLACK};
use super::*;
use crate::extract::store::{self, ContentMatch};
use crate::extract::worker::run_until_empty;
use crate::extract::{Part, Registry, Worker, WorkerOptions};
use crate::index::crawl::scan;
use crate::index::db::Index;
use crate::index::ignore::Rules;
use crate::index::tests::eventually;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{mpsc, Arc};

const HELPER_TEST: &str = "extract::readers::macos::image::tests::helper";
/// How a test helper misbehaves, for every request.
const MISBEHAVE: &str = "SIFS_IMAGE_TEST";
/// A file each test helper adds its process id to as it starts.
const LOG: &str = "SIFS_IMAGE_TEST_LOG";
/// A folder a test and its helper leave each other files in.
const SIGNALS: &str = "SIFS_IMAGE_TEST_SIGNALS";
/// Where a test helper splits what it sends in two.
const SPLIT: &str = "SIFS_IMAGE_TEST_SPLIT";

/// In a test helper, the requests it has been given so far, and the first's id.
static CALLS: AtomicUsize = AtomicUsize::new(0);
static FIRST_ID: AtomicU64 = AtomicU64::new(0);

/// An image's whole reply, text then end, then either a whole reply aimed
/// at the request after it, by the id that request would have were ids
/// counted, or bytes that aren't a frame; and where the first reply ends.
fn reply_then(id: u64, words: &str, frames_after: bool) -> (Vec<u8>, usize) {
    let next = id.wrapping_add(1);
    let mut bytes = Frame::Text { id, text: words.into(), cut: false }.encode();
    bytes.extend(Frame::End { id, outcome: Outcome::Read }.encode());
    let end = bytes.len();
    if frames_after {
        bytes.extend(Frame::Text { id: next, text: "stale".into(), cut: false }.encode());
        bytes.extend(Frame::End { id: next, outcome: Outcome::Read }.encode());
    } else {
        bytes.extend([b'?', 0, 0, 0, 0]);
    }
    (bytes, end)
}

/// Text that makes the first reply of reply_then 64 KiB: what one read of
/// the helper's output takes.
const ONE_READ: usize = 64 * 1024 - 28;

pub(super) fn helper_args() -> Vec<OsString> {
    ["--exact", HELPER_TEST, "--nocapture"].map(OsString::from).to_vec()
}

/// The image helper, when a test starts this binary as one; otherwise it
/// passes at once.
#[test]
fn helper() {
    let Some(idle) = helper::idle_from_env() else {
        return;
    };
    let mut first = true;
    if let Some(log) = std::env::var_os(LOG) {
        first = fs::read_to_string(&log).unwrap_or_default().is_empty();
        let mut file = fs::OpenOptions::new().create(true).append(true).open(&log).unwrap();
        writeln!(file, "{}", std::process::id()).unwrap();
    }
    let how = std::env::var(MISBEHAVE).unwrap_or_default();
    if how == "answer one, then stop reading" {
        answer_one_then_stop_reading();
    }
    serve_and_exit(idle, |request, send| misbehave(&how, first, request, send));
}

/// The folder in SIGNALS.
fn signals() -> PathBuf {
    std::env::var_os(SIGNALS).unwrap().into()
}

/// Waits for the test to leave the file name in signals().
fn wait_for(name: &str) {
    while !signals().join(name).exists() {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Answers its first request, then reads no more of its stdin; it leaves
/// "full" in signals() once what it hasn't read fills the pipe. It ends
/// itself after 10 s.
fn answer_one_then_stop_reading() -> ! {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(10));
        helper::exit(helper::Exit::Late)
    });
    let mut out = std::io::stdout().lock();
    out.write_all(MARKER).and_then(|_| out.flush()).unwrap();
    let mut requests = frame::requests();
    let mut buf = [0u8; 4096];
    let request = loop {
        if let Some(request) = requests.next_frame().unwrap() {
            break request;
        }
        let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
        assert!(n > 0, "stdin closed before a request");
        requests.push(&buf[..n as usize]);
    };
    out.write_all(&Frame::End { id: request.id, outcome: Outcome::Read }.encode()).and_then(|_| out.flush()).unwrap();
    loop {
        let mut stdin: libc::stat = unsafe { std::mem::zeroed() };
        unsafe { libc::fstat(0, &mut stdin) };
        if stdin.st_size > 0 && stdin.st_size >= stdin.st_blksize as libc::off_t {
            fs::write(signals().join("full"), "").unwrap();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn misbehave(how: &str, first: bool, request: &Request, send: &mut Send) -> std::io::Result<()> {
    let id = request.id;
    let width = || Frame::Fact { id, key: FactKey::Width, value: "1".into() };
    let text = || Frame::Text { id, text: "words".into(), cut: false };
    let raw = |bytes: &[u8]| -> std::io::Result<()> {
        let mut out = std::io::stdout().lock();
        out.write_all(bytes)?;
        out.flush()
    };
    match how {
        "spin" => {
            send(width())?;
            spin()
        }
        "allocate" => {
            send(width())?;
            allocate()
        }
        "sleep" => {
            send(width())?;
            park()
        }
        "flood" => {
            send(width())?;
            send(Frame::Text { id, text: "a".repeat(300_000), cut: false })?;
            send(Frame::End { id, outcome: Outcome::Read })
        }
        "close, then spin" | "close, then allocate" => {
            send(width())?;
            unsafe { libc::close(1) };
            if how == "close, then spin" {
                spin()
            } else {
                allocate()
            }
        }
        "busy" => {
            busy(Duration::from_millis(400));
            send(Frame::End { id, outcome: Outcome::Read })
        }
        "end, then spin" | "end, then allocate" | "end, then sleep" | "end, then exit" => {
            send(Frame::End { id, outcome: Outcome::Read })?;
            wait_for("go");
            match how {
                "end, then spin" => spin(),
                "end, then allocate" => allocate(),
                "end, then sleep" => park(),
                _ => helper::exit(helper::Exit::Output),
            }
        }
        // The next request comes while this one ends: its deadline must
        // hold all the same.
        "end as the next request comes" if id == 1 => {
            send(Frame::End { id, outcome: Outcome::Read })?;
            fs::write(signals().join("ended"), "")?;
            wait_for("sent");
            loop {
                let mut unread: libc::c_int = 0;
                unsafe { libc::ioctl(0, libc::FIONREAD, &mut unread) };
                if unread == 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            // Room for the next request to be taken off stdin before this
            // one ends.
            std::thread::sleep(Duration::from_millis(50));
            Ok(())
        }
        "end as the next request comes" => {
            fs::write(signals().join("second"), "")?;
            park()
        }
        "crash" => std::process::abort(),
        "crash after a fact" => {
            send(width())?;
            std::process::abort()
        }
        // Each helper uses 250 ms of processor time; the first then sends a
        // frame for another image, as a leftover would, and the second
        // answers.
        "busy, then a leftover in the first helper" => {
            busy(Duration::from_millis(250));
            if first {
                send(Frame::End { id: id.wrapping_add(1), outcome: Outcome::Read })?;
                park()
            }
            send(text())?;
            send(Frame::End { id, outcome: Outcome::Read })
        }
        // Each helper uses 250 ms of processor time on each of its threads;
        // the first then ends idle, and is reaped before its usage is read.
        "busy, then idle in the first helper" | "busy on four threads, then idle in the first helper" => {
            let threads = if how.contains("four") { 4 } else { 1 };
            std::thread::scope(|s| {
                for _ in 0..threads {
                    s.spawn(|| busy(Duration::from_millis(250)));
                }
            });
            if first {
                helper::exit(helper::Exit::Idle)
            }
            send(text())?;
            send(Frame::End { id, outcome: Outcome::Read })
        }
        "idle" => helper::exit(helper::Exit::Idle),
        "idle in the first helper" if first => helper::exit(helper::Exit::Idle),
        "another id" => {
            send(width())?;
            send(Frame::Fact { id: id.wrapping_add(1), key: FactKey::Height, value: "2".into() })?;
            park()
        }
        // The first helper sends, in one write, its reply and one aimed at
        // the next request; then it waits.
        "early, in one write" if first => {
            if CALLS.fetch_add(1, Ordering::SeqCst) > 0 {
                park()
            }
            raw(&reply_then(id, &"x".repeat(ONE_READ), true).0)
        }
        // The first helper sends its reply and what follows it in two
        // writes, split at SPLIT: both at once if the split is before its
        // own end, the second only once the next request has come if not.
        "early, split" | "garbage, split" if first => {
            let split: usize = std::env::var(SPLIT).unwrap().parse().unwrap();
            let frames_after = how == "early, split";
            if CALLS.fetch_add(1, Ordering::SeqCst) == 0 {
                FIRST_ID.store(id, Ordering::SeqCst);
                let (bytes, end) = reply_then(id, "first", frames_after);
                raw(&bytes[..split])?;
                return if split < end { raw(&bytes[split..]) } else { Ok(()) };
            }
            let (bytes, end) = reply_then(FIRST_ID.load(Ordering::SeqCst), "first", frames_after);
            if split >= end {
                raw(&bytes[split..])?;
            }
            park()
        }
        "an end, then bytes past the cap" => {
            if CALLS.fetch_add(1, Ordering::SeqCst) == 0 {
                return send(Frame::End { id, outcome: Outcome::Read });
            }
            raw(&[&width_then_end(id)[..], &[0; 128]].concat())?;
            park()
        }
        "early, in one write" | "early, split" | "garbage, split" => {
            send(Frame::Text { id, text: "second".into(), cut: false })?;
            send(Frame::End { id, outcome: Outcome::Read })
        }
        "a fact twice" => {
            send(width())?;
            send(width())?;
            park()
        }
        "text twice" => {
            send(width())?;
            send(text())?;
            send(text())?;
            park()
        }
        "a fact after text" => {
            send(text())?;
            send(width())?;
            park()
        }
        "an unknown frame" => {
            send(width())?;
            raw(&[b'?', 0, 0, 0, 0])?;
            park()
        }
        "a frame too long" => {
            send(width())?;
            let mut header = vec![b't'];
            header.extend((frame::max_frame(request.max_chars) as u32 + 1).to_le_bytes());
            raw(&header)?;
            park()
        }
        "a bad frame after a whole one" => {
            let mut bytes = width().encode();
            bytes.extend([b'?', 0, 0, 0, 0]);
            raw(&bytes)?;
            park()
        }
        "a frame after the end" => {
            let mut bytes = width().encode();
            bytes.extend(Frame::End { id, outcome: Outcome::Read }.encode());
            bytes.extend(Frame::Fact { id, key: FactKey::Height, value: "2".into() }.encode());
            raw(&bytes)?;
            park()
        }
        _ => read_image(request, send),
    }
}

fn spin() -> ! {
    loop {
        std::hint::black_box(());
    }
}

/// Uses this much of the processor, on this thread.
fn busy(time: Duration) {
    let used = || {
        let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) };
        Duration::new(now.tv_sec as u64, now.tv_nsec as u32)
    };
    let until = used() + time;
    while used() < until {
        std::hint::black_box(());
    }
}

fn park() -> ! {
    loop {
        std::thread::park();
    }
}

/// 400 MB, every page touched, then nothing more.
fn allocate() -> ! {
    let mut hog = Vec::new();
    for _ in 0..25 {
        let mut chunk = vec![0u8; 16 * MB as usize];
        chunk.iter_mut().step_by(4096).for_each(|b| *b = 1);
        hog.push(chunk);
    }
    std::hint::black_box(&hog);
    park()
}

type Got = Result<(Vec<Part>, Vec<(String, String)>), ReadError>;

fn reader(caps: Caps, misbehave: Option<&str>) -> Image {
    let mut image = Image { caps, ..Image::default() };
    if let Some(how) = misbehave {
        image.helper.env.push((MISBEHAVE.into(), how.into()));
    }
    image
}

fn read_path(image: &Image, path: &Path, time: Duration) -> Got {
    let mut sink = Sink::new(image.caps.chars, Instant::now() + time);
    image.read(path, &mut sink)?;
    Ok(sink.finish())
}

/// A read by a helper that misbehaves, whatever the image.
fn read_misbehaving(how: &str, caps: Caps, time: Duration) -> Got {
    read_path(&reader(caps, Some(how)), Path::new("/nonexistent/any.png"), time)
}

fn read(path: &Path) -> Got {
    read_path(&Image::default(), path, Duration::from_secs(60))
}

fn text(got: &[Part]) -> String {
    assert!(got.iter().all(|p| p.label.is_empty()), "one unlabelled part: {got:?}");
    got.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n")
}

fn fact<'a>(facts: &'a [(String, String)], key: &str) -> Option<&'a str> {
    facts.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn failed(got: Got) -> String {
    match got {
        Err(ReadError::Other(why)) => why,
        other => panic!("expected failed, got {other:?}"),
    }
}

fn gone(pid: u32) -> bool {
    let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
    !alive
}

/// The helpers started, in order, by the process ids they logged.
fn started(log: &Path) -> Vec<u32> {
    fs::read_to_string(log).unwrap_or_default().lines().map(|line| line.parse().unwrap()).collect()
}

fn helper_pid(image: &Image) -> Option<u32> {
    image.running.lock().running.as_ref().map(|running| running.process.id())
}

struct Dir(tempfile::TempDir);

impl Dir {
    fn new() -> Dir {
        Dir(tempfile::tempdir().unwrap())
    }

    fn text(&self, name: &str, format: Format, lines: &[&str], meta: &Meta) -> PathBuf {
        let path = self.0.path().join(name);
        draw::text(&path, format, (1200, 150 * lines.len().max(1)), lines, meta);
        path
    }

    fn sized(&self, name: &str, size: (usize, usize), lines: &[&str]) -> PathBuf {
        let path = self.0.path().join(name);
        draw::text(&path, Format::Png, size, lines, &Meta::default());
        path
    }
}

// --- reading ---

#[test]
fn a_screenshots_lines_are_one_part_in_order() {
    let dir = Dir::new();
    let path = dir.text("shot.png", Format::Png, &["Quarterly report", "Revenue grew everywhere", "Next meeting Monday"], &Meta::default());
    let (got, facts) = read(&path).unwrap();
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(text(&got), "Quarterly report\nRevenue grew everywhere\nNext meeting Monday");
    assert_eq!((fact(&facts, fact::WIDTH), fact(&facts, fact::HEIGHT)), (Some("1200"), Some("450")));
    assert_eq!(fact(&facts, fact::TRUNCATED), None);
    assert_eq!(fact(&facts, fact::STOPPED), None);
}

#[test]
fn jpeg_heic_tiff_and_a_gifs_first_frame_are_read() {
    let dir = Dir::new();
    for (name, format, word) in
        [("a.jpg", Format::Jpeg, "Lighthouse"), ("a.heic", Format::Heic, "Waterfall"), ("a.tiff", Format::Tiff, "Glacier")]
    {
        let path = dir.text(name, format, &[word], &Meta::default());
        assert_eq!(text(&read(&path).unwrap().0), word, "{name}");
    }
    let gif = dir.0.path().join("a.gif");
    draw::pages(&gif, Format::Gif, (1200, 150), &[&["Volcano"], &["Never read"]], &Meta::default());
    assert_eq!(text(&read(&gif).unwrap().0), "Volcano", "the first frame only");
}

#[test]
fn accents_and_other_scripts_are_read_next_to_english() {
    let dir = Dir::new();
    let path = dir.text("scripts.png", Format::Png, &["Résumé café naïve", "東京駅 新幹線", "Station"], &Meta::default());
    let got = text(&read(&path).unwrap().0);
    for words in ["Résumé café naïve", "東京駅", "Station"] {
        assert!(got.contains(words), "{words} in {got:?}");
    }
    let path = dir.text("more.png", Format::Png, &["Welcome", "مرحبا بكم", "欢迎光临"], &Meta::default());
    let got = text(&read(&path).unwrap().0);
    for words in ["Welcome", "مرحبا", "欢迎光临"] {
        assert!(got.contains(words), "{words} in {got:?}");
    }
}

#[test]
fn an_image_in_a_script_vision_doesnt_read_keeps_its_facts() {
    let dir = Dir::new();
    let meta = Meta::default().taken("2024:07:14 09:15:00", Some("+02:00"));
    let path = dir.text("devanagari.jpg", Format::Jpeg, &["नमस्ते दुनिया"], &meta);
    let (got, facts) = read(&path).unwrap();
    assert!(got.iter().all(|p| !p.text.contains("नमस्ते")), "{got:?}");
    assert_eq!(fact(&facts, fact::TAKEN), Some("2024-07-14T09:15:00+02:00"));
}

#[test]
fn a_photos_sign_is_read() {
    let dir = Dir::new();
    let path = dir.0.path().join("street.jpg");
    draw::photo(&path, Format::Jpeg, &["MAIN STREET", "EXIT 42"], &Meta::default().camera("Canon", "EOS R5"));
    let (got, facts) = read(&path).unwrap();
    let got = text(&got);
    assert!(got.contains("MAIN STREET") && got.contains("EXIT 42"), "{got:?}");
    assert_eq!(fact(&facts, fact::CAMERA_MODEL), Some("EOS R5"));
}

// --- facts ---

#[test]
fn when_a_photo_was_taken_is_a_fact() {
    let dir = Dir::new();
    let taken = |meta: Meta| {
        let path = dir.text("t.jpg", Format::Jpeg, &["x"], &meta);
        fact(&read(&path).unwrap().1, fact::TAKEN).map(str::to_string)
    };
    assert_eq!(taken(Meta::default().taken("2025:12:24 18:30:05", Some("-05:00"))).as_deref(), Some("2025-12-24T18:30:05-05:00"));
    assert_eq!(taken(Meta::default().taken("2025:06:01 07:00:00", None)).as_deref(), Some("2025-06-01T07:00:00"));
    let digitized = Meta::default().digitized("2023:03:09 22:41:10", Some("+09:00"));
    assert_eq!(taken(digitized).as_deref(), Some("2023-03-09T22:41:10+09:00"));
    assert_eq!(taken(Meta::default().taken("0000:00:00 00:00:00", None)), None);
    assert_eq!(taken(Meta::default().taken("2025:13:01 00:00:00", Some("+01:00"))), None);
}

#[test]
fn where_a_photo_was_taken_is_a_fact() {
    let dir = Dir::new();
    let place = |meta: Meta| {
        let path = dir.text("p.jpg", Format::Jpeg, &["x"], &meta);
        let facts = read(&path).unwrap().1;
        fact(&facts, fact::LATITUDE).map(|lat| (lat.to_string(), fact(&facts, fact::LONGITUDE).unwrap().to_string()))
    };
    let pair = |lat: &str, lon: &str| Some((lat.to_string(), lon.to_string()));
    assert_eq!(place(Meta::default().gps(33.8568, "S", 151.2153, "E")), pair("-33.856800", "151.215300"));
    assert_eq!(place(Meta::default().gps(22.9068, "S", 43.1729, "W")), pair("-22.906800", "-43.172900"));
    assert_eq!(place(Meta::default().gps(64.1466, "N", 21.9426, "W")), pair("64.146600", "-21.942600"));
    assert_eq!(place(Meta::default().gps(0.0, "N", 0.0, "E")), None, "a camera without a fix");
    assert_eq!(place(Meta::default().gps(95.0, "N", 10.0, "E")), None, "off the globe");
    // ImageIO itself reads an unknown reference ("X") as N or E.
    for (latitude, longitude) in [("W", "E"), ("N", "S"), ("W", "S")] {
        assert_eq!(place(Meta::default().gps(33.0, latitude, 151.0, longitude)), None, "{latitude} {longitude}");
    }
    assert_eq!(place(Meta::default().gps(35.0, "N", 139.0, "E").datum("WGS-84")), pair("35.000000", "139.000000"));
    assert_eq!(place(Meta::default().gps(35.0, "N", 139.0, "E").datum("TOKYO")), None, "another datum");
}

#[test]
fn camera_orientation_and_size_are_facts() {
    let dir = Dir::new();
    let meta = Meta::default().camera("Apple", "iPhone 15 Pro").orientation(6);
    let path = dir.0.path().join("c.heic");
    draw::text(&path, Format::Heic, (640, 480), &["x"], &meta);
    let (_, facts) = read(&path).unwrap();
    assert_eq!(fact(&facts, fact::CAMERA_MAKE), Some("Apple"));
    assert_eq!(fact(&facts, fact::CAMERA_MODEL), Some("iPhone 15 Pro"));
    assert_eq!(fact(&facts, fact::ORIENTATION), Some("6"));
    assert_eq!((fact(&facts, fact::WIDTH), fact(&facts, fact::HEIGHT)), (Some("640"), Some("480")), "as stored");
}

#[test]
fn a_photo_stored_upside_down_is_read() {
    let dir = Dir::new();
    let path = dir.0.path().join("turned.jpg");
    draw::upside_down(&path, &["Harbour view", "Ferry pier"], &Meta::default().orientation(3));
    assert_eq!(text(&read(&path).unwrap().0), "Harbour view\nFerry pier");
}

// --- limits ---

#[test]
fn a_picture_over_the_pixel_cap_keeps_its_facts_and_isnt_read() {
    let dir = Dir::new();
    let caps = Caps { pixels: 1_000_000, ..Caps::default() };
    let over = dir.sized("over.png", (1200, 900), &["Too many pixels"]);
    let (got, facts) = read_path(&reader(caps, None), &over, Duration::from_secs(60)).unwrap();
    assert!(got.is_empty(), "{got:?}");
    assert_eq!((fact(&facts, fact::WIDTH), fact(&facts, fact::HEIGHT)), (Some("1200"), Some("900")));
    assert_eq!(fact(&facts, fact::STOPPED), Some("larger than 1 megapixels"));
    let under = dir.sized("under.png", (1000, 1000), &["Just enough pixels"]);
    let (got, facts) = read_path(&reader(caps, None), &under, Duration::from_secs(60)).unwrap();
    assert_eq!(text(&got), "Just enough pixels");
    assert_eq!(fact(&facts, fact::STOPPED), None);
    assert_eq!(Caps::default().pixels, 200_000_000);
}

/// The frames the helper sends for one image, read in this process.
fn frames_for(path: &Path, max_chars: usize) -> Vec<Frame> {
    let request = Request { id: 7, path: path.into(), deadline_ms: 60_000, max_chars, max_pixels: 200_000_000 };
    let mut frames = Vec::new();
    read_image(&request, &mut |frame| {
        frames.push(frame);
        Ok(())
    })
    .unwrap();
    frames
}

#[test]
fn an_image_with_a_side_under_eight_pixels_isnt_read() {
    let dir = Dir::new();
    let end = |frames: Vec<Frame>| match frames.last() {
        Some(Frame::End { outcome, .. }) => outcome.clone(),
        other => panic!("no end: {other:?}"),
    };
    assert_eq!(end(frames_for(&dir.sized("thin.png", (7, 400), &["x"]), 100)), Outcome::Small);
    assert_eq!(end(frames_for(&dir.sized("flat.png", (400, 7), &["x"]), 100)), Outcome::Small);
    assert_eq!(end(frames_for(&dir.sized("eight.png", (8, 400), &["x"]), 100)), Outcome::Read);
    let (got, facts) = read(&dir.sized("line.png", (2, 400), &["x"])).unwrap();
    assert!(got.is_empty());
    assert_eq!(fact(&facts, fact::STOPPED), None, "nothing recorded as stopped");
    assert_eq!(fact(&facts, fact::WIDTH), Some("2"));
}

#[test]
fn text_is_cut_at_the_character_cap_and_marked_only_when_some_was_left_out() {
    let dir = Dir::new();
    let path = dir.text("long.png", Format::Png, &["Alpha bravo charlie", "Delta echo foxtrot"], &Meta::default());
    let text_of = |frames: &[Frame]| -> Option<(String, bool)> {
        frames.iter().find_map(|f| match f {
            Frame::Text { text, cut, .. } => Some((text.clone(), *cut)),
            _ => None,
        })
    };
    let (whole, cut) = text_of(&frames_for(&path, 100_000)).unwrap();
    assert_eq!((whole.as_str(), cut), ("Alpha bravo charlie\nDelta echo foxtrot", false));
    let chars = whole.chars().count();
    assert_eq!(text_of(&frames_for(&path, chars)), Some((whole.clone(), false)), "all of it fits");
    assert_eq!(text_of(&frames_for(&path, chars - 1)), Some((whole[..whole.len() - 1].to_string(), true)));
    assert_eq!(text_of(&frames_for(&path, 5)), Some(("Alpha".to_string(), true)));
    let caps = Caps { chars: 5, ..Caps::default() };
    let (got, facts) = read_path(&reader(caps, None), &path, Duration::from_secs(60)).unwrap();
    assert_eq!(text(&got), "Alpha");
    assert_eq!(fact(&facts, fact::TRUNCATED), Some("1"));
    assert_eq!(Image::default().limits().max_chars, 100_000);
}

#[test]
fn an_image_is_given_a_minute_by_the_clock_and_read_alone() {
    let limits = Image::default().limits();
    assert_eq!((limits.time, limits.max_bytes, limits.exclusive), (Duration::from_secs(60), 256 * MB, true));
    let caps = Caps::default();
    assert_eq!((caps.cpu, caps.memory, caps.output, caps.idle, caps.keep), (
        Duration::from_secs(10),
        1024 * MB,
        MB as usize,
        Duration::from_secs(30),
        512 * MB
    ));
}

#[test]
fn a_helper_past_its_processor_time_is_killed_and_the_facts_kept() {
    // Under 20 s by the clock: 1 s counted in Mach ticks, not nanoseconds,
    // would take 24 s of processor time.
    let caps = Caps { cpu: Duration::from_secs(1), ..Caps::default() };
    let (got, facts) = read_misbehaving("spin", caps, Duration::from_secs(20)).unwrap();
    assert!(got.is_empty());
    assert_eq!(fact(&facts, fact::WIDTH), Some("1"), "sent before the stop");
    assert_eq!(fact(&facts, fact::STOPPED), Some("used more than 1 s of processor time"));
    assert_eq!(fact(&facts, fact::TRUNCATED), None, "no text was cut");
}

#[test]
fn processor_time_is_counted_per_image() {
    let image = reader(Caps { cpu: Duration::from_secs(1), ..Caps::default() }, Some("busy"));
    let mut pids = Vec::new();
    for _ in 0..4 {
        let (_, facts) = read_path(&image, Path::new("/nonexistent/any.png"), Duration::from_secs(60)).unwrap();
        assert_eq!(fact(&facts, fact::STOPPED), None, "0.4 s for each image, 1.6 s in all");
        pids.push(helper_pid(&image).unwrap());
    }
    assert!(pids.iter().all(|&pid| pid == pids[0]), "one helper: {pids:?}");
}

#[test]
fn a_helper_past_its_memory_cap_is_killed_and_the_facts_kept() {
    let caps = Caps { memory: 200 * MB, ..Caps::default() };
    let (_, facts) = read_misbehaving("allocate", caps, Duration::from_secs(60)).unwrap();
    assert_eq!(fact(&facts, fact::WIDTH), Some("1"));
    assert_eq!(fact(&facts, fact::STOPPED), Some("used more than 200 MB of memory"));
}

#[test]
fn a_helper_past_the_deadline_is_killed_there_and_the_facts_kept() {
    let started = Instant::now();
    let (_, facts) = read_misbehaving("sleep", Caps::default(), Duration::from_secs(1)).unwrap();
    assert_eq!(fact(&facts, fact::WIDTH), Some("1"));
    assert_eq!(fact(&facts, fact::STOPPED), Some("took longer than 60 s"));
    let limit = Duration::from_secs(1) + HELPER_SLACK * 3 / 4;
    assert!(started.elapsed() < limit, "SIFS killed it, not its own deadline: {:?}", started.elapsed());
}

#[test]
fn a_helper_sending_too_much_is_stopped_and_the_facts_kept() {
    let caps = Caps { output: 100 * 1024, ..Caps::default() };
    let (got, facts) = read_misbehaving("flood", caps, Duration::from_secs(60)).unwrap();
    assert!(got.is_empty(), "the text past the cap is dropped");
    assert_eq!(fact(&facts, fact::WIDTH), Some("1"));
    assert_eq!(fact(&facts, fact::STOPPED), Some("sent more than 100 KB"));
}

// --- the helper's protocol ---

#[test]
fn a_helper_that_closes_its_output_is_killed_at_once() {
    // Its caps are far off, so only the end of its output can stop it in time.
    let caps = Caps { cpu: Duration::from_secs(30), ..Caps::default() };
    for how in ["close, then spin", "close, then allocate"] {
        let image = reader(caps, Some(how));
        let started = Instant::now();
        let (_, facts) = read_path(&image, Path::new("/nonexistent/any.png"), Duration::from_secs(30)).unwrap();
        assert!(started.elapsed() < Duration::from_secs(20), "{how}: killed, not waited for: {:?}", started.elapsed());
        assert_eq!(fact(&facts, fact::WIDTH), Some("1"), "{how}");
        assert_eq!(fact(&facts, fact::STOPPED), Some(CRASHED), "{how}");
        assert_eq!(helper_pid(&image), None, "{how}: replaced");
    }
}

#[test]
fn a_crash_keeps_the_facts_sent_before_it() {
    let (_, facts) = read_misbehaving("crash after a fact", Caps::default(), Duration::from_secs(60)).unwrap();
    assert_eq!(fact(&facts, fact::WIDTH), Some("1"));
    assert_eq!(fact(&facts, fact::STOPPED), Some(CRASHED));
    assert_eq!(failed(read_misbehaving("crash", Caps::default(), Duration::from_secs(60))), CRASHED);
}

#[test]
fn a_helper_breaking_the_protocol_is_stopped_and_the_frames_before_kept() {
    for (how, why) in [
        ("another id", "the image reader sent a frame for another image"),
        ("a fact twice", "the image reader sent a fact twice"),
        ("text twice", "the image reader sent the text twice"),
        ("an unknown frame", "the image reader sent an unknown frame (63)"),
        ("a bad frame after a whole one", "the image reader sent an unknown frame (63)"),
    ] {
        let image = reader(Caps::default(), Some(how));
        let (got, facts) = read_path(&image, Path::new("/nonexistent/any.png"), Duration::from_secs(60)).unwrap();
        assert_eq!(fact(&facts, fact::WIDTH), Some("1"), "{how}");
        assert_eq!(fact(&facts, fact::HEIGHT), None, "{how}");
        assert_eq!(fact(&facts, fact::STOPPED), Some(why), "{how}");
        assert_eq!(text(&got), if how == "text twice" { "words" } else { "" }, "{how}");
        assert_eq!(helper_pid(&image), None, "{how}: replaced");
    }
    let (got, facts) = read_misbehaving("a fact after text", Caps::default(), Duration::from_secs(60)).unwrap();
    assert_eq!(text(&got), "words");
    assert_eq!(fact(&facts, fact::WIDTH), None);
    assert_eq!(fact(&facts, fact::STOPPED), Some("the image reader sent a fact after the text"));
    let (_, facts) = read_misbehaving("a frame too long", Caps::default(), Duration::from_secs(60)).unwrap();
    let why = format!("the image reader sent a frame too long to be one ({} bytes)", frame::max_frame(100_000) + 1);
    assert_eq!(fact(&facts, fact::STOPPED), Some(why.as_str()));
}

/// The bytes of a width fact and an end, whatever their id.
fn width_then_end(id: u64) -> Vec<u8> {
    let mut bytes = Frame::Fact { id, key: FactKey::Width, value: "1".into() }.encode();
    bytes.extend(Frame::End { id, outcome: Outcome::Read }.encode());
    bytes
}

#[test]
fn an_image_keeps_its_end_and_a_helper_sending_past_it_reads_no_more() {
    let image = reader(Caps::default(), Some("a frame after the end"));
    let (got, facts) = read_path(&image, Path::new("/nonexistent/any.png"), Duration::from_secs(60)).unwrap();
    assert_eq!((text(&got), fact(&facts, fact::WIDTH), fact(&facts, fact::HEIGHT)), (String::new(), Some("1"), None));
    assert_eq!(fact(&facts, fact::STOPPED), None, "the image was read to its end");
    assert_eq!(helper_pid(&image), None, "replaced");

    // The second request's end at its output cap's last byte, more after it.
    let mut helper = Image::default().helper;
    helper.env.push((MISBEHAVE.into(), "an end, then bytes past the cap".into()));
    let mut running = Running::start(&helper, Caps::default()).unwrap();
    let mut read = |caps: Caps| {
        running.request(Path::new("/nonexistent/any.png"), &mut Sink::new(100, Instant::now() + Duration::from_secs(60)), caps, caps.cpu)
    };
    assert_eq!(read(Caps::default()).stop, Stop::End(Outcome::Read));
    let at_the_cap = Caps { output: width_then_end(0).len(), ..Caps::default() };
    assert_eq!(read(at_the_cap).stop, Stop::End(Outcome::Read));
    assert!(running.must_go(Caps::default()), "it reads no more images");
}

/// What a request with id 1 takes from bytes, fed whole, a byte at a time
/// and in sevens: the same however the pipe splits them.
fn receive(bytes: &[u8], caps: Caps) -> (Option<Taken<Stop>>, String, Vec<(String, String)>) {
    let ways: Vec<_> = [bytes.len().max(1), 1, 7]
        .into_iter()
        .map(|size| {
            let mut intake = Intake::new(frame::decoder(caps.chars));
            let mut order = Order::new(1);
            let mut sink = Sink::new(caps.chars, Instant::now() + Duration::from_secs(60));
            let stop = bytes.chunks(size).find_map(|chunk| order.take(&mut intake, chunk, caps.output, &mut sink));
            let (got, facts) = sink.finish();
            (stop, got.iter().map(|p| p.text.clone()).collect::<String>(), facts)
        })
        .collect();
    assert!(ways.iter().all(|way| *way == ways[0]), "the same in every split: {ways:?}");
    ways.into_iter().next().unwrap()
}

fn wire(frames: &[Frame]) -> Vec<u8> {
    let mut bytes = b"\nrunning 1 test\n".to_vec();
    bytes.extend_from_slice(MARKER);
    for frame in frames {
        bytes.extend(frame.encode());
    }
    bytes
}

#[test]
fn frames_are_taken_only_in_the_order_the_protocol_allows() {
    let fact_ = |key, value: &str| Frame::Fact { id: 1, key, value: value.into() };
    let text_ = |text: &str| Frame::Text { id: 1, text: text.into(), cut: false };
    let end = Frame::End { id: 1, outcome: Outcome::Read };
    let broken = |how: &str| Some(Taken::Stop(Stop::Broken(format!("the image reader sent {how}"))));
    let valid = [fact_(FactKey::Taken, "2025-01-01T00:00:00"), fact_(FactKey::Width, "9"), text_("seen"), end.clone()];
    let (stop, got, facts) = receive(&wire(&valid), Caps::default());
    assert_eq!(stop, Some(Taken::Stop(Stop::End(Outcome::Read))));
    assert_eq!(got, "seen");
    assert_eq!(facts, [("taken".to_string(), "2025-01-01T00:00:00".to_string()), ("width".to_string(), "9".to_string())]);
    for (name, frames, stop, kept) in [
        ("another id", vec![fact_(FactKey::Width, "9"), Frame::Fact { id: 2, key: FactKey::Height, value: "1".into() }], broken("a frame for another image"), ""),
        ("a fact twice", vec![fact_(FactKey::Width, "9"), fact_(FactKey::Width, "10")], broken("a fact twice"), ""),
        ("text twice", vec![text_("one"), text_("two")], broken("the text twice"), "one"),
        ("a fact after text", vec![text_("one"), fact_(FactKey::Width, "9")], broken("a fact after the text"), "one"),
        ("an end for another id", vec![Frame::End { id: 0, outcome: Outcome::Read }], broken("a frame for another image"), ""),
    ] {
        let (got_stop, got, _) = receive(&wire(&frames), Caps::default());
        assert_eq!(got_stop, stop, "{name}");
        assert_eq!(got, kept, "{name}");
    }
    let (stop, _, facts) = receive(&wire(&[fact_(FactKey::Width, "9"), Frame::End { id: 1, outcome: Outcome::CantOpen }]), Caps::default());
    assert_eq!(stop, Some(Taken::Stop(Stop::End(Outcome::CantOpen))));
    assert_eq!(facts.len(), 1);
}

#[test]
fn whole_frames_are_taken_before_a_bad_frame_or_the_output_cap() {
    let width = Frame::Fact { id: 1, key: FactKey::Width, value: "9".into() };
    for (bad, why) in [
        (vec![b'?', 0, 0, 0, 0], "an unknown frame (63)"),
        ([&[b'f', 9, 0, 0, 0][..], &1u64.to_le_bytes(), &[8]].concat(), "an unknown fact (8)"),
        ([&[b't', 9, 0, 0, 0][..], &1u64.to_le_bytes(), &[2]].concat(), "an unknown cut (2)"),
        ([&[b'e', 9, 0, 0, 0][..], &1u64.to_le_bytes(), &[9]].concat(), "an unknown outcome (9)"),
        (vec![b'e', 3, 0, 0, 0, 1, 2, 3], "a short frame"),
        ([&b"t"[..], &(frame::max_frame(100_000) as u32 + 1).to_le_bytes()].concat(), "a frame too long to be one (400065 bytes)"),
    ] {
        let mut bytes = wire(std::slice::from_ref(&width));
        bytes.extend(&bad);
        let (stop, _, facts) = receive(&bytes, Caps::default());
        assert_eq!(stop, Some(Taken::Bad(why.into())), "{why}");
        assert_eq!(facts, [("width".to_string(), "9".to_string())], "{why}");
    }
    let mut padded = wire(std::slice::from_ref(&width));
    let cap = padded.len();
    padded.extend([b't', 0, 4, 0, 0]);
    padded.extend(vec![b'x'; 1024]);
    for output in [cap, cap + 10] {
        let (stop, _, facts) = receive(&padded, Caps { output, ..Caps::default() });
        assert_eq!(stop, Some(Taken::Output), "cap {output}");
        assert_eq!(facts, [("width".to_string(), "9".to_string())], "cap {output}");
    }
}

#[test]
fn text_the_sink_refuses_is_not_received() {
    let mut order = Order::new(1);
    let mut sink = Sink::new(100, Instant::now() - Duration::from_millis(1));
    let refused = order.frame(Frame::Text { id: 1, text: "past the deadline".into(), cut: false }, &mut sink);
    assert_eq!(refused, Some(Stop::Time));
    assert!(sink.finish().0.is_empty());
    assert!(!order.received(), "so the image fails, not an empty success");
}

#[test]
fn text_the_sink_keeps_none_of_is_not_received() {
    let words = || Frame::Text { id: 1, text: "words".into(), cut: false };
    for (chars, received) in [(0, false), (1, true)] {
        let mut order = Order::new(1);
        let mut sink = Sink::new(chars, Instant::now() + Duration::from_secs(60));
        assert_eq!(order.frame(words(), &mut sink), None);
        assert_eq!(order.received(), received, "a text cap of {chars}");
    }
}

/// What a read got: its text and why it stopped, if it did; or its error.
type Words = Result<(String, Option<String>), String>;

/// Two images on one reader whose first helper misbehaves as how says:
/// what each got, the helpers started, and whether the first is gone.
fn two_images(how: &str, env: &[(&str, String)]) -> (Words, Words, usize, bool) {
    let dir = Dir::new();
    let log = dir.0.path().join("helpers.log");
    let mut image = reader(Caps::default(), Some(how));
    image.helper.env.push((LOG.into(), log.clone().into()));
    image.helper.env.extend(env.iter().map(|(k, v)| (OsString::from(k), OsString::from(v))));
    let words = |got: Got| match got {
        Ok((got, facts)) => Ok((text(&got), fact(&facts, fact::STOPPED).map(String::from))),
        Err(e) => Err(format!("{e:?}")),
    };
    let any = Path::new("/nonexistent/any.png");
    let first = words(read_path(&image, any, Duration::from_secs(10)));
    let second = words(read_path(&image, any, Duration::from_secs(10)));
    let helpers = started(&log);
    (first, second, helpers.len(), helpers.first().is_some_and(|pid| gone(*pid)))
}

#[test]
fn a_reply_sent_with_the_last_images_is_never_the_next_images() {
    let (first, second, helpers, first_gone) = two_images("early, in one write", &[]);
    assert_eq!(first, Ok(("x".repeat(ONE_READ), None)), "the first image's own reply");
    assert_eq!(second, Ok(("second".into(), None)), "the next image's own words, not the reply sent before it");
    assert_eq!(helpers, 2, "a new helper read the next image");
    assert!(first_gone, "the helper that sent it is gone");
}

#[test]
fn a_reply_sent_early_is_never_the_next_images_wherever_the_pipe_splits_it() {
    let right = (Ok(("first".to_string(), None)), Ok(("second".to_string(), None)), 2, true);
    for (how, frames_after) in [("early, split", true), ("garbage, split", false)] {
        let (bytes, _) = reply_then(0, "first", frames_after);
        let wrong: Vec<_> = (0..=bytes.len())
            .map(|split| (split, two_images(how, &[(SPLIT, split.to_string())])))
            .filter(|(_, got)| *got != right)
            .collect();
        assert!(wrong.is_empty(), "{how}: the same at every split of the {} bytes: {wrong:?}", bytes.len());
    }
}

#[test]
fn a_fact_at_its_cap_fits_in_a_frame_whatever_the_text_cap() {
    let longest = Frame::Fact { id: 1, key: FactKey::CameraModel, value: "😀".repeat(crate::extract::MAX_FACT_CHARS) };
    for chars in [0, 5, 100_000] {
        let mut decoder = frame::decoder(chars);
        decoder.push(&wire(std::slice::from_ref(&longest)));
        assert_eq!(decoder.next_frame(), Ok(Some(longest.clone())), "text cap {chars}");
    }
}

#[test]
fn a_helper_that_ends_idle_is_replaced_once_and_only_once() {
    let dir = Dir::new();
    let path = dir.text("i.png", Format::Png, &["Second helper"], &Meta::default());
    for (how, ok) in [("idle in the first helper", true), ("idle", false)] {
        let log = dir.0.path().join(format!("{how}.log"));
        let mut image = reader(Caps::default(), Some(how));
        image.helper.env.push((LOG.into(), log.clone().into()));
        let got = read_path(&image, &path, Duration::from_secs(60));
        match got {
            Ok((got, _)) if ok => assert_eq!(text(&got), "Second helper"),
            got if !ok => assert_eq!(failed(got), CRASHED),
            got => panic!("{how}: {got:?}"),
        }
        assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 2, "{how}: two helpers started");
    }
}

#[test]
fn a_helper_replaced_after_a_leftover_has_only_the_images_remaining_processor_time() {
    let dir = Dir::new();
    let log = dir.0.path().join("pids");
    let caps = Caps { cpu: Duration::from_millis(400), ..Caps::default() };
    let mut image = reader(caps, Some("busy, then a leftover in the first helper"));
    image.helper.env.push((LOG.into(), log.clone().into()));
    let got = read_path(&image, Path::new("/nonexistent/any.png"), Duration::from_secs(60));
    assert_eq!(failed(got), "used more than 400 ms of processor time", "250 ms each, 500 ms in all");
    let pids = started(&log);
    assert_eq!(pids.len(), 2, "replaced once");
    assert!(pids.iter().all(|&pid| gone(pid)));
}

#[test]
fn a_helper_that_ends_idle_after_using_processor_time_leaves_the_retry_only_what_is_left() {
    let dir = Dir::new();
    let log = dir.0.path().join("pids");
    let caps = Caps { cpu: Duration::from_millis(400), ..Caps::default() };
    let mut image = reader(caps, Some("busy, then idle in the first helper"));
    image.helper.env.push((LOG.into(), log.clone().into()));
    let got = read_path(&image, Path::new("/nonexistent/any.png"), Duration::from_secs(60));
    assert_eq!(failed(got), "used more than 400 ms of processor time", "250 ms each, 500 ms in all");
    assert!(started(&log).iter().all(|&pid| gone(pid)));
}

#[test]
fn a_helper_gone_before_its_usage_is_read_is_charged_for_every_processor() {
    let dir = Dir::new();
    let log = dir.0.path().join("pids");
    let caps = Caps { cpu: Duration::from_millis(1500), ..Caps::default() };
    let mut image = reader(caps, Some("busy on four threads, then idle in the first helper"));
    image.helper.env.push((LOG.into(), log.clone().into()));
    let got = read_path(&image, Path::new("/nonexistent/any.png"), Duration::from_secs(60));
    assert_eq!(failed(got), "used more than 1500 ms of processor time", "1 s each, 2 s in all");
    assert!(started(&log).iter().all(|&pid| gone(pid)));
}

#[test]
fn an_image_with_no_processor_time_left_starts_no_helper() {
    // A helper that can't start: trying to start one fails the request.
    let mut image = Image::default();
    image.helper.program = "/nonexistent/sifs".into();
    let mut sink = Sink::new(image.caps.chars, Instant::now() + Duration::from_secs(60));
    let read = image.request(&mut image.running.lock(), Path::new("/nonexistent/any.png"), &mut sink, Duration::ZERO);
    assert_eq!(read.unwrap().stop, Stop::Cpu);
}

#[test]
fn the_helper_is_ended_at_its_idle_time_and_the_next_image_starts_another() {
    let dir = Dir::new();
    let path = dir.text("i.png", Format::Png, &["Idle"], &Meta::default());
    let image = reader(Caps { idle: Duration::from_millis(300), ..Caps::default() }, None);
    assert_eq!(text(&read_path(&image, &path, Duration::from_secs(60)).unwrap().0), "Idle");
    let first = helper_pid(&image).unwrap();
    eventually("the idle helper to be ended and reaped", || helper_pid(&image).is_none() && gone(first));
    assert_eq!(text(&read_path(&image, &path, Duration::from_secs(60)).unwrap().0), "Idle");
    assert_ne!(helper_pid(&image), Some(first));
}

/// A helper that sends its end and then, told to go on, misbehaves.
fn kept_then(how: &str, caps: Caps) -> (Image, u32, PathBuf, Dir) {
    let dir = Dir::new();
    let mut image = reader(caps, Some(how));
    image.helper.env.push((SIGNALS.into(), dir.0.path().into()));
    read_path(&image, Path::new("/nonexistent/any.png"), Duration::from_secs(300)).unwrap();
    let pid = helper_pid(&image).unwrap_or_else(|| panic!("{how}: kept after its end"));
    let go = dir.0.path().join("go");
    (image, pid, go, dir)
}

#[test]
fn a_helper_kept_between_images_is_watched_there() {
    // Its idle time is far off, so only what it does can end it in time.
    let far = Duration::from_secs(600);
    for (how, caps) in [
        ("end, then spin", Caps { cpu: Duration::from_millis(300), idle: far, ..Caps::default() }),
        ("end, then allocate", Caps { keep: 200 * MB, idle: far, ..Caps::default() }),
        ("end, then exit", Caps { idle: far, ..Caps::default() }),
    ] {
        let (image, pid, go, _dir) = kept_then(how, caps);
        fs::write(go, "").unwrap();
        eventually(how, || helper_pid(&image).is_none() && gone(pid));
    }
}

#[test]
fn sifs_ends_a_kept_helper_at_its_idle_time_whatever_the_helper_does() {
    // Its serving thread never returns, so its own idle time never comes.
    let (image, pid, go, _dir) = kept_then("end, then sleep", Caps { idle: Duration::from_millis(300), ..Caps::default() });
    fs::write(go, "").unwrap();
    eventually("the idle helper to be ended and reaped", || helper_pid(&image).is_none() && gone(pid));
}

#[test]
fn a_helper_that_doesnt_read_its_requests_is_killed_at_the_deadline() {
    let dir = Dir::new();
    let mut image = reader(Caps::default(), Some("answer one, then stop reading"));
    image.helper.env.push((SIGNALS.into(), dir.0.path().into()));
    read_path(&image, Path::new("/nonexistent/any.png"), Duration::from_secs(60)).expect("the first request is answered");
    let pid = helper_pid(&image).unwrap();
    // The helper can't answer a request it hasn't read (it can't know its
    // id), so only one too long for the pipe is left unread: 256 KB.
    let path = PathBuf::from(format!("/{}", "a".repeat(256 * 1024)));
    let deadline = Duration::from_millis(500);
    let started = Instant::now();
    assert_eq!(failed(read_path(&image, &path, deadline)), "took longer than 60 s");
    assert!(started.elapsed() < deadline + HELPER_SLACK * 3 / 4, "killed at the deadline: {:?}", started.elapsed());
    assert!(dir.0.path().join("full").exists(), "the helper saw its request pipe full");
    assert_eq!(helper_pid(&image), None);
    assert!(gone(pid));
}

#[test]
fn the_end_of_one_request_doesnt_clear_the_next_ones_deadline() {
    let dir = Dir::new();
    let mut helper = Image::default().helper;
    helper.env.push((MISBEHAVE.into(), "end as the next request comes".into()));
    helper.env.push((SIGNALS.into(), dir.0.path().into()));
    let mut running = Running::start(&helper, Caps::default()).unwrap();
    let request = |id, deadline_ms| Request { id, path: "/nonexistent/any.png".into(), deadline_ms, max_chars: 10, max_pixels: 10 };
    let mut stdin = running.process.stdin.take().unwrap();
    stdin.write_all(&request(1, 60_000).encode()).unwrap();
    eventually("the first request to end", || dir.0.path().join("ended").exists());
    stdin.write_all(&request(2, 0).encode()).unwrap();
    fs::write(dir.0.path().join("sent"), "").unwrap();
    eventually("the second request to start", || dir.0.path().join("second").exists());
    assert_eq!(exit_code(&mut running), Some(Exit::Late as i32), "ended a second past the second request's deadline");
}

#[test]
fn stopping_the_worker_ends_the_helper_whoever_holds_the_reader() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap().join("home");
    fs::create_dir_all(&root).unwrap();
    draw::text(&root.join("tiny.png"), Format::Png, (2, 16), &[], &Meta::default());
    let log = dir.path().join("helpers.log");
    let mut image = Image::default();
    image.helper.env.push((LOG.into(), log.clone().into()));
    let image = Arc::new(image);
    let registry = Arc::new(Registry::with(vec![image.clone()]));
    let db = dir.path().join("sifs.db");
    let mut index = Index::open(&db).unwrap();
    store::register_kinds(&mut index, &registry).unwrap();
    let root_str = root.to_str().unwrap();
    index.add_folder(root_str, 0).unwrap();
    scan(&mut index, root_str, &Rules::new(&[], root_str).unwrap(), |_| {}).unwrap();
    let options = WorkerOptions { idle_poll: Duration::from_millis(10), ..WorkerOptions::default() };
    let worker = Worker::start(db, registry.clone(), options).unwrap();
    let read = || index.conn.query_row("SELECT count(*) FROM extracts WHERE status = 'empty'", [], |r| r.get::<_, i64>(0));
    eventually("the image to be read", || read().unwrap() == 1);
    let pid = helper_pid(&image).unwrap();
    assert_eq!(worker.stop().reads, 1);
    assert!(gone(pid), "killed and reaped as the worker stopped, with the reader still held");
    for _ in 0..2 {
        assert_eq!(failed(read_path(&image, &root.join("tiny.png"), Duration::from_secs(60))), STOPPED_READER);
    }
    registry.stop();
    assert_eq!(helper_pid(&image), None);
    assert_eq!(started(&log), [pid], "no helper started after the stop");
}

/// The image reader, through a reader that holds each read at its start
/// until told to go on, and panics the worker as it asks for the limits a
/// third time: just after the worker dispatched a read.
struct Paused {
    image: Arc<Image>,
    limits_asked: AtomicUsize,
    entered: mpsc::Sender<()>,
    go_on: Mutex<mpsc::Receiver<()>>,
    done: mpsc::Sender<Result<(), String>>,
}

impl Reader for Paused {
    fn id(&self) -> &'static str {
        self.image.id()
    }

    fn version(&self) -> u32 {
        self.image.version()
    }

    fn exts(&self) -> &'static [&'static str] {
        self.image.exts()
    }

    fn class(&self) -> Class {
        self.image.class()
    }

    fn limits(&self) -> Limits {
        if self.limits_asked.fetch_add(1, Ordering::SeqCst) == 2 {
            panic!("the worker panics just after it dispatched a read");
        }
        self.image.limits()
    }

    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError> {
        self.entered.send(()).unwrap();
        self.go_on.lock().unwrap().recv().unwrap();
        let got = self.image.read(path, sink);
        self.done.send(got.as_ref().map(drop).map_err(|e| format!("{e:?}"))).unwrap();
        got
    }

    fn stop(&self) {
        self.image.stop();
    }
}

#[test]
fn a_read_dispatched_as_the_worker_panics_starts_no_helper() {
    let dir = Dir::new();
    let tiny = dir.sized("tiny.png", (2, 16), &[]);
    let root = fs::canonicalize(dir.0.path()).unwrap().join("indexed");
    fs::create_dir_all(&root).unwrap();
    fs::copy(&tiny, root.join("image.png")).unwrap();
    let log = dir.0.path().join("helpers.log");
    let mut image = Image::default();
    image.helper.env.push((LOG.into(), log.clone().into()));
    let image = Arc::new(image);
    read_path(&image, &tiny, Duration::from_secs(60)).unwrap();
    let before = helper_pid(&image).unwrap();
    let ((entered, wait_entered), (go_on, wait_go_on), (done, wait_done)) = (mpsc::channel(), mpsc::channel(), mpsc::channel());
    let paused = Arc::new(Paused { image: image.clone(), limits_asked: AtomicUsize::new(0), entered, go_on: Mutex::new(wait_go_on), done });
    let registry = Arc::new(Registry::with(vec![paused.clone()]));
    let db = dir.0.path().join("sifs.db");
    let mut index = Index::open(&db).unwrap();
    store::register_kinds(&mut index, &registry).unwrap();
    let root = root.to_str().unwrap();
    index.add_folder(root, 0).unwrap();
    scan(&mut index, root, &Rules::new(&[], root).unwrap(), |_| {}).unwrap();
    let worker = Worker::start(db, registry, WorkerOptions { concurrency: 2, ..WorkerOptions::default() }).unwrap();
    wait_entered.recv_timeout(Duration::from_secs(10)).unwrap();
    eventually("the worker to panic", || paused.limits_asked.load(Ordering::SeqCst) >= 3);
    drop(worker);
    assert!(gone(before), "the helper is reaped as the worker's thread ends");
    go_on.send(()).unwrap();
    let got = wait_done.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(got, Err(format!("{:?}", ReadError::Other(STOPPED_READER.into()))), "the read is refused");
    assert_eq!(helper_pid(&image), None);
    assert_eq!(started(&log), [before], "no helper started after the worker ended");
}

#[test]
fn a_read_racing_a_stop_ends_before_the_stop_reaps_its_helper_or_is_refused() {
    let dir = Dir::new();
    let tiny = dir.sized("tiny.png", (2, 16), &[]);
    let (mut reaped, mut refused) = (0, 0);
    for round in 0..400 {
        if reaped >= 20 && refused >= 20 {
            break;
        }
        let log = dir.0.path().join(format!("{round}.log"));
        let mut image = Image::default();
        image.helper.env.push((LOG.into(), log.clone().into()));
        let image = Arc::new(image);
        let registry = Registry::with(vec![image.clone()]);
        // Both spin until told to go, then one spins a while longer: the
        // read on even rounds, the stop on odd ones, longer each round.
        let go = AtomicBool::new(false);
        let head_start = |which| if round % 2 == which { round * 200 } else { 0 };
        let wait_then = |spins: usize| {
            while !go.load(Ordering::SeqCst) {
                std::hint::spin_loop();
            }
            (0..spins).for_each(|_| std::hint::spin_loop());
        };
        let got = std::thread::scope(|scope| {
            let read = scope.spawn(|| {
                wait_then(head_start(0));
                read_path(&image, &tiny, Duration::from_secs(60))
            });
            let stop = scope.spawn(|| {
                wait_then(head_start(1));
                registry.stop();
            });
            go.store(true, Ordering::SeqCst);
            stop.join().unwrap();
            read.join().unwrap()
        });
        let helpers = started(&log);
        match got {
            Ok(_) => {
                reaped += 1;
                assert_eq!(helpers.len(), 1, "round {round}: one helper for the read");
            }
            Err(ReadError::Other(why)) if why == STOPPED_READER => {
                refused += 1;
                assert!(helpers.is_empty(), "round {round}: none started for a refused read");
            }
            got => panic!("round {round}: {got:?}"),
        }
        assert_eq!(helper_pid(&image), None, "round {round}: nothing kept after the stop");
        assert!(helpers.iter().all(|pid| gone(*pid)), "round {round}: every helper started is reaped");
    }
    assert!(reaped >= 20 && refused >= 20, "both orders seen: {reaped} reaped, {refused} refused");
}

#[test]
fn a_worker_that_cant_open_the_index_still_ends_the_helper() {
    let dir = Dir::new();
    let tiny = dir.sized("tiny.png", (2, 16), &[]);
    let image = Arc::new(Image::default());
    read_path(&image, &tiny, Duration::from_secs(60)).unwrap();
    let pid = helper_pid(&image).unwrap();
    let file = dir.0.path().join("a file");
    fs::write(&file, "").unwrap();
    let registry = Arc::new(Registry::with(vec![image.clone()]));
    let worker = Worker::start(file.join("sifs.db"), registry, WorkerOptions::default()).unwrap();
    assert_eq!(worker.stop().errors.len(), 1, "an index beneath a file can't open");
    assert!(gone(pid), "killed and reaped as the worker stopped, with the reader still held");
}

#[test]
fn one_helper_serves_many_images_and_is_replaced_after_a_stop() {
    let dir = Dir::new();
    let a = dir.text("a.png", Format::Png, &["Apple"], &Meta::default());
    let b = dir.text("b.png", Format::Png, &["Banana"], &Meta::default());
    let image = Image::default();
    assert_eq!(text(&read_path(&image, &a, Duration::from_secs(60)).unwrap().0), "Apple");
    let pid = helper_pid(&image).unwrap();
    assert_eq!(text(&read_path(&image, &b, Duration::from_secs(60)).unwrap().0), "Banana");
    assert_eq!(helper_pid(&image), Some(pid), "the same helper");
    assert_eq!(failed(read_path(&image, &a, Duration::ZERO)), "took longer than 60 s");
    assert_eq!(helper_pid(&image), None);
    assert!(gone(pid), "the stopped helper is killed and reaped");
    assert_eq!(text(&read_path(&image, &b, Duration::from_secs(60)).unwrap().0), "Banana");
    assert_ne!(helper_pid(&image), Some(pid));
}

#[test]
fn a_helper_that_died_just_before_an_image_is_replaced() {
    let dir = Dir::new();
    let a = dir.text("a.png", Format::Png, &["Again"], &Meta::default());
    let image = Image::default();
    read_path(&image, &a, Duration::from_secs(60)).unwrap();
    // Held, so its watcher can't see it die first.
    let mut slot = image.running.lock();
    let pid = slot.running.as_ref().unwrap().process.id();
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    eventually("the helper to die", || slot.running.as_mut().unwrap().process.try_wait().unwrap().is_some());
    let mut sink = Sink::new(image.caps.chars, Instant::now() + Duration::from_secs(60));
    assert_eq!(image.request(&mut slot, &a, &mut sink, image.caps.cpu).unwrap().stop, Stop::End(Outcome::Read));
    assert_eq!(text(&sink.finish().0), "Again");
    assert_ne!(slot.running.as_ref().map(|running| running.process.id()), Some(pid));
}

#[test]
fn a_helper_keeping_too_much_memory_is_replaced_after_its_image() {
    let dir = Dir::new();
    let a = dir.text("a.png", Format::Png, &["Kept"], &Meta::default());
    let image = reader(Caps { keep: MB, ..Caps::default() }, None);
    // Held, so its watcher can't replace it first.
    let mut slot = image.running.lock();
    let mut sink = Sink::new(image.caps.chars, Instant::now() + Duration::from_secs(60));
    assert_eq!(image.request(&mut slot, &a, &mut sink, image.caps.cpu).unwrap().stop, Stop::End(Outcome::Read));
    assert_eq!(text(&sink.finish().0), "Kept");
    assert!(slot.running.is_none(), "Vision alone takes more than 1 MB");
}

#[test]
fn no_helper_is_left_when_the_reader_goes() {
    let dir = Dir::new();
    let a = dir.text("a.png", Format::Png, &["Gone"], &Meta::default());
    let image = Image::default();
    read_path(&image, &a, Duration::from_secs(60)).unwrap();
    let pid = helper_pid(&image).unwrap();
    assert!(!gone(pid));
    // Stopped, it can't end itself when its stdin closes: only a kill ends it.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGSTOP) };
    drop(image);
    assert!(gone(pid), "killed and reaped");
}

fn sleeping_helper() -> Running {
    let mut helper = Image::default().helper;
    helper.env.push((MISBEHAVE.into(), "sleep".into()));
    Running::start(&helper, Caps::default()).unwrap()
}

fn exit_code(running: &mut Running) -> Option<i32> {
    let mut status = None;
    eventually("the helper to end", || {
        status = running.process.try_wait().unwrap();
        status.is_some()
    });
    status.unwrap().code()
}

#[test]
fn the_helper_ends_when_sifs_goes_away() {
    let mut running = sleeping_helper();
    drop(running.process.stdin.take());
    assert_eq!(exit_code(&mut running), Some(Exit::StdinClosed as i32));
}

#[test]
fn the_helper_ends_itself_a_second_after_its_deadline() {
    let mut running = sleeping_helper();
    let request = Request { id: 1, path: "/nonexistent/any.png".into(), deadline_ms: 0, max_chars: 10, max_pixels: 10 };
    running.process.stdin.as_mut().unwrap().write_all(&request.encode()).unwrap();
    assert_eq!(exit_code(&mut running), Some(Exit::Late as i32), "ended at its own deadline, SIFS still there");
}

#[test]
fn a_file_that_isnt_an_image_is_failed() {
    let dir = Dir::new();
    let path = dir.0.path().join("fake.png");
    fs::write(&path, b"<html>not an image</html>").unwrap();
    assert_eq!(failed(read(&path)), DAMAGED);
    fs::write(&path, b"").unwrap();
    assert_eq!(failed(read(&path)), DAMAGED);
    let whole = fs::read(dir.text("cut.png", Format::Png, &["Cut short"], &Meta::default())).unwrap();
    fs::write(&path, &whole[..64]).unwrap();
    assert_eq!(failed(read(&path)), DAMAGED);
}

// --- through the worker ---

#[test]
fn the_platform_registry_reads_images_on_macos() {
    let registry = Registry::platform();
    for ext in ["png", "heic", "jpg", "gif", "webp", "tiff", "cr3", "dng"] {
        assert_eq!(registry.get(ext).map(|r| r.id()), Some("image"), "{ext}");
    }
    for ext in ["svg", "ico", "icns", "exr"] {
        assert!(registry.get(ext).is_none_or(|r| r.id() != "image"), "{ext}");
    }
    assert_eq!(Image::default().class(), Class::Image);
}

#[test]
fn images_are_read_by_the_worker_and_found_by_their_words() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap().join("home");
    let at = |name: &str| root.join(name).to_str().unwrap().to_string();
    fs::create_dir_all(&root).unwrap();
    draw::text(Path::new(&at("receipt.png")), Format::Png, (1200, 300), &["Harbour Cafe", "Total 18.40"], &Meta::default());
    let meta = Meta::default().taken("2025:08:02 16:20:00", Some("-03:00")).gps(22.9068, "S", 43.1729, "W");
    draw::photo(Path::new(&at("beach.jpg")), Format::Jpeg, &["COPACABANA"], &meta);
    fs::write(at("damaged.png"), b"\x89PNG\r\n\x1a\n not really").unwrap();
    fs::File::create(at("huge.png")).unwrap().set_len(MAX_BYTES + 1).unwrap();
    let registry = Registry::with(vec![Arc::new(Image::default())]);
    let mut index = Index::open(&dir.path().join("sifs.db")).unwrap();
    store::register_kinds(&mut index, &registry).unwrap();
    let root_str = root.to_str().unwrap();
    index.add_folder(root_str, 0).unwrap();
    scan(&mut index, root_str, &Rules::new(&Rules::default_patterns(), root_str).unwrap(), |_| {}).unwrap();
    run_until_empty(&mut index, &registry, &WorkerOptions::default()).unwrap();
    for (word, file) in [("harbour", "receipt.png"), ("copacabana", "beach.jpg")] {
        let found: Vec<ContentMatch> = store::match_contents(&index, word, 10).unwrap();
        assert_eq!(found.iter().map(|m| m.path.as_str()).collect::<Vec<_>>(), [at(file).as_str()], "{word}");
    }
    let status = |name: &str| -> (String, Option<String>) {
        index
            .conn
            .query_row(
                "SELECT e.status, e.error FROM extracts e JOIN files f ON f.id = e.file_id WHERE f.path = ?1",
                [at(name)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    assert_eq!(status("damaged.png"), ("failed".into(), Some(DAMAGED.into())));
    assert_eq!(status("huge.png"), ("skipped".into(), Some("larger than 256 MB".into())));
    let facts: Vec<(String, String)> = index
        .conn
        .prepare("SELECT key, value FROM facts WHERE file_id = (SELECT id FROM files WHERE path = ?1) ORDER BY key")
        .unwrap()
        .query_map([at("beach.jpg")], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (key, value) in [("taken", "2025-08-02T16:20:00-03:00"), ("latitude", "-22.906800"), ("longitude", "-43.172900")] {
        assert!(facts.contains(&(key.to_string(), value.to_string())), "{key} in {facts:?}");
    }
}

#[test]
fn a_word_in_an_image_is_found_by_the_search_over_names_and_contents() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap().join("home");
    fs::create_dir_all(&root).unwrap();
    let receipt = root.join("receipt.png");
    draw::text(&receipt, Format::Png, (1200, 300), &["Harbour Cafe", "Total 18.40"], &Meta::default());
    let registry = Registry::with(vec![Arc::new(Image::default())]);
    let mut index = Index::open(&dir.path().join("sifs.db")).unwrap();
    store::register_kinds(&mut index, &registry).unwrap();
    let root_str = root.to_str().unwrap();
    index.add_folder(root_str, 0).unwrap();
    scan(&mut index, root_str, &Rules::new(&Rules::default_patterns(), root_str).unwrap(), |_| {}).unwrap();
    run_until_empty(&mut index, &registry, &WorkerOptions::default()).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let got = crate::extract::find::search_all(&index, "harbour", 10, now, &crate::index::stop::Until::after(Duration::from_secs(10))).unwrap();
    let hits: Vec<(&str, bool)> = got.hits.iter().map(|hit| (hit.file.path.as_str(), hit.found.is_some())).collect();
    assert_eq!(hits, [(receipt.to_str().unwrap(), true)], "found by its contents");
}
