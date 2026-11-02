//! The desktop app: Tauri commands over the index, background scans, the
//! watcher, and the tray.

use crate::index::crawl::{self, ScanStats};
use crate::index::db::Index;
use crate::index::ignore::Rules;
use crate::index::search::{self, Hit};
use crate::index::watch::Watch;
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_opener::OpenerExt;

struct AppState {
    db_path: PathBuf,
    home: String,
    /// The connection searches and settings use. The crawler and the
    /// watcher open their own.
    reader: Mutex<Index>,
    rules: Arc<Mutex<Rules>>,
    watch: Mutex<Option<Watch>>,
    scanning: Arc<Mutex<HashSet<String>>>,
}

#[derive(Serialize, Clone)]
struct FolderStatus {
    path: String,
    files: i64,
    scanning: bool,
}

#[derive(Serialize, Clone)]
struct Progress {
    root: String,
    indexed: u64,
}

#[derive(Serialize)]
struct Settings {
    folders: Vec<FolderStatus>,
    ignore: Vec<String>,
    suggested: Vec<String>,
    total: i64,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
fn search_files(state: State<AppState>, query: String, limit: Option<usize>) -> Result<Vec<Hit>, String> {
    let index = state.reader.lock().unwrap();
    search::search(&index, &query, limit.unwrap_or(60).min(500), now()).map_err(err)
}

#[tauri::command]
fn settings(state: State<AppState>) -> Result<Settings, String> {
    let index = state.reader.lock().unwrap();
    let scanning = state.scanning.lock().unwrap().clone();
    let folders = index
        .folders()
        .map_err(err)?
        .into_iter()
        .map(|path| FolderStatus { files: index.count_root(&path).unwrap_or(0), scanning: scanning.contains(&path), path })
        .collect();
    let ignore = index.ignore_patterns().map_err(err)?.unwrap_or_else(Rules::default_patterns);
    let suggested = ["Documents", "Desktop", "Downloads"]
        .iter()
        .map(|d| format!("{}/{d}", state.home))
        .filter(|p| std::path::Path::new(p).is_dir())
        .collect();
    Ok(Settings { folders, ignore, suggested, total: index.count().map_err(err)? })
}

#[tauri::command]
fn add_folder(app: AppHandle, state: State<AppState>, path: String) -> Result<(), String> {
    let path = std::fs::canonicalize(&path).map_err(err)?.to_str().ok_or("the folder's path is not valid UTF-8")?.to_string();
    let existing = state.reader.lock().unwrap().folders().map_err(err)?;
    if existing.iter().any(|f| path == *f || path.starts_with(&format!("{}/", f.trim_end_matches('/')))) {
        return Err(format!("{path} is already searched, as part of a folder you added"));
    }
    state.reader.lock().unwrap().add_folder(&path, now()).map_err(err)?;
    if let Some(watch) = state.watch.lock().unwrap().as_mut() {
        watch.add_root(&path).map_err(err)?;
    }
    start_scan(&app, &state, path);
    Ok(())
}

#[tauri::command]
fn remove_folder(state: State<AppState>, path: String) -> Result<(), String> {
    if let Some(watch) = state.watch.lock().unwrap().as_mut() {
        watch.remove_root(&path).map_err(err)?;
    }
    let mut index = state.reader.lock().unwrap();
    index.remove_folder(&path).map_err(err)?;
    index.remove_root(&path).map_err(err)?;
    Ok(())
}

#[tauri::command]
fn set_ignore(app: AppHandle, state: State<AppState>, patterns: Vec<String>) -> Result<(), String> {
    let rules = Rules::new(&patterns, &state.home).map_err(err)?;
    state.reader.lock().unwrap().set_ignore_patterns(&patterns).map_err(err)?;
    *state.rules.lock().unwrap() = rules;
    // Rescan so newly ignored files leave the index and unignored ones join.
    for folder in state.reader.lock().unwrap().folders().map_err(err)? {
        start_scan(&app, &state, folder);
    }
    Ok(())
}

#[tauri::command]
fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener().open_path(path, None::<&str>).map_err(err)
}

#[tauri::command]
fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener().reveal_item_in_dir(path).map_err(err)
}

/// Scans one folder in the background, reporting progress as events.
fn start_scan(app: &AppHandle, state: &AppState, root: String) {
    if !state.scanning.lock().unwrap().insert(root.clone()) {
        return;
    }
    let app = app.clone();
    let db_path = state.db_path.clone();
    let home = state.home.clone();
    let scanning = state.scanning.clone();
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<ScanStats> {
            let mut index = Index::open(&db_path)?;
            // Its own copy of the rules as saved, so a long scan holds no
            // lock the watcher needs.
            let patterns = index.ignore_patterns()?.unwrap_or_else(Rules::default_patterns);
            let rules = Rules::new(&patterns, &home)?;
            crawl::scan(&mut index, &root, &rules, |indexed| {
                let _ = app.emit("scan-progress", Progress { root: root.clone(), indexed });
            })
        })();
        scanning.lock().unwrap().remove(&root);
        match result {
            Ok(stats) => {
                log::info!("scanned {}: {} indexed in {} ms", stats.root, stats.indexed, stats.millis);
                let _ = app.emit("scan-done", stats);
            }
            Err(e) => {
                let _ = app.emit("scan-error", format!("{root}: {e:#}"));
            }
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let home = std::env::var("HOME").unwrap_or_default();
            let data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data)?;
            let db_path = data.join("sifs.db");
            let index = Index::open(&db_path)?;
            let patterns = index.ignore_patterns()?.unwrap_or_else(Rules::default_patterns);
            let rules = Arc::new(Mutex::new(Rules::new(&patterns, &home)?));
            let folders = index.folders()?;
            let handle = app.handle().clone();
            let watch = Watch::start(db_path.clone(), folders.clone(), rules.clone(), move |applied| {
                if applied > 0 {
                    let _ = handle.emit("index-changed", applied);
                }
            })?;
            let state = AppState {
                db_path,
                home,
                reader: Mutex::new(index),
                rules,
                watch: Mutex::new(Some(watch)),
                scanning: Arc::new(Mutex::new(HashSet::new())),
            };
            // Catch up on what changed while the app was closed.
            for folder in folders {
                start_scan(app.handle(), &state, folder);
            }
            app.manage(state);

            let show = MenuItem::with_id(app, "show", "Show SIFS", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().ok_or("no app icon")?)
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } = event {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps SIFS running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            search_files,
            settings,
            add_folder,
            remove_folder,
            set_ignore,
            open_path,
            reveal_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running SIFS");
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
