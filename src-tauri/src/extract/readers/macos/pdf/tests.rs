use super::frame::{Decoder, Frame, MARKER};
use super::helper::{serve, Request};
use super::sample::{self, Page};
use super::*;
use crate::extract::store::{self, ContentMatch};
use crate::extract::worker::run_until_empty;
use crate::extract::{Part, Registry, WorkerOptions};
use crate::index::crawl::scan;
use crate::index::db::Index;
use crate::index::ignore::Rules;
use std::fs;
use std::sync::Arc;

const HELPER_TEST: &str = "extract::readers::macos::pdf::tests::helper";
const MISBEHAVE: &str = "SIFS_PDF_TEST";

pub(super) fn helper_args() -> Vec<OsString> {
    ["--exact", HELPER_TEST, "--ignored", "--nocapture"].map(OsString::from).to_vec()
}

#[test]
#[ignore = "run by the other tests as the PDF helper"]
fn helper() {
    if std::env::var_os(helper::PATH).is_none() {
        return;
    }
    let mut out = std::io::stdout().lock();
    let send = |out: &mut std::io::StdoutLock, frame: Frame| {
        out.write_all(&frame.encode()).unwrap();
        out.flush().unwrap();
    };
    use std::io::Write;
    match std::env::var(MISBEHAVE).as_deref() {
        Ok("flood") => {
            out.write_all(MARKER).unwrap();
            for index in 0..=u32::MAX {
                send(&mut out, Frame::Page(index, "flood ".repeat(1000)));
            }
        }
        Ok("crash") => {
            out.write_all(MARKER).unwrap();
            send(&mut out, Frame::Pages(2));
            send(&mut out, Frame::Page(0, "before the crash".into()));
            std::process::abort();
        }
        Ok("crash at once") => std::process::abort(),
        Ok("garbage") => {
            out.write_all(MARKER).unwrap();
            out.write_all(b"?\0\0\0\0").unwrap();
            out.flush().unwrap();
        }
        _ => helper::serve_and_exit(),
    }
}

type Read = Result<(Vec<Part>, Vec<(String, String)>), ReadError>;

fn reader(caps: Caps, misbehave: Option<&str>) -> Pdf {
    let mut helper = Helper::default();
    if let Some(how) = misbehave {
        helper.env.push((MISBEHAVE.into(), how.into()));
    }
    Pdf { caps, helper }
}

fn read_with(pdf: &Pdf, body: &[u8], time: Duration) -> Read {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.pdf");
    fs::write(&path, body).unwrap();
    read_path(pdf, &path, time)
}

fn read_path(pdf: &Pdf, path: &Path, time: Duration) -> Read {
    let mut sink = Sink::new(1_000_000, Instant::now() + time);
    pdf.read(path, &mut sink)?;
    Ok(sink.finish())
}

fn read(body: &[u8]) -> Read {
    read_with(&Pdf::default(), body, Duration::from_secs(30))
}

fn parts(parts: &[Part]) -> Vec<(&str, &str)> {
    parts.iter().map(|p| (p.label.as_str(), p.text.as_str())).collect()
}

fn fact<'a>(facts: &'a [(String, String)], key: &str) -> Option<&'a str> {
    facts.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn failed(read: Read) -> String {
    match read {
        Err(ReadError::Other(why)) => why,
        other => panic!("expected failed, got {other:?}"),
    }
}

/// Whether the process is gone (not even a zombie waiting to be reaped).
fn gone(pid: u32) -> bool {
    let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
    !alive
}

#[test]
fn registry_reads_pdfs_on_macos() {
    assert_eq!(Registry::platform().get("pdf").map(|r| r.id()), Some("pdf"));
}

#[test]
fn pages_are_read_in_order_and_labelled_by_their_index() {
    let (got, facts) = read(&sample::pdf(&[Page::Text("alpha first"), Page::Blank, Page::Text("charlie third")])).unwrap();
    assert_eq!(parts(&got), [("page 1", "alpha first"), ("page 3", "charlie third")]);
    assert_eq!(fact(&facts, fact::PAGES), Some("3"));
    assert_eq!(fact(&facts, fact::TEXTLESS_PAGES), Some("1"), "the blank page");
    assert_eq!(fact(&facts, fact::TRUNCATED), None);
}

#[test]
fn title_and_author_are_facts() {
    let info = "/Info << /Title (Lease agreement) /Author (A. Landlord) >>";
    let (_, facts) = read(&sample::pdf_with(&[Page::Text("rent")], info)).unwrap();
    assert_eq!(fact(&facts, fact::TITLE), Some("Lease agreement"));
    assert_eq!(fact(&facts, fact::AUTHOR), Some("A. Landlord"));
}

