use std::path::Path;

use miette::Context;

use crate::{
    cli::{GlobalFlags, UnapplyFlags},
    command::apply::{build_ignore, resolve_portals},
    command::util::{clean_up, resolve_target},
    config::Config,
    db::Db,
    output,
    templater::{data::TemplateData, function::BuiltinFunctions},
};

pub fn run(global_flags: GlobalFlags, db_path: &Path, flags: UnapplyFlags) -> miette::Result<()> {
    let source_dir = global_flags.source()?;
    let target_override = global_flags.target()?;

    let db = Db::init(db_path)?;
    let data = TemplateData::read(&source_dir)?;
    let variables = data.resolve_variables(&db)?;
    let functions = BuiltinFunctions::new();
    let config = Config::read_templated(&source_dir, &variables, &functions)
        .wrap_err("failed to read config")?;
    let target_dir = resolve_target(&source_dir, target_override, &config)?;

    let ignore_matcher = build_ignore(&config.ignore, &target_dir)?;
    let portal_entries = resolve_portals(
        &source_dir,
        &target_dir,
        &config.portal,
        &ignore_matcher,
        true,
    )?;

    let n = clean_up(
        &portal_entries,
        &db,
        flags.dry_run,
        flags.prune_empty_dirs,
        global_flags.verbose,
        true,
    )?;
    if flags.dry_run {
        let label = if n == 1 { "removal" } else { "removals" };
        output::print_summary(format_args!("{} {}", n, label));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::cli::UnapplyFlags;
    use crate::command::apply;
    use crate::command::util::assert_captured_output;

    use super::*;

    #[test]
    fn test_unapply_dry_run_print_snapshot() {
        let (temp_dir, source_dir, target_dir) = crate::command::util::tests::setup_test(
            r#""*" = """#,
            "",
            r#""*" = { type = "copy" }"#,
            true,
        );
        apply::run(
            GlobalFlags::new(
                Some(source_dir.to_path_buf()),
                Some(target_dir.to_path_buf()),
                None,
            ),
            &temp_dir.path().join("db"),
            crate::cli::ApplyFlags {
                dry_run: false,
                clean_up: false,
                prune_empty_dirs: false,
            },
        )
        .unwrap();

        run(
            crate::cli::GlobalFlags::new(
                Some(source_dir.to_path_buf()),
                Some(target_dir.to_path_buf()),
                None,
            ),
            &temp_dir.path().join("db"),
            UnapplyFlags {
                dry_run: true,
                prune_empty_dirs: false,
            },
        )
        .unwrap();

        assert_captured_output("unapply_dry_run", temp_dir.path());
    }
}
