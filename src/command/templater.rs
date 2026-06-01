use std::{
    collections::HashMap,
    fs,
    io::{self, BufWriter, Write},
    path::Path,
};

use memmap2::Mmap;
use miette::{Context, Result, miette};
use templater::{Template, Value};

use crate::{
    cli::{GlobalFlags, TemplaterFlags},
    create_dir_err, create_file_err,
    db::Db,
    mmap_template_err, open_template_err, parse_template_err,
    templater::{data::TemplateData, function::BuiltinFunctions},
    write_file_err,
};

struct LastByte<W> {
    inner: W,
    last: Option<u8>,
}

impl<W: Write> Write for LastByte<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(&b) = buf.last() {
            self.last = Some(b);
        }
        self.inner.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub fn run(global: GlobalFlags, db_path: &Path, flags: TemplaterFlags) -> Result<()> {
    let tmpl = if let Some(s) = &flags.string {
        Template::from_bytes(s.as_bytes().to_vec()).wrap_err("failed to parse template")?
    } else {
        let path = flags.file.as_ref().unwrap();
        let file = open_template_err!(fs::File::open(path), path)?;
        let mmap = mmap_template_err!(unsafe { Mmap::map(&file) }, path)?;
        parse_template_err!(Template::from_mmap(mmap), path)?
    };

    let mut variables: HashMap<String, Value> = HashMap::new();
    if !flags.no_data {
        let mut data = if let Some(path) = flags.data_path {
            TemplateData::read_from_file(&path)
        } else {
            TemplateData::read(&global.source()?)
        }?;

        let db = Db::init(db_path)?;
        let active_profiles = db.get_active_profiles()?;

        variables = data.variable;
        for profile in &active_profiles {
            if let Some(vars) = data.profile.remove(&profile.name) {
                variables.extend(vars);
            }
        }
    }

    for var_str in &flags.var {
        let (key, value) = parse_cli_var(var_str)
            .ok_or_else(|| miette!("invalid format `{var_str}`, expect KEY=VALUE"))?;
        variables.insert(key, value);
    }

    if let Some(out_path) = &flags.output {
        if let Some(parent) = out_path.parent() {
            create_dir_err!(fs::create_dir_all(parent), parent)?;
        }
        let file = create_file_err!(fs::File::create(out_path), out_path)?;
        let mut writer = BufWriter::new(file);
        tmpl.render(&mut writer, variables, &BuiltinFunctions::new())
            .wrap_err("failed to render template")?;
        write_file_err!(writer.flush(), out_path)?;
    } else {
        let stdout = io::stdout();
        let mut writer = LastByte {
            inner: stdout.lock(),
            last: None,
        };
        tmpl.render(&mut writer, variables, &BuiltinFunctions::new())
            .wrap_err("failed to render template")?;
        if writer.last != Some(b'\n') {
            writer
                .write_all(b"\n")
                .map_err(|e| miette!(e))
                .wrap_err("failed to write newline to stdout")?;
        }
        writer
            .flush()
            .map_err(|e| miette!(e))
            .wrap_err("failed to flush stdout")?;
    }

    Ok(())
}

fn parse_cli_var(s: &str) -> Option<(String, Value)> {
    let doc = toml::from_str::<toml::Value>(s).ok()?;
    let table = doc.as_table()?;
    let (key, value) = table.iter().next()?;
    let value = Value::try_from(value.clone()).ok()?;
    Some((key.to_string(), value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use test_case::test_case;

    #[test_case("key=42" => ("key".into(), Value::Int(42)); "int")]
    #[test_case("n=-7" => ("n".into(), Value::Int(-7)); "negative_int")]
    #[test_case("flag=true" => ("flag".into(), Value::Bool(true)); "bool_true")]
    #[test_case("off=false" => ("off".into(), Value::Bool(false)); "bool_false")]
    #[test_case("name=\"hello\"" => ("name".into(), Value::Str("hello".into())); "toml_string")]
    #[test_case("list=[1,2,3]" => ("list".into(), Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])); "toml_list")]
    #[test_case("obj={a=1,b=2}" => ("obj".into(), Value::Map(BTreeMap::from([("a".into(), Value::Int(1)), ("b".into(), Value::Int(2))]))); "toml_table")]
    #[test_case("nested=[{x=1}]" => ("nested".into(), Value::List(vec![Value::Map(BTreeMap::from([("x".into(), Value::Int(1))]))])); "toml_nested")]
    fn test_parse_cli_var(input: &str) -> (String, Value) {
        parse_cli_var(input).unwrap()
    }

    #[test_case("bad";          "no_equals")]
    #[test_case("name=hello";   "bare_string")]
    #[test_case("empty=";       "empty_value")]
    #[test_case("path=a/b/c";   "path_with_slash")]
    #[test_case("pi=3.14";      "float_unsupported")]
    fn test_parse_cli_var_error(input: &str) {
        assert!(parse_cli_var(input).is_none());
    }
}
