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
