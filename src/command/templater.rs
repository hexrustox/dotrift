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
    create_dir_err,
    db::Db,
    templater::{data::TemplateData, function::BuiltinFunctions},
};

pub fn run(global: GlobalFlags, db_path: &Path, flags: TemplaterFlags) -> Result<()> {
    let tmpl = if let Some(s) = &flags.string {
        Template::from_bytes(s.as_bytes().to_vec()).wrap_err("Failed to parse template")?
    } else {
        let path = flags.file.as_ref().unwrap();
        let file = fs::File::open(path)
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("Failed to open template file `{}`", path.display()))?;
        let mmap = unsafe { Mmap::map(&file) }
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("Failed to mmap template file `{}`", path.display()))?;
        Template::from_mmap(mmap)
            .wrap_err_with(|| format!("Failed to parse template in `{}`", path.display()))?
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
        let (key, value_str) = var_str
            .split_once('=')
            .ok_or_else(|| miette!("Invalid --var `{var_str}`: expected KEY=VALUE"))?;
        let value = parse_cli_value(value_str);
        variables.insert(key.to_string(), value);
    }

    let writer: Box<dyn Write> = if let Some(out_path) = &flags.output {
        if let Some(parent) = out_path.parent() {
            create_dir_err!(fs::create_dir_all(parent), parent)?;
        }
        let file = fs::File::create(out_path)
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("Failed to create output file `{}`", out_path.display()))?;
        Box::new(file)
    } else {
        Box::new(io::stdout())
    };

    let mut buf = BufWriter::new(writer);
    tmpl.render(&mut buf, variables, &BuiltinFunctions::new())
        .wrap_err("Failed to render template")?;
    buf.flush()
        .map_err(|e| miette!(e))
        .wrap_err("Failed to flush output")?;

    Ok(())
}

fn parse_cli_value(s: &str) -> Value {
    if let Ok(n) = s.parse::<i64>() {
        return Value::Int(n);
    }
    if let Ok(b) = s.parse::<bool>() {
        return Value::Bool(b);
    }
    Value::Str(s.to_string())
}