#[test]
fn a_pdf_that_needs_a_password_is_skipped_and_an_owner_password_alone_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("plain.pdf");
    fs::write(&plain, sample::pdf(&[Page::Helvetica("hidden words")])).unwrap();
    let (locked, owner_only) = (dir.path().join("locked.pdf"), dir.path().join("owner.pdf"));
    sample::encrypt(&plain, &locked, Some("secret"), "owner");
    sample::encrypt(&plain, &owner_only, None, "owner");
    let pdf = Pdf::default();
    match read_path(&pdf, &locked, Duration::from_secs(30)) {
        Err(ReadError::Unsupported(why)) => assert_eq!(why, PASSWORD),
        other => panic!("expected skipped, got {other:?}"),
    }
    let (got, _) = read_path(&pdf, &owner_only, Duration::from_secs(30)).unwrap();
    assert_eq!(parts(&got), [("page 1", "hidden words")]);
}

#[test]
fn pages_without_text_are_counted_for_ocr() {
    let (got, facts) = read(&sample::pdf(&[Page::Image, Page::Image])).unwrap();
    assert!(got.is_empty(), "nothing stored from a scan: {got:?}");
    assert_eq!((fact(&facts, fact::PAGES), fact(&facts, fact::TEXTLESS_PAGES)), (Some("2"), Some("2")));
    let (got, facts) = read(&sample::pdf(&[Page::Text("typed"), Page::Image, Page::Text("also typed")])).unwrap();
    assert_eq!(parts(&got), [("page 1", "typed"), ("page 3", "also typed")]);
    assert_eq!(fact(&facts, fact::TEXTLESS_PAGES), Some("1"));
}

#[test]
fn a_file_that_isnt_a_pdf_is_skipped_without_starting_the_helper() {
    let nowhere = Pdf {
        helper: Helper { program: "/nonexistent/sifs".into(), args: Vec::new(), env: Vec::new() },
        ..Pdf::default()
    };
    match read_with(&nowhere, b"PK\x03\x04 a zip named .pdf", Duration::from_secs(5)) {
        Err(ReadError::Unsupported(why)) => assert_eq!(why, NOT_A_PDF),
        other => panic!("expected skipped, got {other:?}"),
    }
    let (got, _) = read_with(&nowhere, b"", Duration::from_secs(5)).unwrap();
    assert!(got.is_empty(), "an empty file is empty");
    let why = failed(read_with(&nowhere, &sample::pdf(&[Page::Text("x")]), Duration::from_secs(5)));
    assert!(why.starts_with("couldn't start the PDF reader: "), "a real PDF does start it: {why}");
    // The header may follow up to 1,024 bytes of anything, as PDFKit allows.
    let mut late = vec![b' '; 1000];
    late.extend(sample::pdf(&[Page::Text("late header")]));
    assert_eq!(parts(&read(&late).unwrap().0), [("page 1", "late header")]);
}

#[test]
fn a_damaged_pdf_is_failed() {
    let whole = sample::pdf(&[Page::Text("cut short")]);
    assert_eq!(failed(read(&whole[..whole.len() / 3])), DAMAGED);
}

#[test]
fn a_page_past_the_time_limit_is_killed_at_the_deadline() {
    let pdf = Pdf::default();
    let started = Instant::now();
    match read_with(&pdf, &sample::pdf(&[Page::Hang]), Duration::from_secs(1)) {
        Err(ReadError::Timeout) => {}
        other => panic!("expected a timeout, got {other:?}"),
    }
    let limit = Duration::from_secs(1) + HELPER_SLACK * 3 / 4;
    assert!(started.elapsed() < limit, "SIFS killed it, not its own deadline: {:?}", started.elapsed());
}

#[test]
fn pages_read_before_a_stop_are_kept() {
    let body = sample::pdf(&[Page::Text("first"), Page::Text("second"), Page::Hang, Page::Text("never")]);
    let pdf = reader(Caps { cpu: Duration::from_secs(1), ..Caps::default() }, None);
    let (got, facts) = read_with(&pdf, &body, Duration::from_secs(60)).unwrap();
    assert_eq!(parts(&got), [("page 1", "first"), ("page 2", "second")]);
    assert_eq!(fact(&facts, fact::TRUNCATED), Some("1"));
    assert_eq!(fact(&facts, fact::STOPPED), Some("used more than 1 s of processor time, after page 2"));
}

