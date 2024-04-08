// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub mod trie;

use core::str;
use std::path::PathBuf;
use notify::{event::ModifyKind, Event, EventKind, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{CustomMenuItem, Manager, SystemTray, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem};
use trie::Trie;

struct TrieState {
    tries : Vec<String>
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
        let trie = Trie::starts_with(trie, query.clone());
        for val in trie {
            results.push(FileResult::get_file_result(val));
        }
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
            || path_str.contains(".data")
        {
            return true;
        }
        return false;
    }) {
        return;
    }

    let paths = event.paths;
    
    match event.kind {
        EventKind::Create(e) => {
            let drive = paths[0].to_str().unwrap().chars().next().unwrap();
            let file_name = paths[0].file_name().unwrap().to_str().unwrap();
            let drive = format!("./{}", drive);
            let _main = Trie::insert_file(file_name.to_owned(), paths[0].to_str().unwrap().to_owned() , drive).unwrap();
        },
        EventKind::Modify(e) => {
            // println!("file modified");
            if e == ModifyKind::Name(notify::event::RenameMode::From) {
                // remove the file from the trie
                let drive = paths[0].to_str().unwrap().chars().next().unwrap();
                let file_name = paths[0].file_name().unwrap().to_str().unwrap();
                let drive = format!("./{}", drive);
                let _main = Trie::remove_file(file_name.to_owned(), paths[0].to_str().unwrap().to_owned() , drive).unwrap();

            } else if e == ModifyKind::Name(notify::event::RenameMode::To) {
                // add the file to the trie
                let drive = paths[0].to_str().unwrap().chars().next().unwrap();
                let file_name = paths[0].file_name().unwrap().to_str().unwrap();
                let drive = format!("./{}", drive);
                let _main = Trie::insert_file(file_name.to_owned(), paths[0].to_str().unwrap().to_owned() , drive).unwrap();
            }

        },
        EventKind::Remove(e) => {
            let drive = paths[0].to_str().unwrap().chars().next().unwrap();
            let file_name = paths[0].file_name().unwrap().to_str().unwrap();
            let drive = format!("./{}", drive);
            let _main = Trie::remove_file(file_name.to_owned(), paths[0].to_str().unwrap().to_owned() , drive).unwrap();
        },
        _ => {
            println!("file not created");
        }
    }
}

fn main() {
   
    let mut init = TrieState {
        tries: Vec::new()
    };

    let mut watcher = notify::recommended_watcher(move |event| {
        match event {
            Ok(event) => {
                handle_event(event);
            }
            Err(e) => {
                println!("watch error: {:?}", e);
            }
        };
    }).unwrap();

    for drive_letter in "ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars() {
        let drive = format!("{}:\\", drive_letter);
        if std::path::Path::new(&drive).exists() {
            init.tries.push(format!("./{}", drive_letter));
            create_trie( &drive);
        }
    }
    for drive_letter in "ABCDEFGHIJKLMNOPQRSTUVWXYZ".chars() {
        let drive = format!("{}:\\", drive_letter);
        if std::path::Path::new(&drive).exists() {
            let path = PathBuf::from(&drive);
            watcher.watch(&path, notify::RecursiveMode::Recursive).unwrap();
        }
    }
    println!("Starting tauri application");
    let quit = CustomMenuItem::new("quit".to_string(), "Quit");
    let hide = CustomMenuItem::new("hide".to_string(), "Hide");
    let show = CustomMenuItem::new("show".to_string(), "Show");

    let tray = SystemTray::new().with_menu(SystemTrayMenu::new().add_item(hide).add_item(show).add_item(quit));

    tauri::Builder::default().system_tray(tray).on_system_tray_event(|app, event| {

        match event {

            SystemTrayEvent::DoubleClick { .. } => {
                let window = app.get_window("main").unwrap();
                window.show().unwrap();
            },

            SystemTrayEvent::MenuItemClick  { id, .. } => {
                match id.as_str() {
                    "quit" => {
                        app.exit(0);
                    },
                    "hide" => {
                        let window = app.get_window("main").unwrap();
                        window.hide().unwrap();
                    },
                    "show" => {
                        app.get_window("main").unwrap().show().unwrap();
                    },
                    _ => {}
                }
            },
            _ => {}
        }


    })
        .on_window_event(|event| match event.event() {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                event.window().hide().unwrap();
                api.prevent_close();
            }
            _ => {}
        }).manage(init)
        .invoke_handler(tauri::generate_handler![search_files , open_file_or_folder])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn create_trie(path: &str) {
    // create the directory which is the first letter of the path ./ 
    let first = path.chars().next().unwrap();
    let dir_path = format!("./{}", first);
    if std::path::Path::new(&dir_path).exists() && std::fs::read_dir(&dir_path).unwrap().next().is_some() {
        // The directory exists and is not empty, so the data has already been cached
        return;
    }
    let _ = std::fs::create_dir(&dir_path);
    let mut trie = Trie::new();
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
    let _ = trie.save_each_child(&format!("./{}", first)).unwrap();
}
