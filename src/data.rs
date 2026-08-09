use std::{collections::BTreeMap, fs, path::Path};

use miette::{Result, WrapErr, miette};
use templater::value::Value;

#[derive(Debug, Default)]
pub struct DataFile {
    pub variables: BTreeMap<String, Value>,
    pub profiles: BTreeMap<String, BTreeMap<String, Value>>,
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
        let document: toml::Value = toml::from_str(&text)
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot parse `{}`", path.display()))?;
        Self::from_toml(document)
    }

    fn from_toml(document: toml::Value) -> Result<Self> {
        let table = document
            .as_table()
            .ok_or_else(|| miette!("dotrift_data.toml must contain a table"))?;
        let mut data = Self::default();
        for (name, value) in table {
            match name.as_str() {
                "variable" => data.variables = bindings(value, "[variable]")?,
                "profile" => {
                    let profiles = value
                        .as_table()
                        .ok_or_else(|| miette!("[profile] must be a table"))?;
                    for (name, profile) in profiles {
                        if name.is_empty() {
                            return Err(miette!("profile names cannot be empty"));
                        }
                        data.profiles
                            .insert(name.clone(), bindings(profile, "profile")?);
                    }
                }
                other => return Err(miette!("unknown dotrift_data.toml section `{other}`")),
            }
        }
        Ok(data)
    }

    pub fn context(&self, active: &[(String, i64)]) -> BTreeMap<String, Value> {
        let mut context = self.variables.clone();
        let mut active = active.to_vec();
        active.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        for (name, _) in active {
            if let Some(profile) = self.profiles.get(&name) {
                context.extend(profile.clone());
            }
        }
        context
    }
}

fn bindings(value: &toml::Value, section: &str) -> Result<BTreeMap<String, Value>> {
    let table = value
        .as_table()
        .ok_or_else(|| miette!("{section} must be a table"))?;
    table
        .iter()
        .map(|(key, value)| {
            if key.is_empty() {
                return Err(miette!("variable keys cannot be empty"));
            }
            Ok((key.clone(), convert(value)?))
        })
        .collect()
}

fn convert(value: &toml::Value) -> Result<Value> {
    match value {
        toml::Value::String(value) => Ok(Value::Str(value.clone())),
        toml::Value::Integer(value) => Ok(Value::Int(*value)),
        toml::Value::Boolean(value) => Ok(Value::Bool(*value)),
        toml::Value::Array(values) => values
            .iter()
            .map(convert)
            .collect::<Result<Vec<_>>>()
            .map(Value::List),
        toml::Value::Table(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), convert(value)?)))
            .collect::<Result<BTreeMap<_, _>>>()
            .map(Value::Map),
        toml::Value::Float(_) => Err(miette!("unsupported float value in dotrift_data.toml")),
        toml::Value::Datetime(_) => Err(miette!(
            "unsupported date or time value in dotrift_data.toml"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_overlay_whole_values_in_activation_order() {
        let data = DataFile::from_toml(toml::from_str("[variable]\nsettings = { base = true }\n[profile.a]\nsettings = [1]\n[profile.b]\nsettings = [2]").unwrap()).unwrap();
        let context = data.context(&[("b".into(), 2), ("a".into(), 1)]);
        assert_eq!(context["settings"], Value::List(vec![Value::Int(2)]));
    }
}