#[test]
fn a_helper_past_its_memory_cap_is_killed() {
    let pdf = reader(Caps { memory: 200 * MB, cpu: Duration::from_secs(3), ..Caps::default() }, None);
    let why = failed(read_with(&pdf, &sample::pdf(&[Page::Hog]), Duration::from_secs(60)));
    assert_eq!(why, "used more than 200 MB of memory");
}

#[test]
fn a_pdf_is_given_a_minute_by_the_clock() {
    // The worker holds the read this long; processor time is what stops a
    // page that never ends, so a starved helper still finishes.
    assert_eq!(Pdf::default().limits().time, Duration::from_secs(60));
}

#[test]
fn a_helper_past_its_processor_time_is_killed() {
    let pdf = reader(Caps { cpu: Duration::from_secs(1), ..Caps::default() }, None);
    let why = failed(read_with(&pdf, &sample::pdf(&[Page::Hang]), Duration::from_secs(60)));
    assert_eq!(why, "used more than 1 s of processor time");
}

#[test]
fn pages_past_the_page_cap_are_left_out() {
    let body = sample::pdf(&[Page::Text("one"), Page::Text("two"), Page::Text("three")]);
    for (cap, cut) in [(3, false), (2, true)] {
        let pdf = reader(Caps { pages: cap, ..Caps::default() }, None);
        let (got, facts) = read_with(&pdf, &body, Duration::from_secs(30)).unwrap();
        let want = [("page 1", "one"), ("page 2", "two"), ("page 3", "three")];
        assert_eq!(parts(&got), want[..cap], "cap {cap}");
        assert_eq!(fact(&facts, fact::TRUNCATED).is_some(), cut, "cap {cap}");
    }
}

#[test]
fn the_helper_cuts_text_at_the_character_cap() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.pdf");
    fs::write(&path, sample::pdf(&[Page::Text("abcdé"), Page::Text("fghij"), Page::Text("never")])).unwrap();
    for (chars, want, cut) in [(10, vec!["abcdé", "fghij"], true), (7, vec!["abcdé", "fg"], true), (100, vec!["abcdé", "fghij", "never"], false)] {
        let mut out = Vec::new();
        serve(&Request { path: path.clone().into(), pages: 100, chars, millis: 0 }, &mut out).unwrap();
        let frames = Decoder::default().feed(&out).unwrap();
        let pages: Vec<&str> =
            frames.iter().filter_map(|f| if let Frame::Page(_, t) = f { Some(t.as_str()) } else { None }).collect();
        assert_eq!(pages, want, "{chars} characters");
        assert_eq!(frames.contains(&Frame::Cut), cut, "{chars} characters");
        assert_eq!(frames.last(), Some(&Frame::End));
    }
}

#[test]
fn a_helper_sending_too_much_is_stopped() {
    let pdf = reader(Caps { output: MB as usize, ..Caps::default() }, Some("flood"));
    let (got, facts) = read_with(&pdf, &sample::pdf(&[Page::Text("x")]), Duration::from_secs(20)).unwrap();
    assert!(!got.is_empty(), "the pages before the cap are kept");
    let stopped = fact(&facts, fact::STOPPED).unwrap();
    assert!(stopped.starts_with("sent more than 1 MB, after page "), "{stopped}");
}

#[test]
fn a_helper_that_crashes_keeps_what_it_sent() {
    let body = sample::pdf(&[Page::Text("x")]);
    let (got, facts) = read_with(&reader(Caps::default(), Some("crash")), &body, Duration::from_secs(20)).unwrap();
    assert_eq!(parts(&got), [("page 1", "before the crash")]);
    assert_eq!(fact(&facts, fact::STOPPED), Some("PDFKit crashed reading it, after page 1"));
    let at_once = reader(Caps::default(), Some("crash at once"));
    assert_eq!(failed(read_with(&at_once, &body, Duration::from_secs(20))), CRASHED);
    let garbage = reader(Caps::default(), Some("garbage"));
    assert_eq!(failed(read_with(&garbage, &body, Duration::from_secs(20))), "the PDF reader sent an unknown frame (63)");
}

#[test]
fn no_helper_is_left_running() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hang.pdf");
    fs::write(&path, sample::pdf(&[Page::Hang])).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut run = Run::start(&Helper::default(), &path, Caps::default(), 1000, deadline).unwrap();
    let pid = run.child.id();
    let mut sink = Sink::new(1000, deadline);
    assert_eq!(run.supervise(&mut sink, Caps::default()), Stop::Time);
    assert!(!gone(pid), "still running when the read stopped");
    drop(run);
    assert!(gone(pid), "killed and reaped");
}

