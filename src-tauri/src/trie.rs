use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
#[derive(Default, Debug)]
//
#[derive(Serialize, Deserialize)]
pub struct TrieNode {
    pub children: HashMap<u8, TrieNode>,
    pub is_end: bool,
    location: Vec<String>,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Trie {
    pub root: TrieNode,
}

impl Trie {
    pub fn new() -> Self {
        Trie {
            root: TrieNode {
                children: HashMap::new(),
                is_end: false,
                location: Vec::new(),
            },
        }
    }

    pub fn insert(&mut self, word: String, location: String) {
        let mut current = &mut self.root;
        for c in word.bytes() {
            current = current.children.entry(c).or_insert(TrieNode {
                children: HashMap::new(),
                is_end: false,
                location: Vec::new(),
            });
        }
        current.is_end = true;
        current.location.push(location);
    }


    pub fn starts_with(path: &str, prefix: String) -> Vec<String> {
        let first_byte = prefix.as_bytes()[0];
        let path = format!("{}/{}.data", path, first_byte);
        let mut result = Vec::new();
        let current = if let Ok(trie) = Trie::load_trie(&path) {
            trie.root
        } else {
            return result;
        };
        let mut current = &current;
        for c in prefix.bytes().skip(1) {
            match current.children.get(&c) {
                Some(node) => current = node,
                None => return result,
            }
        }
        let mut stack = vec![current];
        while let Some(node) = stack.pop() {
            if node.is_end {
                result.extend(node.location.clone());
            }
            for child in node.children.values() {
                stack.push(child);
            }
        }
        result
    }

    pub fn load_trie(path: &str) -> Result<Trie, Box<dyn Error>> {
        // load the trie from a file
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let trie: Trie = bincode::deserialize_from(reader)?;
        Ok(trie)
    }

    pub fn save_trie(&self, path: &str) -> Result<(), Box<dyn Error>> {
        // save the trie to a file
        let file = std::fs::File::create(path).unwrap();
        let writer = std::io::BufWriter::new(file);
        bincode::serialize_into(writer, self).unwrap();
        Ok(())
    }

    pub fn save_each_child(&self, path: &str) -> Result<(), Box<dyn Error>> {
        for (key, value) in self.root.children.iter() {
            let path = format!("{}/{}.data", path, key);
            let file = std::fs::File::create(path).unwrap();
            let writer = std::io::BufWriter::new(file);
            bincode::serialize_into(writer, value).unwrap();
        }
        Ok(())
    }

    pub fn insert_file(
        word: String,
        location: String,
        drive: String,
    ) -> Result<(), Box<dyn Error>> {
        let first_byte = word.as_bytes()[0];
        let path = format!("{}/{}.data", drive, first_byte);
        println!("{:?}", path);
        let mut root = if let Ok(trie) = Trie::load_trie(&path) {
            trie
        } else {
            Trie::new()
        };

        let mut current = &mut root.root;
        for c in word.bytes().skip(1) {
            current = current.children.entry(c).or_insert(TrieNode {
                children: HashMap::new(),
                is_end: false,
                location: Vec::new(),
            });
        }
        current.is_end = true;
        current.location.push(location);
        root.save_trie(&path)?;
        Ok(())
    }

    // trie remove file function
    pub fn remove_file(
        word: String,
        location: String,
        drive: String,
    ) -> Result<(), Box<dyn Error>> {
        let first_byte = word.as_bytes()[0];
        let path = format!("{}/{}.data", drive, first_byte);
        let mut root = if let Ok(trie) = Trie::load_trie(&path) {
            trie
        } else {
            println!("file not found");
            return Ok(());
        };
        let mut current = &mut root.root;
        for c in word.bytes().skip(1) {
            println!("in the loop");
            match current.children.get_mut(&c) {
                Some(node) => current = node,
                None => return Ok(()),
            }
        }
        println!("current location {:?}", current.location);
        current.is_end = false;
        current.location.retain(|x| x != &location);
        root.save_trie(&path)?;
        Ok(())
    }



}
