//! Writes the M1a output file for one text set (sample/FORMAT.md):
//! m1a_extract <text-set-dir> <out.json> [--directory <banks.json>]
//! Without --directory the run is "directory off" and never opens the file.
use anyhow::Context;
use sifs_lib::findings::{dates::Date, run, Directory, Document};
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [set, out, rest @ ..] = args.as_slice() else {
        anyhow::bail!("usage: m1a_extract <text-set-dir> <out.json> [--directory <banks.json>]");
    };
    let directory = match rest {
        [] => None,
        [flag, path] if flag == "--directory" => {
            Some(Directory::parse(&std::fs::read_to_string(path)?)?)
        }
        _ => anyhow::bail!("unknown arguments: {rest:?}"),
    };
    let index: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(Path::new(set).join("index.json"))?)?;
    let reference = index["reference_date"]
        .as_str()
        .context("index.json has no reference_date")?;
    let part = |i: usize| {
        reference
            .split('-')
            .nth(i)
            .and_then(|p| p.parse::<u32>().ok())
    };
    let reference = part(0)
        .zip(part(1))
        .zip(part(2))
        .and_then(|((y, m), d)| Date::new(y as i32, m, d))
        .context("reference_date is no date")?;
    let docs = index["documents"]
        .as_array()
        .context("index.json has no documents")?
        .iter()
        .map(|id| {
            let id = id.as_str().context("document id")?;
            let text = std::fs::read_to_string(Path::new(set).join(format!("{id}.json")))?;
            Ok(serde_json::from_str::<Document>(&text)?)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let started = std::time::Instant::now();
    let output = run(
        &docs,
        directory.as_ref(),
        index["text_set"].as_str().unwrap_or(""),
        reference,
    );
    let took = started.elapsed();
    std::fs::write(out, serde_json::to_string_pretty(&output)? + "\n")?;
    eprintln!(
        "{} documents in {:.1} ms",
        docs.len(),
        took.as_secs_f64() * 1000.0
    );
    Ok(())
}
