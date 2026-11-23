//! Writes a text set (sample/FORMAT.md, "Input") from an index's read text:
//! sifs-textset --db <index> --out <new or empty folder> --reference-date YYYY-MM-DD
//! Opens the index read-only.

use anyhow::{bail, Context};
use std::path::PathBuf;

const USAGE: &str = "usage: sifs-textset --db <index> --out <new or empty folder> --reference-date YYYY-MM-DD";

fn main() -> anyhow::Result<()> {
    let (mut db, mut out, mut date) = (None, None, None);
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let slot = match flag.as_str() {
            "--db" => &mut db,
            "--out" => &mut out,
            "--reference-date" => &mut date,
            _ => bail!("unknown argument {flag:?}\n{USAGE}"),
        };
        *slot = Some(args.next().with_context(|| format!("{flag} needs a value\n{USAGE}"))?);
    }
    let (Some(db), Some(out), Some(date)) = (db, out, date) else { bail!(USAGE) };
    let out = PathBuf::from(out);
    let exported = sifs_lib::extract::textset::export(&PathBuf::from(db), &out, &date)?;
    println!("{} documents written to {}", exported.documents.len(), out.display());
    Ok(())
}
