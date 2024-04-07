// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod trie;

use core::str;
use std::path::PathBuf;
use notify::{Event, EventKind, Watcher};
use serde::{Deserialize, Serialize};
use tauri::Manager;
use trie::Trie;

struct TrieState {
    tries : Vec<Trie>
}


#[derive(Serialize, Deserialize)]
struct FileResult {
    name: String,
    path: String,
    location: String,
    file_type: String,
}


impl FileResult {
    fn get_file_result(path: String) -> FileResult {
        let path = PathBuf::from(path);
        let file = match path.file_name() {
            Some(name) => name.to_str().unwrap().to_string(),
            None => String::new(),
        };
        let location = match path.parent() {
            Some(parent) => parent.to_str().unwrap().to_string(),
            None => String::new(),
        };
        let file_type = match path.extension() {
            Some(extension) => extension.to_str().unwrap().to_string(),
            None => String::new(),
        };
        FileResult {
            name: file,
            path: path.to_str().unwrap().to_string(),
            location,
            file_type,
        }
    }
}

// tauri command to run the file on double click

#[tauri::command]
fn open_file_or_folder(path: String) {
    let a =  std::process::Command::new("explorer.exe").arg(path).output().unwrap();
    println!("{:?}", a)
}




#[tauri::command]
fn search_files(trie_state: tauri::State<TrieState>, query: String) -> Vec<FileResult> {
    let trie_state = trie_state.to_owned();
    let mut results = vec![];
    for trie in trie_state.tries.iter() {
        let _ = trie.starts_with(query.clone()).iter().for_each(|path| {
            results.push(FileResult::get_file_result(path.clone()));
        });
    }
    results
}


fn handle_event(event: Event) {

    // if path is in AppData,ProgramData,Windows,Windows.old,Program Files ignore it

    if event.paths.iter().any(|path| {
        let path = PathBuf::from(path);
        let path_str = path.to_str().unwrap();
        if path_str.contains("AppData")
            || path_str.contains("ProgramData")
            || path_str.contains("Windows")
            || path_str.contains("Windows.old")
            || path_str.contains("Program Files")
            || path_str.contains("node_modules")
            || path_str.contains("\\target\\debug")
            || path_str.contains(".git")
            || path_str.contains("$")
            || path_str.starts_with("System")
            || path.starts_with(".")
        {
            return true;
        }
        return false;
    }) {
        return;
    }


    match event.kind {
        EventKind::Create(e) => {
            println!("file created in drive {:?}", event.paths);
        },
        EventKind::Modify(e) => {
            println!("file modified in drive {:?}", event.paths);
        },
        EventKind::Remove(e) => {
            println!("file removed in drive {:?}", event.paths);
        },
        _ => {
            println!("file not created");
        }
    }
}

fn main() {
    let mut trie_state = TrieState {
        tries: vec![],
    };


    // add watcher to watch for changes in the file system

    
    let mut watcher = notify::recommended_watcher(move |res| {
        match res {
            Ok(event) => {
                handle_event(event);
            },
            Err(e) => {
                println!("watch error: {:?}", e);
            },
        }
    }).unwrap();

    


    for drive_letter in "ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars() {
        let drive = format!("{}:\\", drive_letter);
        if std::path::Path::new(&drive).exists() {
            let trie = match Trie::load_trie(&format!("{}.data", drive_letter)) {
                Ok(trie) => trie,
                Err(_) => {
                    let mut trie = Trie::new();
                    create_trie(&mut trie, &drive);
                    trie.save_trie(&format!("{}.data", drive_letter)).unwrap();
                    trie
                }
            };
            




            
            
                
            let _ = watcher.watch(std::path::Path::new(&drive), notify::RecursiveMode::Recursive).unwrap();
                
            
            trie_state.tries.push(trie);
        }
    }
    println!("Starting tauri application");

    tauri::Builder::default()
        .setup(|app| {
            app.manage(trie_state);
            let window = app.get_window("main").unwrap();
            window.listen("tauri://destroyed", |msg| {
                println!("got close request {:?}", msg);
                // Ok(())
            });
            Ok(())
        }).invoke_handler(tauri::generate_handler![search_files , open_file_or_folder])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn create_trie(trie: &mut trie::Trie, path: &str) {
    let main = walkdir::WalkDir::new(path).into_iter();
    main.filter_entry(|e| {
        let path = e.path();
        let path_str = path.to_str().unwrap();
        // also filter the target directory
        if path_str.contains("node_modules")
            || path_str.contains("\\target\\debug")
            || path_str.contains(".git")
            || path_str.contains("$")
            || path_str.starts_with("System")
            || path.starts_with(".")
        {
            return false;
        }
        return true;
    })
    .for_each(|entry| {
        match entry {
            Ok(entry) => {
                // println!("{:?}", entry);
                let path = entry.path().to_str();
                // let path = PathBuf::from(path);
                // get the last part of the path
                match path {
                    Some(path) => {
                        let file_name = path.split("\\").last().unwrap().to_string();
                    

                        trie.insert(file_name, path.to_string());
                    }
                    None => {
                        return;
                    }
                }
            }
            Err(_) => {
                return;
            }
        }
    });
}
