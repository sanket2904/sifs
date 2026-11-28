use super::*;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::Instant;

/// A deadline no test reaches.
const LONG: Duration = Duration::from_secs(120);
/// How long a stand-in may take to start on a busy Mac.
const START: Duration = Duration::from_secs(20);
/// How long an ended process may take to leave the process table.
const GONE: Duration = Duration::from_secs(10);

/// Every stand-in that waits gives up after two minutes, so a runner that
/// fails to end it leaves nothing for long.
const LIVES: &str = "i=0; while [ $i -lt 120 ]; do sleep 1; i=$((i+1)); done";

/// A temp folder of stand-in tools. A process whose command line names the
/// folder is one these tests started; dropping it kills any still alive.
struct Scratch(tempfile::TempDir);

impl Scratch {
    fn new() -> Scratch {
        let scratch = Scratch(tempfile::tempdir().unwrap());
        let sleeper = format!("echo $$ > \"$1\"\n{LIVES}");
        let stubborn = format!("trap '' HUP INT TERM QUIT\necho $$ > \"$1\"\n{LIVES}");
        let escaper = "set -m\n/bin/sh \"${0%/*}/sleeper\" \"$1.away\" &\necho $$ > \"$1\"\nwait";
        for (name, body) in [
            ("sleeper", sleeper.as_str()),
            ("stubborn", stubborn.as_str()),
            ("family", "echo $$ > \"$1\"\n/bin/sh \"${0%/*}/child\" \"$1\" &\nwait"),
            ("child", "echo $$ > \"$1.child\"\n/bin/sh \"${0%/*}/sleeper\" \"$1.grand\" &\nwait"),
            ("silent", LIVES),
            ("quick", "[ -n \"$1\" ] && : > \"$1\"\nexit 0"),
            ("escaper", escaper),
        ] {
            scratch.write_tool(name, body);
        }
        scratch
    }

