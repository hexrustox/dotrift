use std::path::Path;

use miette::Result;

use crate::{command::tree::Node, output};

pub fn print_tree(path: &Path, node: &Node) -> Result<usize> {
    let mut count = 0;
    match node {
        Node::Dir(children) => {
            if path != Path::new("/") {
                output::print_dry_create_dir(path);
            }
            for (name, child) in children {
                count += print_tree(&path.join(name), child)?;
            }
        }
        Node::File(entry) => {
            count += 1;
            output::print_dry_create_file(path, &entry.source, entry.deploy_type);
        }
        Node::Claim(_) => {
            #[cfg(test)]
            unreachable!()
        }
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{
        cli::ApplyFlags,
        command::util::{assert_captured_output, tests::setup_test},
    };
    use super::super::tests::mock_apply;

    #[test]
    fn test_dry_run_print_snapshot() {
        let (temp_dir, source_dir, target_dir) =
            setup_test(r#""" = """#, "", r#""subdir/*" = { type = "copy" }"#, true);
        mock_apply(
            &source_dir,
            &target_dir,
            &temp_dir.path().join("db"),
            ApplyFlags {
                dry_run: true,
                clean_up: false,
                prune_empty_dirs: false,
            },
        )
        .unwrap();

        assert_captured_output("apply_dry_run", temp_dir.path())
    }

    #[test]
    fn test_clean_up_dry_run_print_snapshot() {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", "", true);
        mock_apply(
            &source_dir,
            &target_dir,
            &temp_dir.path().join("db"),
            ApplyFlags {
                dry_run: false,
                clean_up: false,
                prune_empty_dirs: false,
            },
        )
        .unwrap();

        fs::write(source_dir.join("dotrift.toml"), "").unwrap();

        mock_apply(
            &source_dir,
            &target_dir,
            &temp_dir.path().join("db"),
            ApplyFlags {
                dry_run: true,
                clean_up: true,
                prune_empty_dirs: false,
            },
        )
        .unwrap();

        assert_captured_output("apply_clean_up_dry_run", temp_dir.path())
    }
}
