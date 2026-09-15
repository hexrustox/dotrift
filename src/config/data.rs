use std::{collections::BTreeMap, fs, path::Path};

use miette::{Result, WrapErr, miette};
use serde::Deserialize;
use templater::value::Value;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct DataFile {
    variable: BTreeMap<String, Value>,
    pub profile: BTreeMap<String, BTreeMap<String, Value>>,
}

impl DataFile {
    pub(crate) fn read(source: &Path) -> Result<Self> {
        crate::platform::ensure_source_dir(source)?;
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
        parse_data_file(&text).wrap_err_with(|| format!("cannot parse `{}`", path.display()))
    }

    pub(crate) fn context(&self, active: &[(String, i64)]) -> BTreeMap<String, Value> {
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

fn parse_data_file(text: &str) -> Result<DataFile> {
    let data: DataFile = toml::from_str(text).map_err(|error| miette!(error))?;
    ensure_no_empty_keys(&data.variable, "[variable]")?;
    for (name, bindings) in &data.profile {
        if name.is_empty() {
            return Err(miette!("empty profile name"));
        }
        ensure_no_empty_keys(bindings, &format!("[profile.{name}]"))?;
    }
    Ok(data)
}

fn ensure_no_empty_keys(bindings: &BTreeMap<String, Value>, table: &str) -> Result<()> {
    if bindings.keys().any(|key| key.is_empty()) {
        return Err(miette!("empty key in `{table}`"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use test_case::test_case;

    use super::*;

    fn write_data_file(dir: &Path, contents: &str) {
        fs::write(dir.join("dotrift_data.toml"), contents).expect("cannot write data file");
    }

    #[test]
    fn missing_data_file_reads_as_empty() {
        let dir = tempdir().expect("cannot create temp dir");
        let data = DataFile::read(dir.path()).expect("cannot read data file");
        assert!(data.variable.is_empty());
        assert!(data.profile.is_empty());
    }

    #[test]
    fn read_loads_variables_and_profiles() {
        let dir = tempdir().expect("cannot create temp dir");
        write_data_file(
            dir.path(),
            "[variable]\nstr = \"str\"\nnum = 1\n[profile.profile1]\nnum2 = 2\n",
        );
        let data = DataFile::read(dir.path()).expect("cannot read data file");
        assert_eq!(data.variable.get("str"), Some(&Value::Str("str".into())));
        assert_eq!(data.variable.get("num"), Some(&Value::Int(1)));
        assert_eq!(
            data.profile
                .get("profile1")
                .and_then(|entry| entry.get("num2")),
            Some(&Value::Int(2))
        );
    }

    #[test]
    fn rejects_missing_source_directory() {
        let dir = tempdir().expect("cannot create temp dir");
        assert!(DataFile::read(&dir.path().join("file1")).is_err());
    }

    #[test]
    fn rejects_unreadable_data_file() {
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
    fn rejects_malformed_toml() {
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

    #[test_case("[variable]\n\"\" = \"str\"\n", "empty key in `[variable]`" ; "empty_variable_key")]
    #[test_case("[profile.profile1]\n\"\" = \"str\"\n", "empty key in `[profile.profile1]`" ; "empty_profile_binding_key")]
    #[test_case("[profile.\"\"]\nstr = \"str\"\n", "empty profile name" ; "empty_profile_name")]
    fn rejects_empty_keys(contents: &str, expected: &str) {
        let dir = tempdir().expect("cannot create temp dir");
        write_data_file(dir.path(), contents);
        let error = DataFile::read(dir.path()).expect_err("empty key or name must fail");
        assert!(
            error
                .chain()
                .any(|cause| cause.to_string().contains(expected)),
            "expected an error containing {expected:?} but got: {error:?}"
        );
    }

    #[test]
    fn accepts_keys_outside_templater_variable_syntax() {
        let dir = tempdir().expect("cannot create temp dir");
        write_data_file(
            dir.path(),
            "[variable]\n\"a-b\" = \"str\"\n\n[profile.profile1]\n\"a-b\" = \"str2\"\n",
        );
        let data = DataFile::read(dir.path()).expect("non-bare keys must remain valid");
        assert_eq!(data.variable.get("a-b"), Some(&Value::Str("str".into())));
        assert_eq!(
            data.profile
                .get("profile1")
                .and_then(|bindings| bindings.get("a-b")),
            Some(&Value::Str("str2".into()))
        );
    }

    #[test]
    fn context_without_active_profiles_contains_only_variables() {
        let mut data = DataFile::default();
        data.variable.insert("str".into(), Value::Str("str".into()));
        assert_eq!(
            data.context(&[]),
            BTreeMap::from([("str".into(), Value::Str("str".into()))])
        );
    }

    #[test]
    fn context_prefers_higher_priority_profile() {
        let mut data = DataFile::default();
        data.variable.insert("a".into(), Value::Int(1));
        data.profile.insert(
            "profile2".into(),
            BTreeMap::from([("a".into(), Value::Int(2))]),
        );
        data.profile.insert(
            "profile1".into(),
            BTreeMap::from([("a".into(), Value::Int(4)), ("num".into(), Value::Int(1))]),
        );
        let context = data.context(&[("profile2".to_string(), 1), ("profile1".to_string(), 2)]);
        assert_eq!(context.get("a"), Some(&Value::Int(4)));
        assert_eq!(context.get("num"), Some(&Value::Int(1)));
    }

    #[test]
    fn context_tie_breaks_equal_priority_profiles_by_name() {
        let mut data = DataFile::default();
        data.profile.insert(
            "profile2".into(),
            BTreeMap::from([("num".into(), Value::Int(2))]),
        );
        data.profile.insert(
            "profile1".into(),
            BTreeMap::from([("num".into(), Value::Int(1))]),
        );
        let context = data.context(&[("profile2".to_string(), 1), ("profile1".to_string(), 1)]);
        assert_eq!(context.get("num"), Some(&Value::Int(2)));
    }

    #[test]
    fn context_ignores_undefined_active_profiles() {
        let mut data = DataFile::default();
        data.variable.insert("a".into(), Value::Int(1));
        let context = data.context(&[("profile3".to_string(), 1)]);
        assert_eq!(context, BTreeMap::from([("a".into(), Value::Int(1))]));
    }
}