    fn write_tool(&self, name: &str, body: &str) -> PathBuf {
        let path = self.tool(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn tool(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }

    /// The live processes whose command line names this folder, from ps.
    fn marked(&self) -> Vec<libc::pid_t> {
        let out = Command::new("/bin/ps").args(["-axo", "pid=,stat=,command="]).output().unwrap();
        let marker = self.0.path().to_str().unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|line| line.contains(marker))
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?.parse().ok()?;
                let stat = fields.next()?;
                (!stat.starts_with('Z')).then_some(pid)
            })
            .collect()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for pid in self.marked() {
            // SAFETY: kill has no memory effects; pid is a process this test started.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

fn pid_in(path: &Path) -> Option<libc::pid_t> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Whether the system has no live process with this pid (a zombie is dead).
fn gone(pid: libc::pid_t) -> bool {
    let out = Command::new("/bin/ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().unwrap();
    let stat = String::from_utf8_lossy(&out.stdout);
    stat.trim().is_empty() || stat.trim().starts_with('Z')
}

fn within(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + limit;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn started(paths: &[PathBuf]) -> Vec<libc::pid_t> {
    assert!(within(START, || paths.iter().all(|p| pid_in(p).is_some())), "the stand-ins started");
    paths.iter().map(|p| pid_in(p).unwrap()).collect()
}

fn alive(pids: &[libc::pid_t]) -> Vec<libc::pid_t> {
    pids.iter().copied().filter(|&pid| !gone(pid)).collect()
}

/// Runs a tool on another thread; the receiver gets what became of it.
fn run_aside(runner: &Arc<Runner>, program: PathBuf, arg: PathBuf, time: Duration) -> mpsc::Receiver<Ran> {
    let (tx, rx) = mpsc::channel();
    let runner = runner.clone();
    thread::spawn(move || tx.send(runner.run(&program, [arg], time)));
    rx
}

#[test]
fn stop_ends_every_running_tool_and_everything_it_started() {
    let scratch = Scratch::new();
    let runner = Arc::new(Runner::default());
    let running: Vec<_> = [("family", "f"), ("stubborn", "s"), ("sleeper", "p"), ("silent", "q")]
        .into_iter()
        .map(|(tool, pid)| run_aside(&runner, scratch.tool(tool), scratch.file(pid), LONG))
        .collect();
    let pids = started(&["f", "f.child", "f.grand", "s", "p"].map(|n| scratch.file(n)));
    assert_eq!(scratch.marked().len(), 6, "family, child, grandchild, stubborn, sleeper, silent");
    runner.stop();
    assert!(within(GONE, || alive(&pids).is_empty()), "alive after stop: {:?}", alive(&pids));
    assert!(within(GONE, || scratch.marked().is_empty()), "alive after stop: {:?}", scratch.marked());
    for rx in running {
        let ran = rx.recv_timeout(GONE).expect("the run returned");
        assert!(matches!(ran, Ran::Stopped), "{ran:?}");
    }
}

#[test]
fn stop_can_be_called_twice_at_once_and_after_it_no_tool_starts() {
    let scratch = Scratch::new();
    let runner = Arc::new(Runner::default());
    let running = run_aside(&runner, scratch.tool("sleeper"), scratch.file("p"), LONG);
    let pid = started(&[scratch.file("p")]);
    let stops: Vec<_> = (0..2)
        .map(|_| {
            let runner = runner.clone();
            thread::spawn(move || runner.stop())
        })
        .collect();
    for stop in stops {
        stop.join().unwrap();
    }
    assert!(within(GONE, || alive(&pid).is_empty()), "alive after stop: {pid:?}");
    let ran = running.recv_timeout(GONE).expect("the run returned");
    assert!(matches!(ran, Ran::Stopped), "{ran:?}");
    runner.stop();
    let touched = scratch.file("ran");
    let ran = runner.run(&scratch.tool("quick"), [&touched], LONG);
    assert!(matches!(ran, Ran::Stopped), "{ran:?}");
    assert!(!touched.exists(), "a start after stop never starts the tool");
}

// The window this closes: a tool started but not yet in the set when stop
// takes the set. Each round, eight threads start a tool each (and then one
// more, which comes after the stop) while the test calls stop once k of
// the first tools are up, k going round 0 to 8.
#[test]
fn no_tool_outlives_a_stop_called_while_many_threads_start_tools() {
    const ROUNDS: usize = 200;
    const THREADS: usize = 8;
    let scratch = Scratch::new();
    let mut ran = 0;
    for round in 0..ROUNDS {
        let runner = Arc::new(Runner::default());
        let barrier = Arc::new(Barrier::new(THREADS + 1));
        let firsts: Vec<_> = (0..THREADS).map(|i| scratch.file(&format!("{round}-{i}"))).collect();
        let threads: Vec<_> = firsts
            .iter()
            .map(|first| {
                let (runner, barrier) = (runner.clone(), barrier.clone());
                let program = scratch.tool("sleeper");
                let pids = [first.clone(), first.with_extension("again")];
                thread::spawn(move || {
                    barrier.wait();
                    pids.map(|pid| runner.run(&program, [pid], LONG))
                })
            })
            .collect();
        barrier.wait();
        let k = round % (THREADS + 1);
        assert!(within(START, || firsts.iter().filter(|p| pid_in(p).is_some()).count() >= k), "round {round}: {k} up");
        runner.stop();
        assert!(within(GONE, || scratch.marked().is_empty()), "round {round}: alive after stop: {:?}", scratch.marked());
        for thread in threads {
            let [first, again] = thread.join().unwrap();
            assert!(matches!(first, Ran::Stopped), "round {round}: {first:?}");
            assert!(matches!(again, Ran::Stopped), "round {round}: {again:?}");
        }
        for first in &firsts {
            assert!(!first.with_extension("again").exists(), "round {round}: a start after stop never starts");
        }
        ran += firsts.iter().filter(|p| p.exists()).count();
    }
    assert!(ran >= (0..ROUNDS).map(|round| round % (THREADS + 1)).sum(), "{ran} tools ran before their stop");
}

#[test]
fn a_tool_past_its_deadline_is_ended_with_everything_it_started() {
    let scratch = Scratch::new();
    let runner = Arc::new(Runner::default());
    let started_at = Instant::now();
    let running = run_aside(&runner, scratch.tool("family"), scratch.file("f"), Duration::from_secs(5));
    let pids = started(&["f", "f.child", "f.grand"].map(|n| scratch.file(n)));
    let ran = running.recv_timeout(Duration::from_secs(30)).expect("the run returned");
    assert!(matches!(ran, Ran::TimedOut), "{ran:?}");
    assert!(started_at.elapsed() >= Duration::from_secs(5), "not before its deadline");
    assert!(within(GONE, || alive(&pids).is_empty()), "alive after the deadline: {:?}", alive(&pids));
}

// A known limit: a process the tool moves into a process group of its own
// is not in the tool's group, so stop does not end it. It still holds the
// tool's output, and the run returns all the same.
#[test]
fn a_child_moved_to_another_group_outlives_stop_and_does_not_hold_the_run() {
    let scratch = Scratch::new();
    let runner = Arc::new(Runner::default());
    let running = run_aside(&runner, scratch.tool("escaper"), scratch.file("e"), LONG);
    let pids = started(&[scratch.file("e"), scratch.file("e.away")]);
    runner.stop();
    let ran = running.recv_timeout(Duration::from_secs(30)).expect("the run returned");
    assert!(matches!(ran, Ran::Stopped), "{ran:?}");
    assert!(within(GONE, || gone(pids[0])), "the tool is ended");
    assert!(!gone(pids[1]), "the child in its own group lives on (the known limit)");
}

fn finished(ran: Ran) -> (i32, Vec<u8>, Vec<u8>) {
    match ran {
        Ran::Finished { code, stdout, stderr } => (code, stdout, stderr),
        other => panic!("not finished: {other:?}"),
    }
}

#[test]
fn arguments_reach_the_tool_byte_for_byte_with_no_shell() {
    let scratch = Scratch::new();
    let echo = scratch.write_tool("echo-args", "printf '%s\\0' \"$@\"");
    let args = ["two words", "it's \"quoted\"", "$(touch made-by-a-shell)", "a\nnewline", "-n", "", "*", "back\\slash"];
    let expected: Vec<u8> = args.iter().flat_map(|arg| arg.bytes().chain([0])).collect();
    let (code, stdout, _) = finished(Runner::default().run(&echo, args, LONG));
    assert_eq!(code, 0);
    assert_eq!(String::from_utf8_lossy(&stdout), String::from_utf8_lossy(&expected));
}

#[test]
fn standard_input_is_closed() {
    let scratch = Scratch::new();
    let reads = scratch.write_tool("reads", "if read line; then echo \"got $line\"; else echo nothing; fi");
    assert_eq!(finished(Runner::default().run(&reads, [""; 0], LONG)), (0, b"nothing\n".to_vec(), Vec::new()));
}

#[test]
fn the_exit_code_is_given_as_it_is() {
    let scratch = Scratch::new();
    for code in [0, 1, 2, 7] {
        let tool = scratch.write_tool(&format!("exit-{code}"), &format!("echo out\necho err >&2\nexit {code}"));
        let ran = finished(Runner::default().run(&tool, [""; 0], LONG));
        assert_eq!(ran, (code, b"out\n".to_vec(), b"err\n".to_vec()));
    }
}

#[test]
fn a_tool_ended_by_a_signal_is_not_reported_as_finished() {
    let scratch = Scratch::new();
    let crash = scratch.write_tool("crash", "echo dying >&2\nkill -TERM $$");
    match Runner::default().run(&crash, [""; 0], LONG) {
        Ran::Signalled { signal, stderr } => assert_eq!((signal, stderr), (libc::SIGTERM, b"dying\n".to_vec())),
        other => panic!("{other:?}"),
    }
}

// Each output alone fills its pipe many times over: a runner that reads
// one only after the tool ends waits on a tool that waits on it.
#[test]
fn both_outputs_are_read_while_the_tool_runs() {
    let scratch = Scratch::new();
    let loud = scratch.write_tool(
        "loud",
        "head -c 1048576 /dev/zero | tr '\\0' e >&2\necho the end >&2\nhead -c 1048576 /dev/zero | tr '\\0' o",
    );
    let (code, stdout, stderr) = finished(Runner::default().run(&loud, [""; 0], Duration::from_secs(20)));
    assert_eq!(code, 0);
    assert!(stdout.len() == 1 << 20 && stdout.iter().all(|&b| b == b'o'), "all of standard output");
    assert_eq!(stderr.len(), STDERR_KEPT, "the last 16 KB of standard error");
    assert!(stderr.ends_with(b"eeethe end\n"), "its end");
}

#[test]
fn standard_output_past_the_cap_ends_the_tool() {
    let scratch = Scratch::new();
    let runner = Runner::default();
    let exact = scratch.write_tool("exact", &format!("head -c {STDOUT_CAP} /dev/zero"));
    let over = scratch.write_tool("over", &format!("head -c {} /dev/zero", STDOUT_CAP + 1));
    let endless = scratch.write_tool("endless", "echo $$ > \"$1\"\nexec /usr/bin/yes \"$0\"");
    assert_eq!(finished(runner.run(&exact, [""; 0], LONG)).1.len(), STDOUT_CAP, "the cap itself fits");
    let ran = runner.run(&over, [""; 0], LONG);
    assert!(matches!(ran, Ran::WroteTooMuch), "{ran:?}");
    let ran = runner.run(&endless, [scratch.file("y")], Duration::from_secs(20));
    assert!(matches!(ran, Ran::WroteTooMuch), "{ran:?}");
    let pid = pid_in(&scratch.file("y")).unwrap();
    assert!(within(GONE, || gone(pid)), "the endless tool is ended");
}

#[test]
fn a_tool_that_never_reads_or_writes_is_ended_at_its_deadline() {
    let scratch = Scratch::new();
    let started_at = Instant::now();
    let ran = Runner::default().run(&scratch.tool("silent"), [""; 0], Duration::from_millis(300));
    assert!(matches!(ran, Ran::TimedOut), "{ran:?}");
    assert!(started_at.elapsed() >= Duration::from_millis(300), "not before its deadline");
    assert!(within(GONE, || scratch.marked().is_empty()), "alive after its deadline: {:?}", scratch.marked());
}

#[test]
fn a_tool_that_cannot_be_started_says_why() {
    let scratch = Scratch::new();
    let plain = scratch.file("plain");
    std::fs::write(&plain, "#!/bin/sh\n").unwrap();
    for (program, kind) in [(scratch.file("absent"), std::io::ErrorKind::NotFound), (plain, std::io::ErrorKind::PermissionDenied)] {
        match Runner::default().run(&program, [""; 0], LONG) {
            Ran::CouldNotStart(e) => assert_eq!(e.kind(), kind, "{program:?}"),
            other => panic!("{program:?}: {other:?}"),
        }
    }
}
