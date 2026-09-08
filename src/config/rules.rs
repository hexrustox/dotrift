use std::path::Path;

use glob::Pattern;
use miette::{Result, miette};
use serde::Deserialize;

use super::portals::{ResolvedPortal, reject_brace_expansion, validate_relative};
use super::{DeployMode, DeployType, DeploymentEntry, GLOB_MATCH_OPTIONS};
use crate::report::Reporter;

#[derive(Debug, Clone, Copy)]
pub(super) struct RuleConfig {
    deploy_type: Option<DeployType>,
    mode: Option<DeployMode>,
}

impl<'de> serde::Deserialize<'de> for RuleConfig {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            #[serde(rename = "type")]
            deploy_type: Option<DeployType>,
            mode: Option<DeployMode>,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.mode.is_some() && raw.deploy_type == Some(DeployType::Symlink) {
            return Err(serde::de::Error::custom(
                "`mode` cannot be used with `type` `symlink`",
            ));
        }
        Ok(RuleConfig {
            deploy_type: raw.deploy_type,
            mode: raw.mode,
        })
    }
}

pub(super) fn compile_rules(
    rules: &indexmap::IndexMap<String, RuleConfig>,
) -> Result<indexmap::IndexMap<Pattern, RuleConfig>> {
    let mut compiled = indexmap::IndexMap::with_capacity(rules.len());
    for (pattern, rule) in rules {
        validate_relative(pattern, "rule")?;
        reject_brace_expansion(pattern, "rule")?;
        let pattern = Pattern::new(pattern.strip_prefix("./").unwrap_or(pattern))
            .map_err(|error| miette!("invalid rule pattern `{pattern}` because {error}"))?;
        compiled.insert(pattern, *rule);
    }
    Ok(compiled)
}

