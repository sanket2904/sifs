//! Measures the index on real folders: sifs-bench <db> <folder>... -- <query>...
//! Reads the folders only; writes nothing but the index file given.

use sifs_lib::index::{crawl::scan, db::Index, ignore::Rules, search::search};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let (db, folders) = args[..split].split_first().expect("usage: sifs-bench <db> <folder>... -- <query>...");
    let queries = if split < args.len() { &args[split + 1..] } else { &[][..] };
    let home = std::env::var("HOME")?;
    let rules = Rules::new(&Rules::default_patterns(), &home)?;
    let mut index = Index::open(std::path::Path::new(db))?;
    for folder in folders {
        let stats = scan(&mut index, folder, &rules, |_| {})?;
        println!("scan {folder}: {} indexed, {} skipped, {} ms", stats.indexed, stats.skipped, stats.millis);
    }
    println!("rows: {}", index.count()?);
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
    }
    Ok(())
}
