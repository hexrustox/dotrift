use std::{
    fs::{self, symlink_metadata},
    io::{BufWriter, Write},
    os::unix::fs::{self as unix_fs, MetadataExt, PermissionsExt},
    path::Path,
};

use memmap2::Mmap;
use miette::{Context, Report, Result, miette};

use crate::{
    command::{
        prompt::{CollisionOptions, prompt_collision},
        tree::Node,
        util::{PathExt, clone_file, hash_file, is_managed, read_mtime},
    },
    config::DeployType,
    create_file_err,
    db::{Db, DbEntry},
    mmap_template_err, open_template_err, output, parse_template_err, render_template_err,
    write_file_err,
};

use super::{PortalEntry, TemplateContext};

pub fn traverse_tree(
    target: &Path,
    node: &Node,
    db: &Db,
    overwrite_identical: bool,
    verbose: bool,
    template_ctx: &TemplateContext,
) -> Result<()> {
    match node {
        Node::Dir(children) => {
            if deploy_dir(target, db, verbose)? {
                return Ok(());
            }
            for (name, child) in children {
                traverse_tree(
                    &target.join(name),
                    child,
                    db,
                    overwrite_identical,
                    verbose,
                    template_ctx,
                )?;
            }
        }
        Node::File(entry) => {
            deploy_file(
                target,
                entry,
                db,
                overwrite_identical,
                verbose,
                template_ctx,
            )?;
        }
        Node::Claim(_) => {
            #[cfg(test)]
            unreachable!()
        }
    }
    Ok(())
}

fn abort_deploy(at: &Path) -> Report {
    miette!("aborted at `{}`", at.display())
}

fn deploy_dir(path: &Path, db: &Db, verbose: bool) -> Result<bool> {
    if path.path_exists() {
        if path.path_is_dir() {
            return Ok(false);
        }
        let choice = prompt_collision(None, path, true, false);
        match choice {
            CollisionOptions::Skip => return Ok(true),
            CollisionOptions::Overwrite => {
                crate::remove_file_err!(fs::remove_file(path), path)?;
                if verbose {
                    output::print_removed(path);
                }
                db.delete_entry(path)?;
            }
            CollisionOptions::Quit => {
                return Err(abort_deploy(path));
            }
            CollisionOptions::Diff => unreachable!(),
        }
    }
    crate::create_dir_err!(fs::create_dir_all(path), path)?;
    if verbose {
        output::print_created_dir(path);
    }
    Ok(false)
}

fn is_identical(
    target: &Path,
    source: &Path,
    deploy_type: DeployType,
    source_hash: &mut Option<u64>,
    target_hash: &mut Option<u64>,
) -> bool {
    match deploy_type {
        DeployType::Symlink => {
            target.path_is_symlink() && fs::read_link(target).is_ok_and(|l| l == source)
        }
        DeployType::Copy if source.path_is_symlink() => {
            target.path_is_symlink()
                && fs::read_link(source)
                    .is_ok_and(|src_dest| fs::read_link(target).is_ok_and(|l| l == src_dest))
        }
        DeployType::Copy => {
            source.path_is_file()
                && target.path_is_file()
                && symlink_metadata(source)
                    .is_ok_and(|m1| symlink_metadata(target).is_ok_and(|m2| m1.size() == m2.size()))
                && hash_file(source).is_ok_and(|h1| {
                    *source_hash = Some(h1);
                    hash_file(target).is_ok_and(|h2| {
                        *target_hash = Some(h2);
                        h1 == h2
                    })
                })
        }
        DeployType::Tmpl => false,
    }
}

