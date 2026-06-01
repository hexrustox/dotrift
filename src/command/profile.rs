use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use miette::{Result, bail};
use templater::value::Value;

use crate::{cli::GlobalFlags, db::Db, path::data_path, templater::data::TemplateData};
use crate::{eoutput, output};

pub fn list(global: &GlobalFlags, db_path: &Path) -> Result<()> {
    let source_dir = global.source()?;
    let data = TemplateData::read(&source_dir)?;

    if data.profile.is_empty() {
        bail!(
            "no profiles defined in `{}`",
            data_path(&source_dir).display()
        );
    }

    let db = Db::init(db_path)?;
    let active_profiles: HashSet<String> = db
        .get_active_profiles()?
        .into_iter()
        .map(|p| p.name)
        .collect();

    for profile in data.profile.keys() {
        let active = active_profiles.contains(profile);
        if active {
            output!("{profile} (active)");
        } else {
            output!("{profile}");
        }
    }

    Ok(())
}

pub fn activate(global: &GlobalFlags, db_path: &Path, name: &str) -> Result<()> {
    let source_dir = global.source()?;
    let data = TemplateData::read(&source_dir)?;

    if !data.profile.contains_key(name) {
        bail!(
            "profile `{name}` is not defined in `{}`",
            data_path(&source_dir).display()
        );
    }

    let db = Db::init(db_path)?;
    db.activate_profile(name)?;
    eoutput!("profile `{name}` activated");
    Ok(())
}

pub fn deactivate(db_path: &Path, name: &str) -> Result<()> {
    let db = Db::init(db_path)?;
    db.deactivate_profile(name)?;
    eoutput!("profile `{name}` deactivated");
    Ok(())
}

pub fn show(global: &GlobalFlags, db_path: &Path) -> Result<()> {
    let source_dir = global.source()?;
    let mut data = TemplateData::read(&source_dir)?;
    let db = Db::init(db_path)?;
    let active_profiles = db.get_active_profiles()?;

    let mut ctx: BTreeMap<String, Value> = BTreeMap::new();

    for (k, v) in data.variable {
        ctx.insert(k, v);
    }

    for profile in active_profiles {
        if let Some(vars) = data.profile.remove(&profile.name) {
            for (k, v) in vars {
                ctx.insert(k, v);
            }
        }
    }

    if ctx.is_empty() {
        return Ok(());
    }

    let max_key_len = ctx.keys().map(|k| k.len()).max().unwrap_or(0);
    for (key, val) in &ctx {
        let mut buf = Vec::new();
        val.write_to(&mut buf).unwrap();
        output!(
            "{key:<width$}  {}",
            String::from_utf8_lossy(&buf),
            width = max_key_len
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::cli::GlobalFlags;
    use tempfile::TempDir;

    use super::*;

    fn setup_profile(data_toml: &str) -> (TempDir, PathBuf, PathBuf) {
        let temp_dir = tempfile::tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let db_path = temp_dir.path().join("db");
        std::fs::create_dir(&source_dir).unwrap();
        if !data_toml.is_empty() {
            std::fs::write(source_dir.join("dotrift_data.toml"), data_toml).unwrap();
        }
        (temp_dir, source_dir, db_path)
    }

    fn flags(source_dir: &std::path::Path) -> GlobalFlags {
        GlobalFlags::new(Some(source_dir.to_path_buf()), None, None)
    }

    #[test]
    fn test_profile_list_empty() {
        let (_tmp, source_dir, db_path) = setup_profile("");
        list(&flags(&source_dir), &db_path).unwrap_err();
    }

    #[test]
    fn test_profile_list_no_active() {
        let data = r#"
[profile.a]

[profile.b]
"#;
        let (tmp, source_dir, db_path) = setup_profile(data);
        list(&flags(&source_dir), &db_path).unwrap();
        crate::command::util::assert_captured_output("profile_list_no_active", tmp.path());
    }

    #[test]
    fn test_profile_list_active() {
        let data = r#"
[profile.a]

[profile.b]

[profile.c]
"#;
        let (tmp, source_dir, db_path) = setup_profile(data);
        activate(&flags(&source_dir), &db_path, "a").unwrap();
        crate::output::test_capture::take_all();
        list(&flags(&source_dir), &db_path).unwrap();
        crate::command::util::assert_captured_output("profile_list_active", tmp.path());
    }

    #[test]
    fn test_profile_activate_valid() {
        let data = r#"
[profile.a]
"#;
        let (tmp, source_dir, db_path) = setup_profile(data);
        activate(&flags(&source_dir), &db_path, "a").unwrap();
        crate::command::util::assert_captured_output("profile_activate_valid", tmp.path());
    }

    #[test]
    fn test_profile_activate_invalid() {
        let (_tmp, source_dir, db_path) = setup_profile("");
        activate(&flags(&source_dir), &db_path, "nope").unwrap_err();
    }

    #[test]
    fn test_profile_deactivate() {
        let data = r#"
[profile.a]
"#;
        let (tmp, source_dir, db_path) = setup_profile(data);
        activate(&flags(&source_dir), &db_path, "a").unwrap();
        crate::output::test_capture::take_all();
        deactivate(&db_path, "a").unwrap();
        crate::command::util::assert_captured_output("profile_deactivate", tmp.path());
    }

    #[test]
    fn test_profile_deactivate_nonexistent() {
        let (_tmp, _source_dir, db_path) = setup_profile("");
        deactivate(&db_path, "nope").unwrap_err();
    }

    #[test]
    fn test_profile_show_base_only() {
        let data = r#"
[variable]
name = "Alice"
age = 30
"#;
        let (tmp, source_dir, db_path) = setup_profile(data);
        show(&flags(&source_dir), &db_path).unwrap();
        crate::command::util::assert_captured_output("profile_show_base_only", tmp.path());
    }

    #[test]
    fn test_profile_show_with_profiles() {
        let data = r#"
[variable]
name = "Alice"
editor = "nano"

[profile.work]
email = "work@example.com"

[profile.personal]
editor = "vim"
gh_user = "me"
"#;
        let (tmp, source_dir, db_path) = setup_profile(data);
        activate(&flags(&source_dir), &db_path, "work").unwrap();
        activate(&flags(&source_dir), &db_path, "personal").unwrap();
        crate::output::test_capture::take_all();
        show(&flags(&source_dir), &db_path).unwrap();
        crate::command::util::assert_captured_output("profile_show_with_profiles", tmp.path());
    }
}
