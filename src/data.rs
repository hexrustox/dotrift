use std::{collections::BTreeMap, fs, path::Path};

use miette::{Result, WrapErr, miette};
use serde::Deserialize;
use templater::value::Value;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DataFile {
    pub variable: BTreeMap<String, Value>,
    pub profile: BTreeMap<String, BTreeMap<String, Value>>,
}

impl DataFile {
    pub fn read(source: &Path) -> Result<Self> {
        if !source.is_dir() {
            return Err(miette!(
                "source directory `{}` does not exist",
                source.display()
            ));
        }
        let path = source.join("dotrift_data.toml");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(miette!(error))
                    .wrap_err_with(|| format!("cannot read `{}`", path.display()));
            }
        };
        toml::from_str(&text)
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot parse `{}`", path.display()))
    }

    pub fn context(&self, active: &[(String, i64)]) -> BTreeMap<String, Value> {
        let mut context = self.variable.clone();
        let mut active = active.to_vec();
        active.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        for (name, _) in active {
            if let Some(profile) = self.profile.get(&name) {
                context.extend(profile.clone());
            }
        }
        context
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn write_data_file(dir: &Path, contents: &str) {
        fs::write(dir.join("dotrift_data.toml"), contents).expect("cannot write data file");
    }

    #[test]
    fn read_missing_data_file_returns_default() {
        let dir = tempdir().expect("cannot create temp dir");
        let data = DataFile::read(dir.path()).expect("cannot read data file");
        assert!(data.variable.is_empty());
        assert!(data.profile.is_empty());
    }

    #[test]
    fn read_parses_variables_and_profiles() {
        let dir = tempdir().expect("cannot create temp dir");
        write_data_file(
            dir.path(),
            "[variable]\nname = \"dotrift\"\ncount = 3\n[profile.home]\neditor = \"nvim\"\n",
        );
        let data = DataFile::read(dir.path()).expect("cannot read data file");
        assert_eq!(
            data.variable.get("name"),
            Some(&Value::Str("dotrift".into()))
        );
        assert_eq!(data.variable.get("count"), Some(&Value::Int(3)));
        assert_eq!(
            data.profile
                .get("home")
                .and_then(|entry| entry.get("editor")),
            Some(&Value::Str("nvim".into()))
        );
    }

    #[test]
    fn read_rejects_nonexistent_source_directory() {
        let dir = tempdir().expect("cannot create temp dir");
        assert!(DataFile::read(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn read_rejects_unreadable_data_file() {
        let dir = tempdir().expect("cannot create temp dir");
        fs::create_dir(dir.path().join("dotrift_data.toml")).expect("cannot create directory");
        let error =
            DataFile::read(dir.path()).expect_err("reading a directory data file must fail");
        assert!(
            error
                .chain()
                .any(|cause| cause.to_string().contains("dotrift_data.toml")),
            "expected an error mentioning the data file but got: {error:?}"
        );
    }

    #[test]
    fn read_rejects_invalid_toml() {
        let dir = tempdir().expect("cannot create temp dir");
        write_data_file(dir.path(), "[variable\n");
        let error = DataFile::read(dir.path()).expect_err("malformed data file must fail to parse");
        assert!(
            error
                .chain()
                .any(|cause| cause.to_string().contains("dotrift_data.toml")),
            "expected an error mentioning the data file but got: {error:?}"
        );
    }

    #[test]
    fn context_starts_from_variables_without_active_profiles() {
        let mut data = DataFile::default();
        data.variable
            .insert("name".into(), Value::Str("dotrift".into()));
        assert_eq!(
            data.context(&[]),
            BTreeMap::from([("name".into(), Value::Str("dotrift".into()))])
        );
    }

    #[test]
    fn context_applies_profiles_in_priority_then_name_order() {
        let mut data = DataFile::default();
        data.variable.insert("a".into(), Value::Int(1));
        data.profile.insert(
            "work".into(),
            BTreeMap::from([
                ("a".into(), Value::Int(2)),
                ("zone".into(), Value::Str("work".into())),
            ]),
        );
        data.profile.insert(
            "home".into(),
            BTreeMap::from([
                ("a".into(), Value::Int(3)),
                ("zone".into(), Value::Str("home".into())),
            ]),
        );
        let context = data.context(&[("work".to_string(), 1), ("home".to_string(), 2)]);
        assert_eq!(context.get("a"), Some(&Value::Int(3)));
        assert_eq!(context.get("zone"), Some(&Value::Str("home".into())));
    }

    #[test]
    fn context_tie_breaks_equal_priority_by_name() {
        let mut data = DataFile::default();
        data.profile.insert(
            "work".into(),
            BTreeMap::from([("zone".into(), Value::Str("work".into()))]),
        );
        data.profile.insert(
            "home".into(),
            BTreeMap::from([("zone".into(), Value::Str("home".into()))]),
        );
        let context = data.context(&[("work".to_string(), 1), ("home".to_string(), 1)]);
        assert_eq!(context.get("zone"), Some(&Value::Str("work".into())));
    }

    #[test]
    fn context_ignores_active_profiles_without_definition() {
        let mut data = DataFile::default();
        data.variable.insert("a".into(), Value::Int(1));
        let context = data.context(&[("gone".to_string(), 1)]);
        assert_eq!(context, BTreeMap::from([("a".into(), Value::Int(1))]));
    }
}
