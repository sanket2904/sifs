
use serde::{Serialize, Deserialize};
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
#[derive(Default, Debug , Serialize, Deserialize)]
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
            }
        }
    }

    pub fn insert(&mut self, word: String , location: String) {
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

    // pub fn search(&self, word: String) -> Vec<String> {
    //     let mut current = &self.root;
    //     for c in word.bytes() {
    //         match current.children.get(&c) {
    //             Some(node) => current = node,
    //             None => return Vec::new(),
    //         }
    //     }
    //     return current.location;
    // }


    // refactor this so that It will return a vector of PathBuf with partial match
   pub fn starts_with(&self, prefix: String) -> Vec<String> {
        let mut current = &self.root;
        let mut result = Vec::new();
        for c in prefix.bytes() {
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

    pub fn load_trie(path: &str) -> Result<Trie,Box<dyn Error> > {
        // load the trie from a file
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let trie : Trie =  bincode::deserialize_from(reader)?;
        Ok(trie)
    }

    pub fn save_trie(&self , path: &str) -> Result<(), Box<dyn Error>> {
        // save the trie to a file
        let file = std::fs::File::create(path).unwrap();
        let writer = std::io::BufWriter::new(file);
        bincode::serialize_into(writer, self).unwrap();
        Ok(())
    }

    

    
}
