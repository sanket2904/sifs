use super::draw::{self, Format, Meta};
use super::frame::{FactKey, MARKER};
use super::helper::{read_image, serve_and_exit, Send};
use crate::extract::readers::macos::supervise::{Taken, HELPER_SLACK};
use super::*;
use crate::extract::store::{self, ContentMatch};
use crate::extract::worker::run_until_empty;
use crate::extract::{Part, Registry, WorkerOptions};
use crate::index::crawl::scan;
use crate::index::db::Index;
use crate::index::ignore::Rules;
use crate::index::tests::eventually;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

const HELPER_TEST: &str = "extract::readers::macos::image::tests::helper";
/// How a test helper misbehaves, for every request.
const MISBEHAVE: &str = "SIFS_IMAGE_TEST";
/// A file each test helper adds its process id to as it starts.
const LOG: &str = "SIFS_IMAGE_TEST_LOG";

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
    serve_and_exit(idle, |request, send| misbehave(&how, first, request, send));
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
        "crash" => std::process::abort(),
        "crash after a fact" => {
            send(width())?;
            std::process::abort()
        }
        "idle" => helper::exit(helper::Exit::Idle),
        "idle in the first helper" if first => helper::exit(helper::Exit::Idle),
        "another id" => {
            send(width())?;
            send(Frame::Fact { id: id + 1, key: FactKey::Height, value: "2".into() })?;
            park()
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

fn helper_pid(image: &Image) -> Option<u32> {
    image.running.lock().unwrap().as_ref().map(|running| running.process.id())
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
        ("a frame after the end", "the image reader sent a frame after the end"),
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

/// What a request with id 1 takes from bytes, fed whole, a byte at a time
/// and in sevens: the same however the pipe splits them.
fn receive(bytes: &[u8], caps: Caps) -> (Option<Taken<Stop>>, String, Vec<(String, String)>) {
    let ways: Vec<_> = [bytes.len().max(1), 1, 7]
        .into_iter()
        .map(|size| {
            let mut intake = Intake::new(frame::decoder(caps.chars));
            let mut order = Order { id: 1, step: Step::Facts { seen: 0 } };
            let mut sink = Sink::new(caps.chars, Instant::now() + Duration::from_secs(60));
            let stop = bytes.chunks(size).find_map(|chunk| intake.take(chunk, caps.output, |f| order.frame(f, &mut sink)));
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
fn the_helper_ends_after_its_idle_time_and_the_next_image_starts_another() {
    let dir = Dir::new();
    let path = dir.text("i.png", Format::Png, &["Idle"], &Meta::default());
    let image = reader(Caps { idle: Duration::from_millis(300), ..Caps::default() }, None);
    assert_eq!(text(&read_path(&image, &path, Duration::from_secs(60)).unwrap().0), "Idle");
    let first = helper_pid(&image).unwrap();
    let mut status = None;
    eventually("the idle helper to end", || {
        status = image.running.lock().unwrap().as_mut().unwrap().process.try_wait().unwrap();
        status.is_some()
    });
    assert_eq!(status.unwrap().code(), Some(Exit::Idle as i32));
    assert_eq!(text(&read_path(&image, &path, Duration::from_secs(60)).unwrap().0), "Idle");
    assert_ne!(helper_pid(&image), Some(first));
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
fn a_helper_that_died_between_images_is_replaced() {
    let dir = Dir::new();
    let a = dir.text("a.png", Format::Png, &["Again"], &Meta::default());
    let image = Image::default();
    read_path(&image, &a, Duration::from_secs(60)).unwrap();
    let pid = helper_pid(&image).unwrap();
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    eventually("the helper to die", || image.running.lock().unwrap().as_mut().unwrap().process.try_wait().unwrap().is_some());
    assert_eq!(text(&read_path(&image, &a, Duration::from_secs(60)).unwrap().0), "Again");
    assert_ne!(helper_pid(&image), Some(pid));
}

#[test]
fn a_helper_keeping_too_much_memory_is_replaced_after_its_image() {
    let dir = Dir::new();
    let a = dir.text("a.png", Format::Png, &["Kept"], &Meta::default());
    let image = reader(Caps { keep: MB, ..Caps::default() }, None);
    assert_eq!(text(&read_path(&image, &a, Duration::from_secs(60)).unwrap().0), "Kept");
    assert_eq!(helper_pid(&image), None, "Vision alone takes more than 1 MB");
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
