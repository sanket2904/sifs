//! JSON, GeoJSON and notebooks: the keys and string values, one to a line,
//! with escapes decoded. Numbers and the punctuation are left out. A file
//! that doesn't parse is indexed as plain text.

use crate::extract::decode::{decode, read_capped};
use crate::extract::{fact, Class, Flow, Limits, ReadError, Reader, Sink, MB};
use serde_json::Value;
use std::path::Path;

pub struct Json;

impl Reader for Json {
    fn id(&self) -> &'static str {
        "json"
    }
    fn version(&self) -> u32 {
        1
    }
    fn exts(&self) -> &'static [&'static str] {
        &["json", "geojson", "ipynb"]
    }
    fn class(&self) -> Class {
        Class::Other
    }
    fn limits(&self) -> Limits {
        Limits { max_bytes: MB, ..Limits::default() }
    }
    fn read(&self, path: &Path, sink: &mut Sink) -> Result<(), ReadError> {
        let decoded = decode(&read_capped(path, self.limits().max_bytes)?, None)?;
        if let Some(encoding) = decoded.encoding {
            sink.fact(fact::ENCODING, encoding);
        }
        match serde_json::from_str::<Value>(&decoded.text) {
            Ok(value) => {
                walk(&value, sink)?;
            }
            Err(_) => {
                sink.text(&decoded.text)?;
            }
        }
        Ok(())
    }
}

fn walk(value: &Value, sink: &mut Sink) -> Result<Flow, ReadError> {
    match value {
        Value::String(s) => line(s, sink),
        Value::Array(items) => {
            for item in items {
                if walk(item, sink)? == Flow::Full {
                    return Ok(Flow::Full);
                }
            }
            Ok(Flow::More)
        }
        Value::Object(map) => {
            for (key, item) in map {
                if line(key, sink)? == Flow::Full || walk(item, sink)? == Flow::Full {
                    return Ok(Flow::Full);
                }
            }
            Ok(Flow::More)
        }
        Value::Number(_) | Value::Bool(_) | Value::Null => Ok(Flow::More),
    }
}

fn line(s: &str, sink: &mut Sink) -> Result<Flow, ReadError> {
    if s.trim().is_empty() {
        return Ok(Flow::More);
    }
    match sink.text(s)? {
        Flow::More if !s.ends_with('\n') => sink.text("\n"),
        flow => Ok(flow),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn read(body: &[u8]) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        std::fs::write(&path, body).unwrap();
        let mut sink = Sink::new(1_000_000, Instant::now() + Duration::from_secs(10));
        Json.read(&path, &mut sink).unwrap();
        sink.finish().0.into_iter().map(|p| p.text).collect()
    }

    #[test]
    fn keys_and_string_values_are_kept_with_escapes_decoded() {
        let text = read(br#"{"theme": "caf\u00e9 noir", "size": 12, "tags": ["walrus", true, null, 3.5], "nested": {"city": "Pune"}}"#);
        // Key order depends on serde_json's features, which other crates set.
        let mut lines: Vec<&str> = text.lines().collect();
        lines.sort();
        assert_eq!(lines, vec!["Pune", "café noir", "city", "nested", "size", "tags", "theme", "walrus"]);
    }

    #[test]
    fn json_that_does_not_parse_is_read_as_text() {
        let text = read(b"{\"unfinished\": \"marmoset\", ");
        assert_eq!(text, "{\"unfinished\": \"marmoset\", ");
    }
}