pub(super) fn apply_rules(
    entry: ResolvedPortal,
    rules: &indexmap::IndexMap<Pattern, RuleConfig>,
    target_root: &Path,
) -> Result<DeploymentEntry> {
    let mut deploy_type = DeployType::Symlink;
    let mut mode = None;
    for (pattern, rule) in rules {
        if pattern.matches_path_with(&entry.target, GLOB_MATCH_OPTIONS) {
            if let Some(value) = rule.deploy_type {
                deploy_type = value;
            }
            if let Some(value) = &rule.mode {
                mode = Some(*value);
            }
        }
    }
    if deploy_type == DeployType::Symlink {
        if mode.is_some() {
            Reporter::always().warning(format_args!(
                "ignoring `mode` for `{}`: effective `type` is `symlink`",
                entry.target.display()
            ));
        }
        mode = None;
    }
    Ok(DeploymentEntry {
        source_path: entry.source,
        target_path: target_root.join(entry.target),
        deploy_type,
        mode,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;
    use test_case::test_case;

    use crate::deploy_entry;

    use super::*;

    #[test_case("" => matches Ok(RuleConfig { deploy_type: None, mode: None }) ; "empty_table_means_no_properties")]
    #[test_case("type = \"symlink\"" => matches Ok(RuleConfig { deploy_type: Some(DeployType::Symlink), mode: None }) ; "symlink_type_parsed")]
    #[test_case("type = \"copy\"" => matches Ok(RuleConfig { deploy_type: Some(DeployType::Copy), mode: None }) ; "copy_type_parsed")]
    #[test_case("type = \"template\"" => matches Ok(RuleConfig { deploy_type: Some(DeployType::Template), mode: None }) ; "template_type_parsed")]
    #[test_case("mode = \"644\"" => matches Ok(RuleConfig { deploy_type: None, mode: Some(DeployMode(0o644)) }) ; "mode_parsed_without_type")]
    #[test_case("type = \"copy\"\nmode = \"644\"" => matches Ok(RuleConfig { deploy_type: Some(DeployType::Copy), mode: Some(DeployMode(0o644)) }) ; "mode_combines_with_copy")]
    fn deserializes_valid_rule_config(toml: &str) -> Result<RuleConfig, toml::de::Error> {
        toml::from_str(toml)
    }

    #[test_case("type = \"symlink\"\nmode = \"644\"" => matches Err(e) if e.to_string().contains("cannot be used") ; "mode_rejected_with_symlink")]
    #[test_case("bogus = 1" => matches Err(_) ; "unknown_property_rejected")]
    #[test_case("type = \"hardlink\"" => matches Err(_) ; "invalid_type_rejected")]
    #[test_case("mode = \"888\"" => matches Err(_) ; "invalid_mode_rejected")]
    fn rejects_invalid_rule_config(toml: &str) -> Result<RuleConfig, toml::de::Error> {
        toml::from_str(toml)
    }

    macro_rules! resolved {
        ($source:literal, $target:literal) => {
            ResolvedPortal {
                source: PathBuf::from($source),
                target: PathBuf::from($target),
            }
        };
    }

    macro_rules! rule_map {
        ($($source:literal => $config:expr),* $(,)?) => {
            indexmap::IndexMap::from_iter([
                $((Pattern::new($source).unwrap(), $config)),*
            ])
        };
    }

    macro_rules! rule_config {
        ($deploy:ident) => {
            RuleConfig {
                deploy_type: Some(DeployType::$deploy),
                mode: None,
            }
        };
        ($deploy:ident, $mode:literal) => {
            RuleConfig {
                deploy_type: Some(DeployType::$deploy),
                mode: Some(DeployMode($mode)),
            }
        };
    }

    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!()
        => deploy_entry!("vimrc", ".vimrc", Symlink);
        "no_rules_defaults_to_symlink_without_mode"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*" => rule_config!(Copy))
        => deploy_entry!("vimrc", ".vimrc", Copy);
        "copy_rule_overrides_default_symlink"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*" => rule_config!(Template))
        => deploy_entry!("vimrc", ".vimrc", Template);
        "template_rule_overrides_default_symlink"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*" => rule_config!(Copy, 0o644))
        => deploy_entry!("vimrc", ".vimrc", Copy, 0o644);
        "mode_is_applied_with_copy"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*.conf" => rule_config!(Copy, 0o600))
        => deploy_entry!("vimrc", ".vimrc", Symlink);
        "unmatched_rule_is_ignored"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*" => rule_config!(Template), "*.vimrc" => rule_config!(Copy))
        => deploy_entry!("vimrc", ".vimrc", Copy);
        "last_matching_rule_wins_type"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*" => rule_config!(Copy, 0o600), "*.vimrc" => rule_config!(Template))
        => deploy_entry!("vimrc", ".vimrc", Template, 0o600);
        "mode_survives_later_type_only_rule"
    )]
    #[test_case(
        resolved!("deep", "nested/x"), rule_map!("nested/*" => rule_config!(Template))
        => deploy_entry!("deep", "nested/x", Template);
        "pattern_applies_to_nested_target"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*" => rule_config!(Copy, 0o644), "*.vimrc" => rule_config!(Symlink))
        => deploy_entry!("vimrc", ".vimrc", Symlink);
        "mode_with_effective_symlink_is_dropped"
    )]
    #[test_case(
        resolved!("src", "a/b"), rule_map!("a/b" => rule_config!(Copy))
        => deploy_entry!("src", "a/b", Copy);
        "literal_rule_matches_exact_target"
    )]
    #[test_case(
        resolved!("src", "sub/a/b"), rule_map!("a/b" => rule_config!(Copy))
        => deploy_entry!("src", "sub/a/b", Symlink);
        "literal_rule_does_not_match_nested_prefix"
    )]
    #[test_case(
        resolved!("src", "a/b/c"), rule_map!("a/b" => rule_config!(Copy))
        => deploy_entry!("src", "a/b/c", Symlink);
        "literal_rule_does_not_match_child"
    )]
    #[test_case(
        resolved!("src", "a/b"), rule_map!("a/*" => rule_config!(Copy))
        => deploy_entry!("src", "a/b", Copy);
        "wildcard_single_component_matches_direct_child"
    )]
    #[test_case(
        resolved!("src", "a/b/c"), rule_map!("a/*" => rule_config!(Copy))
        => deploy_entry!("src", "a/b/c", Symlink);
        "wildcard_single_component_does_not_cross_separator"
    )]
    #[test_case(
        resolved!("src", "a/b"), rule_map!("*" => rule_config!(Copy))
        => deploy_entry!("src", "a/b", Symlink);
        "star_does_not_match_nested_target"
    )]
    #[test_case(
        resolved!("src", "a/b/c"), rule_map!("a/**" => rule_config!(Copy))
        => deploy_entry!("src", "a/b/c", Copy);
        "recursive_wildcard_matches_deep_target"
    )]
    #[test_case(
        resolved!("src", "a/b"), rule_map!("a/**" => rule_config!(Copy))
        => deploy_entry!("src", "a/b", Copy);
        "recursive_wildcard_matches_direct_child"
    )]
    fn applies_matching_rules(
        entry: ResolvedPortal,
        rules: indexmap::IndexMap<Pattern, RuleConfig>,
    ) -> DeploymentEntry {
        let dir = tempdir().expect("cannot create temp dir");
        let actual = match apply_rules(entry, &rules, dir.path()) {
            Ok(actual) => actual,
            Err(error) => panic!("apply_rules failed: {error}"),
        };
        let mut actual = actual;
        actual.target_path = actual
            .target_path
            .strip_prefix(dir.path())
            .expect("target path is under the temp dir")
            .to_path_buf();
        actual
    }

    #[test]
    fn mode_with_effective_symlink_warns_on_stderr() {
        crate::report::clear();
        let dir = tempdir().expect("cannot create temp dir");
        let entry = apply_rules(
            resolved!("vimrc", "a/b"),
            &rule_map!("a/*" => rule_config!(Copy, 0o600), "a/b" => rule_config!(Symlink)),
            dir.path(),
        )
        .expect("apply_rules failed");
        assert_eq!(entry.deploy_type, DeployType::Symlink);
        assert_eq!(entry.mode, None);
        assert_eq!(
            crate::report::take_errors(),
            "ignoring `mode` for `a/b`: effective `type` is `symlink`\n"
        );
        assert_eq!(crate::report::take_output(), "");
    }
}
