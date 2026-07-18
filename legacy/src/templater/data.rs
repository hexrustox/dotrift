use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
};

use miette::{Context, Result, miette};
use serde::Deserialize;
use templater::value::Value;

use crate::{db::Db, path::data_path};

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TemplateData {
    pub variable: HashMap<String, Value>,
    pub profile: BTreeMap<String, HashMap<String, Value>>,
}

impl TemplateData {
    pub fn read_from_file(path: &Path) -> Result<Self> {
        let s = crate::read_file_err!(fs::read_to_string(path), path)?;
        crate::parse_err!(toml::from_str(&s), path)
    }

    pub fn read(source_dir: &Path) -> Result<Self> {
        let path = data_path(source_dir);
        if !path.is_file() {
            return Ok(Self::default());
        }
        Self::read_from_file(&path)
    }

    pub fn resolve_variables(mut self, db: &Db) -> Result<HashMap<String, Value>> {
        let active_profiles = db.get_active_profiles()?;
        for profile in active_profiles {
            if let Some(vars) = self.profile.remove(&profile.name) {
                self.variable.extend(vars);
            }
        }
        Ok(self.variable)
    }
}