fn deploy_file(
    target: &Path,
    entry: &PortalEntry,
    db: &Db,
    overwrite_identical: bool,
    verbose: bool,
    template_ctx: &TemplateContext,
) -> Result<()> {
    let mut source_hash = None;
    let mut target_hash = None;

    if target.path_exists() {
        if target.path_is_dir() {
            let choice = prompt_collision(Some(&entry.source), target, false, true);
            match choice {
                CollisionOptions::Skip => return Ok(()),
                CollisionOptions::Overwrite => {
                    crate::remove_dir_err!(fs::remove_dir_all(target), target)?;
                    if verbose {
                        output::print_removed(target);
                    }
                    db.delete_entry_with_prefix(target)?;
                }
                CollisionOptions::Quit => {
                    return Err(abort_deploy(target));
                }
                CollisionOptions::Diff => unreachable!(),
            }
        } else {
            let identical = is_identical(
                target,
                &entry.source,
                entry.deploy_type,
                &mut source_hash,
                &mut target_hash,
            );
            if identical {
                if overwrite_identical {
                    update_db(target, entry, db, target_hash)?;
                }
                return Ok(());
            }

            #[cfg(test)]
            {
                super::tests::CHECK_MANAGED.set(true);
            }
            let managed = is_managed(target, db, target_hash);
            if !managed {
                let choice = prompt_collision(Some(&entry.source), target, false, false);
                match choice {
                    CollisionOptions::Skip => return Ok(()),
                    CollisionOptions::Overwrite => {}
                    CollisionOptions::Quit => {
                        return Err(abort_deploy(target));
                    }
                    CollisionOptions::Diff => unreachable!(),
                }
            }
        }
    }

    match entry.deploy_type {
        DeployType::Symlink => {
            let _ = fs::remove_file(target);
            crate::symlink_err!(
                unix_fs::symlink(&entry.source, target),
                target,
                &entry.source
            )?
        }
        DeployType::Copy => {
            clone_file(&entry.source, target)?;
        }
        DeployType::Tmpl => {
            let src = entry
                .source
                .clone()
                .canonicalize()
                .map_err(|e| miette!(e))
                .wrap_err_with(|| format!("failed to resolve `{}`", entry.source.display()))?;
            let file = open_template_err!(fs::File::open(&src), &src)?;

            // SAFETY: Assume this process has exclusive access to the file — opened read-only,
            // no concurrent writer modifies or truncates it while mapped.
            let mmap = mmap_template_err!(unsafe { Mmap::map(&file) }, &src)?;

            let tmpl = parse_template_err!(templater::Template::from_mmap(mmap), &src)?;
            let out = create_file_err!(fs::File::create(target), target)?;
            let mut writer = BufWriter::new(out);
            render_template_err!(
                tmpl.render(
                    &mut writer,
                    &template_ctx.variables,
                    &template_ctx.functions
                ),
                &src
            )?;
            write_file_err!(writer.flush(), target)?;
        }
    }
    match entry.deploy_type {
        DeployType::Copy | DeployType::Tmpl => {
            if let Some(mode) = entry.mode
                && target.path_is_file()
            {
                fs::set_permissions(target, fs::Permissions::from_mode(mode.0 as u32))
                    .map_err(|e| miette!(e))
                    .wrap_err_with(|| {
                        format!("failed to set permissions on `{}`", target.display())
                    })?;
            }
        }
        _ => {}
    }
    if verbose {
        output::print_created_file(target, &entry.source, entry.deploy_type);
    }

    update_db(target, entry, db, source_hash)?;
    Ok(())
}

