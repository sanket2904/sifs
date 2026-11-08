use super::Tokenizer;
use crate::index::db::Index;
use rusqlite::params;
use std::ops::ControlFlow;

fn words(tokenizer: &mut Tokenizer, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    tokenizer
        .words(text, |word, _| {
            if !word.is_empty() {
                out.push(word.to_string());
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    out
}

#[test]
fn words_are_the_contents_index_words_for_every_bmp_character() {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open(&dir.path().join("t.db")).unwrap();
    let block = |n: u32| -> String { (n * 256..(n + 1) * 256).filter_map(char::from_u32).map(|c| format!("a{c}b ")).collect() };
    index.conn.execute_batch("BEGIN").unwrap();
    for n in 0..256 {
        index
            .conn
            .execute("INSERT INTO contents (id, file_id, part, label, text) VALUES (?1, 1, ?1, '', ?2)", params![n + 1, block(n)])
            .unwrap();
    }
    index.conn.execute_batch("COMMIT; CREATE VIRTUAL TABLE temp.v USING fts5vocab(main, contents_fts, 'instance');").unwrap();
    let mut indexed = vec![Vec::<String>::new(); 256];
    let mut stmt = index.conn.prepare("SELECT doc, term FROM temp.v ORDER BY doc, offset").unwrap();
    for row in stmt.query_map([], |r| Ok((r.get::<_, usize>(0)?, r.get::<_, String>(1)?))).unwrap() {
        let (doc, term) = row.unwrap();
        if !term.is_empty() {
            indexed[doc - 1].push(term);
        }
    }
    let mut tokenizer = Tokenizer::contents(&index.conn).unwrap();
    for (n, theirs) in indexed.iter().enumerate() {
        let ours = words(&mut tokenizer, &block(n as u32));
        let differ = ours.iter().zip(theirs).position(|(a, b)| a != b).unwrap_or(ours.len().min(theirs.len()));
        assert_eq!(ours.len(), theirs.len(), "U+{:04X} block: first difference at word {differ}", n * 256);
        assert!(ours == *theirs, "U+{:04X} block: ours {:?}, index {:?}", n * 256, &ours[differ..], &theirs[differ..]);
        for term in theirs {
            assert_eq!(words(&mut tokenizer, term), std::slice::from_ref(term), "a word as the index keeps it is that word again");
        }
    }
}

#[test]
fn word_positions_are_bytes_of_the_text_and_breaking_stops() {
    let dir = tempfile::tempdir().unwrap();
    let index = Index::open(&dir.path().join("t.db")).unwrap();
    let mut tokenizer = Tokenizer::contents(&index.conn).unwrap();
    let text = "Ré\u{301}sumé, किराया\0x";
    let mut seen = Vec::new();
    let finished = tokenizer
        .words(text, |word, at| {
            seen.push((word.to_string(), &text[at]));
            ControlFlow::Continue(())
        })
        .unwrap();
    assert!(finished);
    assert_eq!(seen, [("resume".to_string(), "Ré\u{301}sumé"), ("किराया".to_string(), "किराया"), ("x".to_string(), "x")]);
    let mut first = None;
    assert!(!tokenizer.words(text, |word, _| { first = Some(word.to_string()); ControlFlow::Break(()) }).unwrap());
    assert_eq!(first.as_deref(), Some("resume"));
}
