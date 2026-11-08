//! Measures the index on real folders: sifs-bench [--contents] <db> <folder>... -- <query>...
//! Reads the folders only; writes nothing but the index file given.
//!
//! With --contents, it also reads inside the files after scanning them, and
//! reports the time per 1,000 files by reader, MB read per second, what
//! became of the files, how much the index grew by table, peak memory, and
//! how long each write held the writer lock.

use sifs_lib::extract::{store, worker::run_until_empty, Registry, WorkerOptions};
use sifs_lib::index::{crawl::scan, db::Index, ignore::Rules, search::search};
use std::collections::BTreeMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn main() -> anyhow::Result<()> {
    sifs_lib::extract::serve_helper_if_asked();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let contents = match args[..split].iter().position(|a| a == "--contents") {
        Some(at) => {
            args.remove(at);
            true
        }
        None => false,
    };
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let (db, folders) =
        args[..split].split_first().expect("usage: sifs-bench [--contents] <db> <folder>... -- <query>...");
    let queries = if split < args.len() { &args[split + 1..] } else { &[][..] };
    let home = std::env::var("HOME")?;
    let rules = Rules::new(&Rules::default_patterns(), &home)?;
    let mut index = Index::open(std::path::Path::new(db))?;
    let registry = Registry::platform();
    if contents {
        // As the app does: the queue fills during the scan.
        store::register_kinds(&mut index, &registry)?;
    }
    for folder in folders {
        let stats = scan(&mut index, folder, &rules, |_| {})?;
        println!("scan {folder}: {} indexed, {} skipped, {} ms", stats.indexed, stats.skipped, stats.millis);
    }
    println!("rows: {}", index.count()?);
    println!("peak memory after scanning: {:.1} MB", peak_rss_mb());
    if contents {
        read_contents(&mut index, &registry)?;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    for q in queries {
        let mut times = Vec::new();
        let mut n = 0;
        for _ in 0..20 {
            let t = Instant::now();
            n = search(&index, q, 50, now)?.len();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(|a, b| a.total_cmp(b));
        println!("query {q:?}: {n} hits, p50 {:.2} ms, p95 {:.2} ms", times[10], times[18]);
        if contents {
            let t = Instant::now();
            let n = store::match_contents(&index, q, 50)?.len();
            println!("  in contents: {n} files, {:.2} ms", t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    Ok(())
}

fn read_contents(index: &mut Index, registry: &Registry) -> anyhow::Result<()> {
    let before = table_sizes(index)?;
    let queued: i64 = index.conn().query_row("SELECT COUNT(*) FROM extract_queue", [], |r| r.get(0))?;
    let (wall, cpu) = (Instant::now(), cpu_time());
    let stats = run_until_empty(index, registry, &WorkerOptions::default())?;
    let (wall, cpu) = (wall.elapsed(), cpu_time() - cpu);
    let after = table_sizes(index)?;

    let files: i64 = index.conn().query_row("SELECT COUNT(*) FROM extracts", [], |r| r.get(0))?;
    println!(
        "\nreading inside files: {queued} queued, {files} done in {:.1} s ({:.1} s of CPU)",
        wall.as_secs_f64(),
        cpu.as_secs_f64()
    );
    if files > 0 {
        println!("  {:.0} ms per 1,000 files, wall clock", wall.as_secs_f64() * 1000.0 / files as f64 * 1000.0);
    }
    println!("  by reader (time is the readers' own, summed over threads):");
    let mut stmt = index.conn().prepare(
        "SELECT reader, COUNT(*), SUM(millis), SUM(CASE WHEN status IN ('done', 'empty') THEN size ELSE 0 END), SUM(chars)
         FROM extracts GROUP BY reader ORDER BY COUNT(*) DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?))
    })?;
    for row in rows {
        let (reader, n, millis, bytes, chars) = row?;
        let mb = bytes as f64 / 1e6;
        let per_second = if millis > 0 { mb / (millis as f64 / 1000.0) } else { f64::INFINITY };
        println!(
            "    {reader:6} {n:7} files  {:8.0} ms per 1,000  {mb:8.1} MB read  {per_second:7.1} MB/s  {:.1}M characters kept",
            millis as f64 / n as f64 * 1000.0,
            chars as f64 / 1e6
        );
    }
    println!("  what became of them:");
    let mut stmt =
        index.conn().prepare("SELECT status, COUNT(*) FROM extracts GROUP BY status ORDER BY COUNT(*) DESC")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (status, n) = row?;
        println!("    {status:8} {n}");
    }
    let mut stmt = index.conn().prepare(
        "SELECT status, COALESCE(error, ''), COUNT(*) FROM extracts WHERE status IN ('failed', 'skipped')
         GROUP BY 1, 2 ORDER BY 3 DESC LIMIT 8",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))? {
        let (status, error, n) = row?;
        println!("      {status}: {error} ({n})");
    }

    println!("  index growth by table:");
    let mut grown: Vec<(String, i64)> = after
        .iter()
        .map(|(name, size)| (name.clone(), size - before.get(name).copied().unwrap_or(0)))
        .filter(|(_, d)| *d != 0)
        .collect();
    grown.sort_by_key(|(_, d)| -d);
    let total: i64 = grown.iter().map(|(_, d)| d).sum();
    for (name, d) in &grown {
        println!(
            "    {name:24} {:9.1} KB  {:6.2} KB per file",
            *d as f64 / 1024.0,
            *d as f64 / 1024.0 / files.max(1) as f64
        );
    }
    println!(
        "    {:24} {:9.1} KB  {:6.2} KB per file",
        "total",
        total as f64 / 1024.0,
        total as f64 / 1024.0 / files.max(1) as f64
    );

    let mut holds = stats.holds.clone();
    holds.sort();
    if let Some(longest) = holds.last() {
        let p95 = holds[(holds.len() * 95 / 100).min(holds.len() - 1)];
        let total: Duration = holds.iter().sum();
        println!(
            "  writer lock: {} writes, {:.0} ms in all, longest {:.1} ms, p95 {:.1} ms",
            holds.len(),
            ms(total),
            ms(*longest),
            ms(p95)
        );
    }
    if stats.abandoned > 0 || !stats.errors.is_empty() {
        println!("  {} reads abandoned, errors: {:?}", stats.abandoned, stats.errors);
    }
    println!("peak memory after reading: {:.1} MB", peak_rss_mb());
    Ok(())
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Bytes per table and index, from SQLite's dbstat, after a checkpoint.
fn table_sizes(index: &Index) -> anyhow::Result<BTreeMap<String, i64>> {
    index.conn().query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    let mut stmt = index.conn().prepare("SELECT name, SUM(pgsize) FROM dbstat GROUP BY name")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn rusage() -> libc::rusage {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    usage
}

fn cpu_time() -> Duration {
    let u = rusage();
    let tv = |t: libc::timeval| Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64);
    tv(u.ru_utime) + tv(u.ru_stime)
}

/// Peak resident memory: ru_maxrss is in KB on Linux, bytes on macOS.
fn peak_rss_mb() -> f64 {
    let max = rusage().ru_maxrss as f64;
    if cfg!(target_os = "macos") {
        max / 1024.0 / 1024.0
    } else {
        max / 1024.0
    }
}