#[test]
fn the_helper_ends_when_sifs_goes_away() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hang.pdf");
    fs::write(&path, sample::pdf(&[Page::Hang])).unwrap();
    let mut run = Run::start(&Helper::default(), &path, Caps::default(), 1000, Instant::now() + Duration::from_secs(120))
        .unwrap();
    drop(run.child.stdin.take());
    let mut status = None;
    crate::index::tests::eventually("the helper to end", || {
        status = run.child.try_wait().unwrap();
        status.is_some()
    });
    assert_eq!(status.unwrap().code(), Some(4), "ended because its stdin closed");
}

#[test]
fn the_helper_ends_itself_a_second_after_its_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hang.pdf");
    fs::write(&path, sample::pdf(&[Page::Hang])).unwrap();
    let mut run = Run::start(&Helper::default(), &path, Caps::default(), 1000, Instant::now()).unwrap();
    let mut status = None;
    crate::index::tests::eventually("the helper to end", || {
        status = run.child.try_wait().unwrap();
        status.is_some()
    });
    assert_eq!(status.unwrap().code(), Some(3), "ended at its own deadline, SIFS still there");
}

#[test]
fn the_decoder_skips_what_comes_before_the_marker_and_waits_for_whole_frames() {
    let mut bytes = b"\nrunning 1 test\n".to_vec();
    bytes.extend_from_slice(MARKER);
    bytes.extend(Frame::Pages(2).encode());
    bytes.extend(Frame::Page(1, "héllo".into()).encode());
    bytes.extend(Frame::End.encode());
    let mut decoder = Decoder::default();
    let mut frames = Vec::new();
    for b in &bytes {
        frames.extend(decoder.feed(std::slice::from_ref(b)).unwrap());
    }
    assert_eq!(frames, [Frame::Pages(2), Frame::Page(1, "héllo".into()), Frame::End]);
    let mut junk = MARKER.to_vec();
    junk.extend([b'?', 0, 0, 0, 0]);
    assert!(Decoder::default().feed(&junk).is_err(), "an unknown frame ends the output");
}

#[test]
fn pdfs_are_read_by_the_worker_and_found_by_their_words() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap().join("home");
    let at = |name: &str| root.join(name).to_str().unwrap().to_string();
    fs::create_dir_all(&root).unwrap();
    fs::write(at("lease.pdf"), sample::pdf(&[Page::Text("cover"), Page::Text("Résumé किराया 家賃 receipt ❤️ paid")])).unwrap();
    fs::write(at("scan.pdf"), sample::pdf(&[Page::Image])).unwrap();
    fs::write(at("fake.pdf"), b"<html>not a pdf</html>").unwrap();
    let mut big = sample::pdf(&[Page::Text("past four megabytes")]);
    big.extend(vec![b'%'; 5 * MB as usize]);
    fs::write(at("big.pdf"), big).unwrap();
    fs::File::create(at("huge.pdf")).unwrap().set_len(MAX_BYTES + 1).unwrap();
    let registry = Registry::with(vec![Arc::new(Pdf::default())]);
    let mut index = Index::open(&dir.path().join("sifs.db")).unwrap();
    store::register_kinds(&mut index, &registry).unwrap();
    let root_str = root.to_str().unwrap();
    index.add_folder(root_str, 0).unwrap();
    scan(&mut index, root_str, &Rules::new(&Rules::default_patterns(), root_str).unwrap(), |_| {}).unwrap();
    run_until_empty(&mut index, &registry, &WorkerOptions::default()).unwrap();
    for word in ["resume", "किराया", "家賃", "receipt"] {
        let found: Vec<ContentMatch> = store::match_contents(&index, word, 10).unwrap();
        assert_eq!(found.iter().map(|m| (m.path.as_str(), m.label.as_str())).collect::<Vec<_>>(), [(at("lease.pdf").as_str(), "page 2")], "{word}");
    }
    assert_eq!(store::match_contents(&index, "megabytes", 10).unwrap().len(), 1, "a 5 MB PDF is read");
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
    assert_eq!(status("scan.pdf"), ("empty".into(), None));
    assert_eq!(status("fake.pdf"), ("skipped".into(), Some(NOT_A_PDF.into())));
    assert_eq!(status("huge.pdf"), ("skipped".into(), Some("larger than 512 MB".into())));
    let textless: String = index
        .conn
        .query_row(
            "SELECT value FROM facts WHERE key = 'textless_pages' AND file_id = (SELECT id FROM files WHERE path = ?1)",
            [at("scan.pdf")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(textless, "1", "the scan is marked for OCR");
}
