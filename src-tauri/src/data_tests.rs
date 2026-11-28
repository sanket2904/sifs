use super::*;

const SUPPORT: &str = "/Users/someone/Library/Application Support";

fn system(identifier: &str) -> PathBuf {
    Path::new(SUPPORT).join(identifier)
}

fn installed() -> PathBuf {
    system(INSTALLED)
}

fn dir(build: Build, system: &Path, var: Option<&str>) -> Result<DataDir, Refusal> {
    data_dir(build, system, var.map(OsStr::new))
}

#[test]
fn the_installed_app_keeps_its_folder_and_index_to_the_byte() {
    let data = dir(Build::Release, &installed(), None).unwrap();
    // What the app opened before: app_data_dir() joined with sifs.db.
    let before = installed().join("sifs.db");
    assert_eq!(data.index().as_os_str(), before.as_os_str());
    assert_eq!(data.index().as_os_str(), "/Users/someone/Library/Application Support/com.sanket2904.sifs/sifs.db");
    assert_eq!(data.path().as_os_str(), installed().as_os_str());
}

#[test]
fn the_variable_is_used_as_given_in_any_build() {
    for build in [Build::Release, Build::Development] {
        let data = dir(build, &installed(), Some("/tmp/sifs-try/")).unwrap();
        assert_eq!(data.path().as_os_str(), "/tmp/sifs-try/", "{build:?}");
        assert_eq!(data.index(), PathBuf::from("/tmp/sifs-try/sifs.db"), "{build:?}");
    }
}

#[test]
fn a_bad_variable_is_refused_and_never_falls_back() {
    for build in [Build::Release, Build::Development] {
        for value in ["", "data", "./data", "~/data", "../data", "Library/Application Support/com.sanket2904.sifs"] {
            assert_eq!(
                dir(build, &installed(), Some(value)),
                Err(Refusal::NotAbsolute(value.into())),
                "{build:?} {value:?}"
            );
        }
    }
}

#[test]
fn a_development_build_defaults_beside_the_installed_folder() {
    let beside = system(DEVELOPMENT);
    // Run with tauri.dev.conf.json, or without it (plain `tauri dev`).
    for system in [system(DEVELOPMENT), installed()] {
        let data = dir(Build::Development, &system, None).unwrap();
        assert_eq!(data.path().as_os_str(), beside.as_os_str(), "{system:?}");
        assert_ne!(data.path(), installed());
    }
}

#[test]
fn a_development_build_refuses_the_installed_folder_however_spelled() {
    let spellings = [
        "/Users/someone/Library/Application Support/com.sanket2904.sifs",
        "/Users/someone/Library/Application Support/com.sanket2904.sifs/",
        "/Users/someone/Library/Application Support/COM.Sanket2904.SIFS",
        "/users/SOMEONE/library/application support/com.sanket2904.sifs",
        "/Users/someone/Library/Application Support/com.sanket2904.sifs/old",
        "/Users/someone/Library/Application Support/com.sanket2904.sifs.dev/../com.sanket2904.sifs",
        "/Users/someone/Library/Application Support/x/../com.sanket2904.sifs/./",
        "/Users//someone/./Library/Application Support//com.sanket2904.sifs",
    ];
    for system in [system(DEVELOPMENT), installed()] {
        for value in spellings {
            let refused = dir(Build::Development, &system, Some(value));
            assert!(matches!(refused, Err(Refusal::Installed { .. })), "{value:?} with {system:?}: {refused:?}");
        }
    }
}

#[test]
fn a_development_build_takes_folders_that_only_look_alike() {
    for value in [
        "/Users/someone/Library/Application Support/com.sanket2904.sifs.dev",
        "/Users/someone/Library/Application Support/com.sanket2904.sifs-copy",
        "/Users/someone/Library/Application Support",
        "/tmp/com.sanket2904.sifs",
    ] {
        let data = dir(Build::Development, &system(DEVELOPMENT), Some(value)).unwrap();
        assert_eq!(data.path().as_os_str(), value);
    }
}

#[test]
fn a_refusal_says_why_and_names_the_variable() {
    let relative = dir(Build::Release, &installed(), Some("data")).unwrap_err().to_string();
    assert!(relative.contains("SIFS_DATA_DIR") && relative.contains("\"data\""), "{relative}");
    let installed = dir(Build::Development, &installed(), Some(installed().to_str().unwrap())).unwrap_err().to_string();
    assert!(installed.contains("SIFS_DATA_DIR") && installed.contains("development build"), "{installed}");
}

#[test]
fn debug_builds_are_development_builds() {
    let expected = if cfg!(debug_assertions) { Build::Development } else { Build::Release };
    assert_eq!(Build::THIS, expected);
}

fn identifier(config: &str) -> String {
    let config: serde_json::Value = serde_json::from_str(config).unwrap();
    config["identifier"].as_str().unwrap().to_owned()
}

#[test]
fn the_names_are_the_identifiers_in_the_tauri_configs() {
    assert_eq!(identifier(include_str!("../tauri.conf.json")), INSTALLED);
    assert_eq!(identifier(include_str!("../tauri.dev.conf.json")), DEVELOPMENT);
    let package: serde_json::Value = serde_json::from_str(include_str!("../../package.json")).unwrap();
    assert_eq!(package["scripts"]["app"], "tauri dev --config src-tauri/tauri.dev.conf.json");
}

#[test]
fn the_app_asks_for_one_per_app_folder_and_hands_it_to_data_dir() {
    let app = include_str!("app.rs");
    assert_eq!(app.matches("app_data_dir()").count(), 1, "one ask for the system folder");
    for other in ["app_local_data_dir", "app_config_dir", "app_cache_dir", "app_log_dir", "home_dir", "sifs.db"] {
        assert!(!app.contains(other), "app.rs names {other}; per-app paths come from data.rs");
    }
    assert_eq!(app.matches("data::data_dir(").count(), 1, "the system folder goes through data_dir");
    assert_eq!(app.matches("DATA_DIR_VAR").count(), 1, "the variable is read in one place");
}
