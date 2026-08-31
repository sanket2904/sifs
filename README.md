# SIFS

SIFS is a Windows desktop file-search application built with Rust, Tauri, and SvelteKit. It indexes file and directory names from available Windows drives into serialized tries, then uses prefix lookup to return matching paths quickly.

The project combines a native Rust indexing engine with a lightweight web-based desktop interface.

## Features

- Indexes all detected drive letters on startup
- Stores one serialized trie shard per leading byte
- Searches cached file names by prefix
- Watches the filesystem and updates the index for create, rename, and delete events
- Opens a selected result in Windows Explorer
- Runs from the Windows system tray and hides instead of exiting when the window is closed

## How it works

The Rust backend walks each available Windows drive and builds a trie of file and directory names. Each top-level trie child is serialized with `bincode` into a drive-specific cache directory such as `./C`. The Tauri command `search_files` loads the relevant shard and returns matching paths to the Svelte interface.

Filesystem changes are tracked with `notify`, allowing the cache to be updated while the application is running. Cache directories are removed when the application is closed through the tray's **Quit** action.

## Requirements

- Windows (the current implementation depends on drive letters and `explorer.exe`)
- Rust 1.60 or newer
- Node.js and npm
- The platform prerequisites for [Tauri 1](https://v1.tauri.app/v1/guides/getting-started/prerequisites/)

## Development

Install the frontend dependencies:

```bash
npm install
```

Install the Tauri CLI if it is not already available:

```bash
cargo install tauri-cli --version '^1.6'
```

Start the desktop application in development mode:

```bash
cargo tauri dev
```

## Build

Create a production bundle with:

```bash
cargo tauri build
```

## Usage

1. Start SIFS and wait for the initial drive scan to finish.
2. Enter the beginning of a file or directory name in the search box.
3. Press **Enter** to search.
4. Double-click a result to open it with Windows Explorer.
5. Use the system-tray menu to hide, show, or quit the application.

## Project structure

```text
src/                     SvelteKit user interface
src-tauri/src/main.rs    Tauri commands, drive indexing, and filesystem watcher
src-tauri/src/trie.rs    Trie storage, lookup, insertion, and removal
src-tauri/tauri.conf.json
```

## Engineering roadmap

- Add case-insensitive, fuzzy, and substring search modes
- Move the initial drive scan to a background task with progress reporting
- Store index data in the platform application-data directory
- Handle non-UTF-8 paths without conversion panics
- Add keyboard navigation and search-as-you-type
- Add automated tests for trie persistence and filesystem updates

## Status

SIFS is a Windows-focused prototype that demonstrates persistent trie indexing, live filesystem updates, and a Rust-to-Svelte desktop integration.
