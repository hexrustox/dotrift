//! Filesystem path helpers shared across commands.

use std::{
    fs,
    path::{Path, PathBuf},
};

use miette::{Result, WrapErr, miette};
use normalize_path::NormalizePath;

pub(crate) fn ensure_absolute(path: &Path) -> Result<std::path::PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|error| miette!(error))
            .wrap_err("cannot resolve the current directory")?
            .join(path))
    }
}

pub(crate) fn ensure_source_dir(path: &Path) -> Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(miette!(
            "source directory `{}` is not a directory",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(miette!(
            "source directory `{}` does not exist",
            path.display()
        )),
        Err(error) => {
            Err::<(), _>(miette!(error))
                .wrap_err_with(|| format!("cannot access source directory `{}`", path.display()))?;
            unreachable!()
        }
    }
}

pub(crate) fn prettify_path(path: &Path) -> PathBuf {
    let normalized = path.normalize();
    if let Some(home) = dirs::home_dir() {
        let home = home.normalize();
        if let Ok(stripped) = normalized.strip_prefix(&home) {
            if stripped.as_os_str().is_empty() {
                return PathBuf::from("~");
            }
            let mut result = PathBuf::from("~");
            result.extend(stripped.iter());
            return result;
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use test_case::test_case;

    use super::*;

    #[test]
    fn ensure_absolute_keeps_an_absolute_path() {
        let absolute = std::env::current_dir().unwrap();
        assert_eq!(ensure_absolute(&absolute).unwrap(), absolute);
    }

    #[test]
    fn ensure_absolute_joins_a_relative_path_onto_the_current_directory() {
        let resolved = ensure_absolute(Path::new("file1")).unwrap();
        let current = std::env::current_dir().unwrap();
        assert_eq!(resolved, current.join("file1"));
    }

    #[test]
    fn ensure_source_dir_accepts_a_directory() {
        let temp = tempfile::tempdir().expect("cannot create temp dir");
        fs::create_dir(temp.path().join("dir1")).unwrap();
        ensure_source_dir(&temp.path().join("dir1")).unwrap();
    }

    #[test_case("file1"; "not_a_directory")]
    #[test_case("missing1"; "missing")]
    fn ensure_source_dir_rejects(source_name: &str) {
        let temp = tempfile::tempdir().expect("cannot create temp dir");
        fs::write(temp.path().join("file1"), "content1").unwrap();
        let path = temp.path().join(source_name);
        let error = ensure_source_dir(&path).unwrap_err().to_string();
        assert!(
            error.contains(&format!("source directory `{}`", path.display())),
            "{error}"
        );
    }

    #[test]
    fn ensure_source_dir_errors_for_an_unreadable_directory() {
        if unsafe { libc::geteuid() } == 0 {
            // Root bypasses directory permissions, so the access error is
            // unreachable.
            return;
        }
        let temp = tempfile::tempdir().expect("cannot create temp dir");
        let path = temp.path().join("dir1");
        fs::create_dir(&path).unwrap();
        let permissions = || {
            let mut permissions = fs::metadata(temp.path()).unwrap().permissions();
            permissions.set_mode(0o000);
            fs::set_permissions(temp.path(), permissions).unwrap();
        };
        let restore = || {
            let mut permissions = fs::metadata(temp.path()).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(temp.path(), permissions).unwrap();
        };
        permissions();
        let error = ensure_source_dir(&path).unwrap_err().to_string();
        restore();
        assert!(
            error.contains(&format!(
                "cannot access source directory `{}`",
                path.display()
            )),
            "{error}"
        );
    }

    #[test]
    fn prettify_path_shortens_a_home_relative_path() {
        let home = dirs::home_dir().unwrap().normalize();
        let pretty = prettify_path(&home.join("file1"));
        assert_eq!(pretty, PathBuf::from("~").join("file1"));
    }

    #[test]
    fn prettify_path_shortens_the_home_itself_to_a_tilde() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(prettify_path(&home), PathBuf::from("~"));
    }

    #[test]
    fn prettify_path_keeps_a_path_outside_the_home() {
        let temp = tempfile::tempdir().expect("cannot create temp dir");
        let nested = temp.path().join("dir1").join("file1");
        assert_eq!(prettify_path(&nested), nested);
    }
}