fn update_db(target: &Path, entry: &PortalEntry, db: &Db, source_hash: Option<u64>) -> Result<()> {
    let is_regular = target.path_is_file();
    db.insert_or_update(&DbEntry {
        deploy_type: entry.deploy_type,
        source_path: entry.source.clone(),
        hash: if is_regular {
            Some(match entry.deploy_type {
                DeployType::Tmpl => hash_file(target)?,
                _ => source_hash
                    .map(Ok)
                    .unwrap_or_else(|| hash_file(&entry.source))?,
            })
        } else {
            None
        },
        symlink_target: if entry.deploy_type == DeployType::Copy && entry.source.path_is_symlink() {
            Some(crate::read_link_err!(
                fs::read_link(&entry.source),
                &entry.source
            )?)
        } else {
            None
        },
        mtime: if is_regular { read_mtime(target) } else { None },
        target_path: target.to_path_buf(),
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;
    use test_case::test_case;

    use super::super::tests::{CHECK_MANAGED, FLAGS, mock_apply};
    use super::is_identical;
    use crate::cli::ApplyFlags;
    use crate::command::{
        prompt::{CollisionOptions, tests::PROMPT_SELECTION},
        util::{hash_file, tests::setup_test},
    };
    use crate::config::DeployType;
    use crate::db::Db;

    #[test_case(
        |s, t| {
            std::os::unix::fs::symlink(s.join("src"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Symlink)
        => true; "symlink_identical")]
    #[test_case(
        |s, t| {
            std::os::unix::fs::symlink(s.join("other"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Symlink)
        => false; "symlink_not_identical")]
    #[test_case(
        |s, _| {
            fs::write(s.join("src"), "").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Symlink)
        => false; "symlink_target_not_symlink")]
    #[test_case(
        |s, t| {
            fs::write(s.join("src"), "a").unwrap();
            fs::write(t.join("target"), "a").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => true; "copy_file_identical")]
    #[test_case(
        |s, t| {
            fs::write(s.join("src"), "a").unwrap();
            fs::write(t.join("target"), "b").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_file_not_identical")]
    #[test_case(
        |s, t| {
            fs::write(s.join("src"), "a").unwrap();
            std::os::unix::fs::symlink(s.join("src"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_file_target_is_symlink")]
    #[test_case(
        |s, _| {
            std::os::unix::fs::symlink(Path::new("/a"), s.join("src")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_symlink_source_target_missing")]
    #[test_case(
        |s, t| {
            std::os::unix::fs::symlink(Path::new("/a"), s.join("src")).unwrap();
            std::os::unix::fs::symlink(Path::new("/a"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => true; "copy_symlink_source_identical")]
    #[test_case(
        |s, t| {
            std::os::unix::fs::symlink(Path::new("/a"), s.join("src")).unwrap();
            std::os::unix::fs::symlink(Path::new("/b"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_symlink_source_not_identical")]
    #[test_case(
        |s, t| {
            std::os::unix::fs::symlink(Path::new("/a"), s.join("src")).unwrap();
            fs::write(t.join("target"), "").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_symlink_source_target_is_file")]
    #[test_case(
        |s, t| {
            fs::write(s.join("src"), "same").unwrap();
            fs::write(t.join("target"), "same").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Tmpl)
        => false; "tmpl_always_false"
    )]
    fn test_is_identical(
        setup: impl FnOnce(&Path, &Path),
        paths: impl FnOnce(&Path, &Path) -> (PathBuf, PathBuf, DeployType),
    ) -> bool {
        let temp_dir = tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let target_dir = temp_dir.path().join("target");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&target_dir).unwrap();

        setup(&source_dir, &target_dir);
        let (target, source, deploy_type) = paths(&source_dir, &target_dir);
        is_identical(&target, &source, deploy_type, &mut None, &mut None)
    }

    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "").unwrap();
        },
        |s, t| {
            assert!(t.join("file").exists());
            assert!(t.join("file").is_symlink());
            assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
        },
        DeployType::Symlink; "symlink_fresh"
    )]
    #[test_case(
        |s, _| {
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Symlink; "symlink_nested"
    )]
    #[test_case(
        |s, _| {
            std::os::unix::fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        },
        |s, t| {
            assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
        },
        DeployType::Symlink; "symlink_broken_source"
    )]
    #[test_case(
        |s, _| {
            std::os::unix::fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/a"));
        },
        DeployType::Copy; "copy_symlink_source_fresh"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("file").exists());
            assert!(!t.join("file").is_symlink());
        },
        DeployType::Copy; "copy_fresh"
    )]
    #[test_case(
        |s, _| {
            fs::create_dir_all(s.join("dir/sub")).unwrap();
            fs::write(s.join("dir/file1"), "a").unwrap();
            fs::write(s.join("dir/sub/file2"), "b").unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("dir/file1")).unwrap(), "a");
            assert_eq!(fs::read_to_string(t.join("dir/sub/file2")).unwrap(), "b");
        },
        DeployType::Copy; "copy_nested_dir_fresh"
    )]
    #[test_case(
        |s, t| {
            std::os::unix::fs::symlink(Path::new("/a"), s.join("file")).unwrap();
            std::os::unix::fs::symlink(s.join("file"), t.join("file")).unwrap();
        },
        |_, _| {
            assert!(!CHECK_MANAGED.with_borrow(|b| *b));
        },
        DeployType::Symlink; "symlink_identical"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "a").unwrap();
        },
        |_, _| {
            assert!(!CHECK_MANAGED.with_borrow(|b| *b));
        },
        DeployType::Copy; "copy_identical_file"
    )]
    #[test_case(
        |s, t| {
            std::os::unix::fs::symlink(Path::new("/a"), s.join("file")).unwrap();
            std::os::unix::fs::symlink(Path::new("/a"), t.join("file")).unwrap();
        },
        |_, _| {
            assert!(!CHECK_MANAGED.with_borrow(|b| *b));
        },
        DeployType::Copy; "copy_identical_symlink"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("dir"), "").unwrap();
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Symlink; "symlink_file_vs_dir_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(s.join("dir"), "").unwrap();
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir").is_file());
        },
        DeployType::Symlink; "symlink_file_vs_dir_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("dir"), "").unwrap();
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Copy; "copy_file_vs_dir_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(s.join("dir"), "").unwrap();
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir").is_file());
        },
        DeployType::Copy; "copy_file_vs_dir_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
            fs::write(t.join("dir"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir").is_file());
        },
        DeployType::Symlink; "symlink_dir_vs_file_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
            fs::write(t.join("dir"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Symlink; "symlink_dir_vs_file_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
            fs::write(t.join("dir"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir").is_file());
        },
        DeployType::Copy; "copy_dir_vs_file_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
            fs::write(t.join("dir"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Copy; "copy_dir_vs_file_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file"), "").unwrap();
            fs::write(t.join("file"), "").unwrap();
        },
        |_, t| {
            assert!(!t.join("file").is_symlink());
        },
        DeployType::Symlink; "symlink_file_vs_file_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(s.join("file"), "").unwrap();
            fs::write(t.join("file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("file").is_symlink());
        },
        DeployType::Symlink; "symlink_file_vs_file_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "b").unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "b");
        },
        DeployType::Copy; "copy_file_vs_file_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "b").unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "a");
        },
        DeployType::Copy; "copy_file_vs_file_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file1"), "").unwrap();
            PROMPT_SELECTION.set(CollisionOptions::Quit);
            fs::write(s.join("file2"), "").unwrap();
            fs::write(t.join("file2"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("file1").exists());
            assert!(!t.join("file2").exists());
        },
        DeployType::Symlink => panics "abort"; "quit_symlink"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file1"), "").unwrap();
            PROMPT_SELECTION.set(CollisionOptions::Quit);
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "b").unwrap();
        },
        |_, t| {
            assert!(t.join("file1").exists());
            assert!(!t.join("file2").exists());
        },
        DeployType::Copy => panics "abort"; "quit_copy"
    )]
    #[test_case(
        |s, t| {
            fs::create_dir_all(t.join("real_dir")).unwrap();
            std::os::unix::fs::symlink(t.join("real_dir"), s.join("link_dir")).unwrap();
        },
        |s, t| {
            assert!(t.join("link_dir").is_symlink());
            assert_eq!(fs::read_link(t.join("link_dir")).unwrap(), s.join("link_dir"));
        },
        DeployType::Symlink; "symlink_to_dir_as_source"
    )]
    #[test_case(
        |s, t| {
            fs::create_dir_all(t.join("real_dir")).unwrap();
            std::os::unix::fs::symlink(t.join("real_dir"), s.join("link_dir")).unwrap();
        },
        |_, t| {
            assert!(t.join("link_dir").is_symlink());
            assert_eq!(fs::read_link(t.join("link_dir")).unwrap(), t.join("real_dir"));
        },
        DeployType::Copy; "copy_symlink_to_dir_as_source"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("dotrift_data.toml"), r#"[variable]
