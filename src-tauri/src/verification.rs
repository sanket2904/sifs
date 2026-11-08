//! Verification support (.pi/skills/verify). `sifs-verify launch` starts a
//! test instance with SIFS_VERIFY_BACKGROUND=1 and a fake HOME. Without that
//! variable nothing here changes what SIFS does.

use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Whether this process is a test instance the verify skill started.
pub fn instance() -> bool {
    std::env::var_os("SIFS_VERIFY_BACKGROUND").is_some_and(|v| v == "1")
}

/// A test instance searches only inside its fake home. The system folder
/// picker opens in the real home whatever HOME says, so a folder picked there
/// by mistake is refused before anything scans it. Ancestors are compared by
/// device and inode, so case and /System/Volumes/Data spellings agree.
pub fn check_folder(instance: bool, home: &str, folder: &Path) -> Result<(), String> {
    if !instance {
        return Ok(());
    }
    let id = |path: &Path| std::fs::metadata(path).map(|m| (m.dev(), m.ino()));
    let home_id = id(Path::new(home)).map_err(|e| format!("verification instance: its HOME {home:?}: {e}"))?;
    let folder = std::fs::canonicalize(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    for ancestor in folder.ancestors() {
        if id(ancestor).map_err(|e| format!("{}: {e}", ancestor.display()))? == home_id {
            return Ok(());
        }
    }
    Err(format!("{} is outside {home}: a verification instance searches only inside its fake home", folder.display()))
}

#[cfg(test)]
mod tests {
    use super::check_folder;
    use std::fs;
    use std::path::PathBuf;

    struct Dirs {
        _tmp: tempfile::TempDir,
        home: PathBuf,
        outside: PathBuf,
    }

    fn dirs() -> Dirs {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let outside = tmp.path().join("Documents");
        fs::create_dir_all(home.join("Documents")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(tmp.path().join("home-other")).unwrap();
        std::os::unix::fs::symlink(&outside, home.join("link")).unwrap();
        Dirs { _tmp: tmp, home, outside }
    }

    #[test]
    fn a_verification_instance_adds_only_folders_inside_its_home() {
        let d = dirs();
        let home = d.home.to_str().unwrap();
        assert_eq!(check_folder(true, home, &d.home.join("Documents")), Ok(()));
        assert_eq!(check_folder(true, home, &d.home), Ok(()));
        let refused = [
            d.outside.clone(),
            d.home.with_file_name("home-other"),
            d.home.join("link"),
            d.home.join("..").join("Documents"),
        ];
        for folder in refused {
            assert!(check_folder(true, home, &folder).is_err(), "{} was accepted", folder.display());
        }
    }

    #[test]
    fn every_spelling_of_a_folder_inside_the_home_gets_the_same_answer() {
        let d = dirs();
        let home = fs::canonicalize(&d.home).unwrap();
        let documents = home.join("Documents");
        let mut spellings = vec![];
        let upper = PathBuf::from(documents.to_str().unwrap().to_uppercase());
        if upper.exists() {
            spellings.push(upper);
        }
        let data = PathBuf::from(format!("/System/Volumes/Data{}", documents.display()));
        if data.exists() {
            spellings.push(data);
        }
        for folder in &spellings {
            assert_eq!(check_folder(true, home.to_str().unwrap(), folder), Ok(()), "{} was refused", folder.display());
        }
        let outside = PathBuf::from(format!("/System/Volumes/Data{}", fs::canonicalize(&d.outside).unwrap().display()));
        if outside.exists() {
            assert!(check_folder(true, home.to_str().unwrap(), &outside).is_err());
        }
    }

    #[test]
    fn a_normal_instance_adds_any_folder() {
        let d = dirs();
        assert_eq!(check_folder(false, d.home.to_str().unwrap(), &d.outside), Ok(()));
    }
}
