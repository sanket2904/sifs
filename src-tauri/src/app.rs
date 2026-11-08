//! The desktop app: Tauri commands over the index, background scans, the
//! watcher, reading inside files, and the tray.

use crate::extract::{self, Registry, Waker, Worker, WorkerOptions};
use crate::index::db::Index;
use crate::index::folders::{Event, Folders};
use crate::index::ignore::Rules;
use crate::index::search::{self, Hit};
use crate::index::stop::Until;
use crate::index::watch::Watch;
use serde::Serialize;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_opener::OpenerExt;

struct AppState {
    home: String,
    /// The connection searches and settings use. The crawler and the
    /// watcher open their own.
    reader: Mutex<Index>,
    /// The connection searches over contents use, so a slow one never
    /// holds up names or settings.
    contents_reader: Mutex<Index>,
    /// The newest search_all's number: an older one still running, or
    /// waiting for contents_reader, stops.
    searches: Arc<AtomicU64>,
    /// The same for search_files.
    name_searches: Arc<AtomicU64>,
    rules: Arc<Mutex<Rules>>,
    watch: Mutex<Option<Watch>>,
    /// Scans and removals of folders, in the background.
    folders: Folders,
    /// Reads inside files in the background; paused while a scan runs.
    extract: Worker,
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
    contents: extract::Progress,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Names only, off the main thread (a short query compares most names),
/// within NAME_TIME from now. A newer call stops this one.
#[tauri::command]
async fn search_files(app: AppHandle, query: String, limit: Option<usize>) -> Result<Vec<Hit>, String> {
    let until = Until::after(search::NAME_TIME).or_newer(&app.state::<AppState>().name_searches);
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let index = state.reader.lock().unwrap();
        search::search_until(&index, &query, limit.unwrap_or(60).min(500), now(), &until).map_err(err)
    })
    .await
    .map_err(err)?
}

/// Names and contents ranked together, off the main thread, within
/// CONTENT_TIME from now (waiting for the connection included); past it,
/// what was found, marked. A newer call stops this one.
#[tauri::command]
async fn search_all(app: AppHandle, query: String, limit: Option<usize>) -> Result<extract::find::Results, String> {
    let until = Until::after(extract::find::CONTENT_TIME).or_newer(&app.state::<AppState>().searches);
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let index = state.contents_reader.lock().unwrap();
        extract::find::search_all(&index, &query, limit.unwrap_or(60).min(500), now(), &until).map_err(err)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
fn settings(state: State<AppState>) -> Result<Settings, String> {
    let index = state.reader.lock().unwrap();
    let scanning = state.folders.scanning();
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
    let contents = extract::store::progress(&index).map_err(err)?;
    Ok(Settings { folders, ignore, suggested, total: index.count().map_err(err)?, contents })
}

/// The files that couldn't be read inside, or were skipped, and why.
#[tauri::command]
fn content_failures(state: State<AppState>, limit: Option<usize>) -> Result<Vec<extract::Failure>, String> {
    let index = state.reader.lock().unwrap();
    extract::store::failures(&index, limit.unwrap_or(100).min(1000)).map_err(err)
}

#[tauri::command]
fn add_folder(state: State<AppState>, path: String) -> Result<(), String> {
    let path = std::fs::canonicalize(&path).map_err(err)?.to_str().ok_or("the folder's path is not valid UTF-8")?.to_string();
    let inside = state.folders.add(&mut state.reader.lock().unwrap(), &path, now()).map_err(err)?;
    let mut watched = Ok(());
    if let Some(watch) = state.watch.lock().unwrap().as_mut() {
        // Folders taken into this one stop being watched on their own
        // first, so unwatching them can't drop the new watch's paths.
        for child in &inside {
            if let Err(e) = watch.remove_root(child) {
                log::warn!("unwatching {child}: {e:#}");
            }
        }
        watched = watch.add_root(&path);
    }
    // Scanned even if it can't be watched (inotify's limit, say): its files
    // are found, only later changes are missed until SIFS starts again.
    // The window reloads its settings after adding, so it shows only this one.
    state.folders.scan(path.clone());
    watched.map_err(|e| {
        log::warn!("watching {path}: {e:#}");
        format!("{path} was added, but changes in it won't be noticed until SIFS restarts: {e:#}")
    })
}

#[tauri::command]
fn remove_folder(state: State<AppState>, path: String) -> Result<(), String> {
    let forgot = state.folders.remove(&mut state.reader.lock().unwrap(), &path).map_err(err)?;
    // Only a folder is unwatched: a path taken into a folder around it is
    // watched through that folder, and unwatching it would drop those watches.
    if forgot {
        if let Some(watch) = state.watch.lock().unwrap().as_mut() {
            watch.remove_root(&path).map_err(err)?;
        }
    }
    Ok(())
}

#[tauri::command]
fn set_ignore(state: State<AppState>, patterns: Vec<String>) -> Result<(), String> {
    let rules = Rules::new(&patterns, &state.home).map_err(err)?;
    state.reader.lock().unwrap().set_ignore_patterns(&patterns).map_err(err)?;
    *state.rules.lock().unwrap() = rules;
    // Rescan so newly ignored files leave the index and unignored ones join.
    for folder in state.reader.lock().unwrap().folders().map_err(err)? {
        state.folders.scan(folder);
    }
    Ok(())
}

#[tauri::command]
fn open_path(app: AppHandle, state: State<AppState>, path: String) -> Result<(), String> {
    // If it is waiting to be read inside, it is read next.
    state.extract.opened(&path);
    app.opener().open_path(path, None::<&str>).map_err(err)
}

#[tauri::command]
fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener().reveal_item_in_dir(path).map_err(err)
}