name = "world""#).unwrap();
            fs::write(s.join("file"), "Hello {{ name }}").unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "Hello world");
        },
        DeployType::Tmpl; "tmpl_fresh"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("dotrift_data.toml"), r#"[variable]
name = "world""#).unwrap();
            fs::write(t.join("template"), "Hello {{ name }}").unwrap();
            std::os::unix::fs::symlink(t.join("template"), s.join("file")).unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "Hello world");
        },
        DeployType::Tmpl; "tmpl_symlink_source_fresh"
    )]
    fn test_first_apply(
        setup: impl FnOnce(&Path, &Path),
        assert: impl FnOnce(&Path, &Path),
        deploy_type: DeployType,
    ) {
        let (temp_dir, source_dir, target_dir) = setup_test(
            r#""" = """#,
            "",
            match deploy_type {
                DeployType::Symlink => "",
                DeployType::Copy => r#""**/*" = { type = "copy" }"#,
                DeployType::Tmpl => r#""**/*" = { type = "tmpl" }"#,
            },
            false,
        );
        setup(&source_dir, &target_dir);
        mock_apply(&source_dir, &target_dir, &temp_dir.path().join("db"), FLAGS).unwrap();
        assert(&source_dir, &target_dir);
    }

    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, _| {},
        |s, t, db| {
            assert!(t.join("file").exists());
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_identical"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, _| {},
        |s, t, db| {
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Symlink);
            assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
            assert!(entry.hash.is_none());
        },
        DeployType::Symlink; "symlink_reapply_identical"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |s, _| {
            fs::write(s.join("file"), "b").unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "b");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_source_changed"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |s, _| {
            fs::write(s.join("file"), "b").unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Symlink);
            assert!(entry.hash.is_none());
        },
        DeployType::Symlink; "symlink_reapply_source_changed"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            fs::write(t.join("file"), "external").unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "external");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_external_modification_skip"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(t.join("file"), "external").unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "a");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_external_modification_overwrite"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            let _ = fs::remove_file(t.join("file"));
            fs::write(t.join("file"), "external").unwrap();
        },
        |_, t, db| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "external");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Symlink);
            assert!(entry.hash.is_none());
        },
        DeployType::Symlink; "symlink_reapply_target_replaced_with_file_skip"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            let _ = fs::remove_file(t.join("file"));
            std::os::unix::fs::symlink(Path::new("/wrong"), t.join("file")).unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/wrong"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.source_path, s.join("file"));
            assert_eq!(entry.deploy_type, DeployType::Symlink);
            assert!(entry.hash.is_none());
        },
        DeployType::Symlink; "symlink_reapply_target_replaced_with_wrong_symlink_skip"
    )]
    #[test_case(
        |s, _| {
            std::os::unix::fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        },
        |_, _| {},
        |_, t, db| {
            assert!(t.join("file").is_symlink());
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/a"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert!(entry.hash.is_none());
            assert_eq!(entry.symlink_target, Some(PathBuf::from("/a")));
        },
        DeployType::Copy; "copy_reapply_symlink_source_identical"
    )]
    #[test_case(
        |s, _| {
            std::os::unix::fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        },
        |s, _| {
            let _ = fs::remove_file(s.join("file"));
            std::os::unix::fs::symlink(Path::new("/b"), s.join("file")).unwrap();
        },
        |_, t, db| {
            assert!(t.join("file").is_symlink());
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/b"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert!(entry.hash.is_none());
            assert_eq!(entry.symlink_target, Some(PathBuf::from("/b")));
        },
        DeployType::Copy; "copy_reapply_symlink_source_changed"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            let _ = fs::remove_file(t.join("file"));
            std::os::unix::fs::symlink(Path::new("/evil"), t.join("file")).unwrap();
        },
        |s, t, db| {
            assert!(t.join("file").is_symlink());
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/evil"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_target_type_changed_skip"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            let _ = fs::remove_file(t.join("file"));
            std::os::unix::fs::symlink(Path::new("/evil"), t.join("file")).unwrap();
        },
        |_, t, db| {
            assert!(!t.join("file").is_symlink());
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "a");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert!(entry.hash.is_some());
        },
        DeployType::Copy; "copy_reapply_target_type_changed_overwrite"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("dotrift_data.toml"), r#"[variable]
