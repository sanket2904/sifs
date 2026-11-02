//! Prints the ranked results of a query against an index: sifs-query <db> <query> [limit]
use sifs_lib::index::{db::Index, search::search};
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let index = Index::open(std::path::Path::new(&args[0]))?;
    let limit = args.get(2).and_then(|l| l.parse().ok()).unwrap_or(60);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64;
    for (i, hit) in search(&index, &args[1], limit, now)?.iter().enumerate() {
        println!("{:3} {:7.2} {}", i + 1, hit.score, hit.file.path);
    }
    Ok(())
}