/// Passes the background scans' and removals' news on to the window, and
/// wakes the reader of files when a scan ends (it waits for scans).
fn folder_event(app: &AppHandle, reading: &OnceLock<Waker>, event: Event) {
    match event {
        Event::Progress { root, indexed } => {
            let _ = app.emit("scan-progress", Progress { root, indexed });
        }
        Event::Scanned(stats) => {
            log::info!("scanned {}: {} indexed in {} ms", stats.root, stats.indexed, stats.millis);
            let _ = app.emit("scan-done", stats);
        }
        Event::ScanFailed { root, error } => {
            let _ = app.emit("scan-error", format!("{root}: {error}"));
        }
        Event::Removed { removed, .. } => {
            let _ = app.emit("index-changed", removed);
        }
    }
    if let Some(reading) = reading.get() {
        reading.wake();
    }
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
            let mut index = Index::open(&db_path)?;
            // Before the watcher reads the folders: folders inside others
            // (allowed in phase 1) are taken into them. resume does it again.
            index.tidy_roots()?;
            let patterns = index.ignore_patterns()?.unwrap_or_else(Rules::default_patterns);
            let rules = Arc::new(Mutex::new(Rules::new(&patterns, &home)?));
            let roots = index.folders()?;
            // The readers' extensions are in the index before anything is
            // scanned, so the queue's triggers see them.
            let registry = Arc::new(Registry::platform());
            extract::store::register_kinds(&mut index, &registry)?;
            let reading: Arc<OnceLock<Waker>> = Arc::default();
            let (handle, woken) = (app.handle().clone(), reading.clone());
            let folders = Folders::new(db_path.clone(), home.clone(), Duration::from_secs(5), move |event| {
                folder_event(&handle, &woken, event)
            });
            let (busy, progress) = (folders.clone(), app.handle().clone());
            let worker = Worker::start(
                db_path.clone(),
                registry,
                WorkerOptions {
                    pause: Arc::new(move || busy.is_scanning()),
                    on_progress: Arc::new(move |p: &extract::Progress| {
                        let _ = progress.emit("extract-progress", p);
                    }),
                    ..WorkerOptions::default()
                },
            )?;
            let _ = reading.set(worker.waker());
            let handle = app.handle().clone();
            let reading = worker.waker();
            // Without a watcher SIFS still scans; it just misses changes.
            let watch = Watch::start(db_path.clone(), roots, rules.clone(), move |applied| {
                if applied > 0 {
                    let _ = handle.emit("index-changed", applied);
                    reading.wake();
                }
            })
            .map_err(|e| log::warn!("watching for changes: {e:#}"))
            .ok();
            // Catch up on what changed while the app was closed, and finish
            // removing folders it was removing when it stopped.
            folders.resume(&mut index)?;
            let contents_reader = Mutex::new(Index::open(&db_path)?);
            let state = AppState {
                home,
                reader: Mutex::new(index),
                contents_reader,
                searches: Arc::default(),
                name_searches: Arc::default(),
                rules,
                watch: Mutex::new(watch),
                folders,
                extract: worker,
            };
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
            search_all,
            settings,
            add_folder,
            remove_folder,
            set_ignore,
            open_path,
            reveal_path,
            content_failures
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
