//! Where the app keeps its data. One rule, decided before the app starts, so
//! a development build can never open the installed app's index.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Set to an absolute path, the app keeps its data there instead.
pub const DATA_DIR_VAR: &str = "SIFS_DATA_DIR";
/// The installed app's identifier (`tauri.conf.json`): its data folder's name.
pub const INSTALLED: &str = "com.sanket2904.sifs";
/// The development identifier (`tauri.dev.conf.json`), and the name of a
/// development build's data folder, beside the installed app's.
pub const DEVELOPMENT: &str = "com.sanket2904.sifs.dev";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Build {
    /// `tauri build`: the installed app.
    Release,
    /// Any debug build: `tauri dev`, `tauri build --debug`, the tests.
    Development,
}

impl Build {
    pub const THIS: Build = if cfg!(debug_assertions) { Build::Development } else { Build::Release };
}

/// The folder the app keeps its data in. Only [`data_dir`] makes one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataDir(PathBuf);

impl DataDir {
    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn index(&self) -> PathBuf {
        self.0.join("sifs.db")
    }
}

/// Why the app does not start.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The variable is empty or not an absolute path.
    NotAbsolute(OsString),
    /// A development build was pointed at a folder named [`INSTALLED`], or
    /// inside one.
    Installed(PathBuf),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NotAbsolute(value) => write!(
                f,
                "{DATA_DIR_VAR} must be an absolute path, not {value:?}; SIFS does not fall back to its usual folder"
            ),
            Refusal::Installed(chosen) => write!(
                f,
                "{DATA_DIR_VAR} is {}, but a development build never uses a folder named {INSTALLED} or anything inside one, wherever it is; choose a folder with another name",
                chosen.display()
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// The data folder for `build`, given `system`, the system's data folder for
/// the identifier this build runs with (Tauri's `app_data_dir`), and the
/// value of SIFS_DATA_DIR if it is set.
pub fn data_dir(build: Build, system: &Path, var: Option<&OsStr>) -> Result<DataDir, Refusal> {
    let chosen = match (var, build) {
        (Some(value), _) if Path::new(value).is_absolute() => PathBuf::from(value),
        (Some(value), _) => return Err(Refusal::NotAbsolute(value.to_owned())),
        (None, Build::Release) => return Ok(DataDir(system.to_path_buf())),
        (None, Build::Development) => system.with_file_name(DEVELOPMENT),
    };
    if build == Build::Development && named_installed(&chosen) {
        return Err(Refusal::Installed(chosen));
    }
    Ok(DataDir(chosen))
}

/// Whether `path` is a folder named [`INSTALLED`] or inside one, anywhere.
/// The name decides, not the parents: they may be spelled any way the disk
/// accepts (an accented name in either Unicode form), while the name is plain
/// ASCII. `.` and `..` are resolved, and case is folded upper then lower, so
/// `ſ` and the Kelvin sign, which the Mac's disk takes for `s` and `k`, fold.
/// The disk is not read, so a symlink under another name that leads to the
/// installed folder is not seen: seeing it means resolving the installed
/// app's real folder, and such a link is made on purpose, never by a typo in
/// the variable.
fn named_installed(path: &Path) -> bool {
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => names.push(name.to_string_lossy().to_uppercase().to_lowercase()),
            Component::ParentDir => {
                names.pop();
            }
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    names.iter().any(|name| name == INSTALLED)
}

#[cfg(test)]
#[path = "data_tests.rs"]
mod tests;