name = "world""#).unwrap();
            fs::write(s.join("file"), "Hello {{ name }}").unwrap();
        },
        |_, _| {},
        |_, t, db| {
            let rendered = fs::read_to_string(t.join("file")).unwrap();
            assert_eq!(rendered, "Hello world");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Tmpl);
            assert_eq!(entry.hash.unwrap(), hash_file(&t.join("file")).unwrap());
        },
        DeployType::Tmpl; "tmpl_reapply_identical"
    )]
    fn test_reapply(
        setup1: impl FnOnce(&Path, &Path),
        setup2: impl FnOnce(&Path, &Path),
        assert: impl FnOnce(&Path, &Path, &Db),
        deploy_type: DeployType,
    ) {
        let (temp_dir, source_dir, target_dir) = setup_test(
            r#""" = """#,
            "",
            match deploy_type {
                DeployType::Symlink => "",
                DeployType::Copy => r#""**/*" = { type = "copy" }"#,
                DeployType::Tmpl => r#""**/*" = { type = "tmpl" }"#,
            },
            false,
        );
        setup1(&source_dir, &target_dir);
        let db_path = temp_dir.path().join("db");
        mock_apply(&source_dir, &target_dir, &db_path, FLAGS).unwrap();
        setup2(&source_dir, &target_dir);
        mock_apply(&source_dir, &target_dir, &db_path, FLAGS).unwrap();
        let db = Db::init(&db_path).unwrap();
        assert(&source_dir, &target_dir, &db);
    }

    #[test]
    fn test_deploy_permission() {
        let (temp_dir, source_dir, target_dir) = setup_test(
            r#""" = """#,
            "",
            r#""**/*" = { type = "copy", mode = "123" }"#,
            false,
        );
        fs::write(source_dir.join("file"), "").unwrap();
        mock_apply(&source_dir, &target_dir, &temp_dir.path().join("db"), FLAGS).unwrap();
        assert_eq!(
            target_dir
                .join("file")
                .metadata()
                .unwrap()
                .permissions()
                .mode(),
            0o100123
        );
    }

    #[test_case(FLAGS; "creates_file")]
    #[test_case(ApplyFlags { dry_run: true, ..FLAGS }; "dry_run_no_file")]
    fn test_deploy_respects_dry_run(flags: ApplyFlags) {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", "", false);
        fs::write(source_dir.join("file"), "").unwrap();
        mock_apply(&source_dir, &target_dir, &temp_dir.path().join("db"), flags).unwrap();
        assert_eq!(target_dir.join("file").exists(), !flags.dry_run);
    }

    #[test_case(false; "removes_file")]
    #[test_case(true; "dry_run_preserves_file")]
    fn test_apply_clean_up(dry_run: bool) {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", "", false);
        fs::write(source_dir.join("file"), "").unwrap();
        mock_apply(&source_dir, &target_dir, &temp_dir.path().join("db"), FLAGS).unwrap();
        assert!(target_dir.join("file").exists());
        fs::write(source_dir.join("dotrift.toml"), "").unwrap();
        mock_apply(
            &source_dir,
            &target_dir,
            &temp_dir.path().join("db"),
            ApplyFlags {
                dry_run,
                clean_up: true,
                ..FLAGS
            },
        )
        .unwrap();
        assert_eq!(target_dir.join("file").exists(), dry_run);
    }
}
