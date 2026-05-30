use crate::command::{self, CommandType, Invoke, PathArgs, WorkingDir};
use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use indexmap::IndexMap;
use serde::{de, de::Deserializer, Deserialize};
use std::{collections::HashMap, fs};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize)]
#[allow(clippy::module_name_repetitions)]
pub struct CommandConfig {
    #[serde(rename = "type")]
    pub(crate) typ: CommandType,
    #[serde(default, deserialize_with = "string_or_seq_string")]
    pub(crate) include: Vec<String>,
    #[serde(default, deserialize_with = "string_or_seq_string")]
    pub(crate) exclude: Vec<String>,
    #[serde(
        default,
        alias = "shared-include",
        deserialize_with = "string_or_seq_string"
    )]
    pub(crate) shared_include: Vec<String>,
    #[serde(
        default,
        alias = "shared-exclude",
        deserialize_with = "string_or_seq_string"
    )]
    pub(crate) shared_exclude: Vec<String>,
    #[serde(default)]
    pub(crate) invoke: Option<Invoke>,
    #[serde(default, alias = "working-dir", deserialize_with = "working_dir")]
    pub(crate) working_dir: Option<WorkingDir>,
    #[serde(default, alias = "path-args")]
    pub(crate) path_args: Option<PathArgs>,
    #[serde(deserialize_with = "string_or_seq_string")]
    pub(crate) cmd: Vec<String>,
    #[serde(default)]
    pub(crate) env: HashMap<String, String>,
    #[serde(
        default,
        alias = "lint-flags",
        deserialize_with = "string_or_seq_string"
    )]
    pub(crate) lint_flags: Vec<String>,
    #[serde(
        default,
        alias = "tidy-flags",
        deserialize_with = "string_or_seq_string"
    )]
    pub(crate) tidy_flags: Vec<String>,
    #[serde(default = "String::new", alias = "path-flag")]
    pub(crate) path_flag: String,
    #[serde(alias = "ok-exit-codes", deserialize_with = "u8_or_seq_u8")]
    pub(crate) ok_exit_codes: Vec<u8>,
    #[serde(
        default,
        alias = "lint-failure-exit-codes",
        deserialize_with = "u8_or_seq_u8"
    )]
    pub(crate) lint_failure_exit_codes: Vec<u8>,
    #[serde(default, alias = "expect-stderr")]
    pub(crate) expect_stderr: bool,
    #[serde(
        default,
        alias = "ignore-stderr",
        deserialize_with = "string_or_seq_string"
    )]
    pub(crate) ignore_stderr: Vec<String>,
    #[serde(default, deserialize_with = "string_or_seq_string")]
    pub(crate) labels: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    #[serde(default, deserialize_with = "string_or_seq_string")]
    pub(crate) exclude: Vec<String>,
    #[serde(default)]
    shared: HashMap<String, Vec<String>>,
    commands: IndexMap<String, CommandConfig>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ConfigError {
    #[error("File at {file} cannot be read: {error:}")]
    FileCannotBeRead { file: Utf8PathBuf, error: String },
    #[error(r#"Cannot set invoke = "per-file" and path-args = "{path_args:}""#)]
    CannotInvokePerFileWithPathArgs { path_args: PathArgs },
    #[error(r#"Cannot set invoke = "per-dir" and path-args = "{path_args:}""#)]
    CannotInvokePerDirInRootWithPathArgs { path_args: PathArgs },
    #[error(r#"Cannot set invoke = "once" and working-dir = "dir""#)]
    CannotInvokeOnceWithWorkingDirEqDir,
    #[error("Command \"{command}\" references unknown shared key \"{key}\"")]
    UnknownSharedKey { command: String, key: String },
    #[error("Command \"{command}\" does not define an \"include\" or \"shared-include\"")]
    NoInclude { command: String },
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
}

// Provided by Claude.ai. This is much simpler than how this used to work.
fn string_or_seq_string<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        String(String),
        Vec(Vec<String>),
    }

    match StringOrVec::deserialize(deserializer)? {
        StringOrVec::String(s) => Ok(vec![s]),
        StringOrVec::Vec(v) => Ok(v),
    }
}

fn u8_or_seq_u8<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum U8OrVec {
        U8(u8),
        Vec(Vec<u8>),
    }

    match U8OrVec::deserialize(deserializer)? {
        U8OrVec::U8(s) => Ok(vec![s]),
        U8OrVec::Vec(v) => Ok(v),
    }
}

fn working_dir<'de, D>(deserializer: D) -> Result<Option<WorkingDir>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum WorkingDirSerialization {
        Simple(String),
        ChdirTo(HashMap<String, String>),
    }

    match WorkingDirSerialization::deserialize(deserializer)? {
        WorkingDirSerialization::Simple(s) => match s.as_str().try_into() {
            Ok(w) => Ok(Some(w)),
            Err(_) => Err(de::Error::invalid_value(
                de::Unexpected::Str(&s),
                &r#"one of "root", "dir", or chdir-to = "path""#,
            )),
        },
        WorkingDirSerialization::ChdirTo(c) => {
            if c.len() != 1 {
                return Err(de::Error::invalid_value(
                    de::Unexpected::Map,
                    &r#"a map with a single key, "chdir-to""#,
                ));
            }
            let (key, value) = c
                .into_iter()
                .next()
                .expect("we already know there is exactly 1 entry");
            if key != "chdir-to" {
                return Err(de::Error::invalid_value(
                    de::Unexpected::Map,
                    &r#"a map with a single key, "chdir-to""#,
                ));
            }

            Ok(Some(WorkingDir::ChdirTo(Utf8PathBuf::from(value))))
        }
    }
}

const DEFAULT_LABEL: &str = "default";

impl Config {
    pub(crate) fn new(file: &Utf8Path) -> Result<Config> {
        let bytes = fs::read(file).map_err(|e| ConfigError::FileCannotBeRead {
            file: file.to_owned(),
            error: e.to_string(),
        })?;

        let content = String::from_utf8(bytes)
            .with_context(|| format!("Config file at {file} contains invalid UTF-8"))?;

        toml::from_str::<Config>(&content)
            .with_context(|| format!("Failed to parse config file at {file} as TOML"))
    }

    pub(crate) fn into_tidy_commands(
        self,
        project_root: &Utf8Path,
        command: Option<&str>,
        label: Option<&str>,
    ) -> Result<Vec<command::Command>> {
        self.into_commands(project_root, command, label, CommandType::Tidy)
    }

    pub(crate) fn into_lint_commands(
        self,
        project_root: &Utf8Path,
        command: Option<&str>,
        label: Option<&str>,
    ) -> Result<Vec<command::Command>> {
        self.into_commands(project_root, command, label, CommandType::Lint)
    }

    fn into_commands(
        self,
        project_root: &Utf8Path,
        command: Option<&str>,
        label: Option<&str>,
        typ: CommandType,
    ) -> Result<Vec<command::Command>> {
        let mut commands: Vec<command::Command> = vec![];
        for (name, mut c) in self.commands {
            c.include.splice(
                0..0,
                Self::resolve_shared_keys(&c.shared_include, &self.shared, &name)?,
            );
            c.exclude.splice(
                0..0,
                Self::resolve_shared_keys(&c.shared_exclude, &self.shared, &name)?,
            );

            if c.include.is_empty() {
                return Err(ConfigError::NoInclude { command: name }.into());
            }

            if let Some(c) = command {
                if name != c {
                    continue;
                }
            }

            if !c.matches_label(label.unwrap_or(DEFAULT_LABEL)) {
                continue;
            }

            if c.typ != typ && c.typ != CommandType::Both {
                continue;
            }

            let cmd = c.try_into_command(project_root, &name)?;
            commands.push(cmd);
        }

        Ok(commands)
    }

    fn resolve_shared_keys(
        keys: &[String],
        shared: &HashMap<String, Vec<String>>,
        command: &str,
    ) -> Result<Vec<String>> {
        let mut result = Vec::new();
        for key in keys {
            match shared.get(key) {
                Some(globs) => result.extend_from_slice(globs),
                None => {
                    return Err(ConfigError::UnknownSharedKey {
                        command: command.to_string(),
                        key: key.clone(),
                    }
                    .into())
                }
            }
        }
        Ok(result)
    }

    // Note: returns raw CommandConfig values; shared_include/shared_exclude are not resolved.
    pub(crate) fn command_info(self) -> Vec<(String, CommandConfig)> {
        self.commands.into_iter().collect()
    }
}

impl CommandConfig {
    fn try_into_command(self, project_root: &Utf8Path, name: &str) -> Result<command::Command> {
        let params = self
            .into_command_params(project_root, name)
            .with_context(|| format!(r#"Failed to build parameters for command "{name}""#))?;
        let cmd = command::Command::new(params)
            .with_context(|| format!(r#"Failed to create command "{name}" from parameters"#))?;
        Ok(cmd)
    }

    fn into_command_params(
        self,
        project_root: &Utf8Path,
        name: &str,
    ) -> Result<command::CommandParams> {
        let (invoke, working_dir, path_args) =
            Self::invoke_args(self.invoke, self.working_dir, self.path_args).context(
                "Invalid configuration combination for command invoke/working-dir/path-args",
            )?;

        Ok(command::CommandParams {
            project_root: project_root.to_owned(),
            name: name.to_string(),
            typ: self.typ,
            include: self.include,
            exclude: self.exclude,
            invoke,
            working_dir,
            path_args,
            cmd: self.cmd,
            env: self.env,
            lint_flags: self.lint_flags,
            tidy_flags: self.tidy_flags,
            path_flag: self.path_flag,
            ok_exit_codes: self.ok_exit_codes,
            lint_failure_exit_codes: self.lint_failure_exit_codes,
            expect_stderr: self.expect_stderr,
            ignore_stderr: self.ignore_stderr,
        })
    }

    fn invoke_args(
        invoke: Option<Invoke>,
        working_dir: Option<WorkingDir>,
        path_args: Option<PathArgs>,
    ) -> Result<(Invoke, WorkingDir, PathArgs)> {
        let invoke = invoke.unwrap_or(Invoke::PerFile);
        let working_dir = working_dir.unwrap_or(WorkingDir::Root);
        let path_args = path_args.unwrap_or(PathArgs::File);

        match (invoke, &working_dir, path_args) {
            (Invoke::PerFile, _, path_args)
                if path_args != PathArgs::File && path_args != PathArgs::AbsoluteFile =>
            {
                return Err(ConfigError::CannotInvokePerFileWithPathArgs { path_args }.into());
            }
            (Invoke::PerDir, &WorkingDir::Root | &WorkingDir::ChdirTo(_), path_args)
                if path_args == PathArgs::Dot || path_args == PathArgs::None =>
            {
                return Err(ConfigError::CannotInvokePerDirInRootWithPathArgs { path_args }.into());
            }
            (Invoke::Once, &WorkingDir::Dir, _) => {
                return Err(ConfigError::CannotInvokeOnceWithWorkingDirEqDir.into());
            }
            _ => (),
        }

        Ok((invoke, working_dir, path_args))
    }

    fn matches_label(&self, label: &str) -> bool {
        if self.labels.is_empty() {
            return label == DEFAULT_LABEL;
        }
        self.labels.iter().any(|l| *l == label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::{Utf8Path, Utf8PathBuf};
    use mitsein::prelude::*;
    use pretty_assertions::assert_eq;
    use serial_test::parallel;
    use test_case::test_case;

    #[test]
    #[parallel]
    fn command_order_is_preserved1() -> Result<()> {
        let toml_text = r#"
            [commands.rustfmt]
            type    = "both"
            include = "**/*.rs"
            cmd     = [ "rustfmt", "--skip-children", "--unstable-features" ]
            lint-flags = "--check"
            ok-exit-codes = 0
            lint-failure-exit-codes = 1

            [commands.clippy]
            type      = "lint"
            include   = "**/*.rs"
            invoke    = "once"
            path-args = "none"
            cmd       = "$PRECIOUS_ROOT/dev/bin/force-clippy.sh"
            ok-exit-codes = 0
            lint-failure-exit-codes = 101

            [commands.omegasort-gitignore]
            type = "both"
            include = "**/.gitignore"
            cmd = [ "omegasort", "--sort=path" ]
            lint-flags = "--check"
            tidy-flags = "--in-place"
            ok-exit-codes = 0
            lint-failure-exit-codes = 1
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let keys = config
            .commands
            .keys()
            .map(|k| k.as_str())
            .collect::<Vec<&str>>();
        let expect: Vec<&str> = vec!["rustfmt", "clippy", "omegasort-gitignore"];
        assert_eq!(keys, expect);

        Ok(())
    }

    #[test]
    #[parallel]
    fn command_order_is_preserved2() -> Result<()> {
        let toml_text = r#"
            [commands.clippy]
            type      = "lint"
            include   = "**/*.rs"
            invoke    = "once"
            path-args = "none"
            cmd       = "$PRECIOUS_ROOT/dev/bin/force-clippy.sh"
            ok-exit-codes = 0
            lint-failure-exit-codes = 101

            [commands.rustfmt]
            type    = "both"
            include = "**/*.rs"
            cmd     = [ "rustfmt", "--skip-children", "--unstable-features" ]
            lint-flags = "--check"
            ok-exit-codes = 0
            lint-failure-exit-codes = 1

            [commands.omegasort-gitignore]
            type = "both"
            include = "**/.gitignore"
            cmd = [ "omegasort", "--sort=path" ]
            lint-flags = "--check"
            tidy-flags = "--in-place"
            ok-exit-codes = 0
            lint-failure-exit-codes = 1
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let keys = config
            .commands
            .keys()
            .map(|k| k.as_str())
            .collect::<Vec<&str>>();
        let expect: Vec<&str> = vec!["clippy", "rustfmt", "omegasort-gitignore"];
        assert_eq!(keys, expect);

        Ok(())
    }

    #[test]
    #[parallel]
    fn command_order_is_preserved3() -> Result<()> {
        let toml_text = r#"
            [commands.omegasort-gitignore]
            type = "both"
            include = "**/.gitignore"
            cmd = [ "omegasort", "--sort=path" ]
            lint-flags = "--check"
            tidy-flags = "--in-place"
            ok-exit-codes = 0
            lint-failure-exit-codes = 1

            [commands.clippy]
            type      = "lint"
            include   = "**/*.rs"
            invoke    = "once"
            path-args = "none"
            cmd       = "$PRECIOUS_ROOT/dev/bin/force-clippy.sh"
            ok-exit-codes = 0
            lint-failure-exit-codes = 101

            [commands.rustfmt]
            type    = "both"
            include = "**/*.rs"
            cmd     = [ "rustfmt", "--skip-children", "--unstable-features" ]
            lint-flags = "--check"
            ok-exit-codes = 0
            lint-failure-exit-codes = 1
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let keys = config
            .commands
            .keys()
            .map(|k| k.as_str())
            .collect::<Vec<&str>>();
        let expect: Vec<&str> = vec!["omegasort-gitignore", "clippy", "rustfmt"];
        assert_eq!(keys, expect);

        Ok(())
    }

    #[test_case(
        Invoke::PerFile,
        WorkingDir::Root,
        PathArgs::Dir,
        ConfigError::CannotInvokePerFileWithPathArgs { path_args: PathArgs::Dir } ;
        r#"invoke = "per-file" + path-args = "dir""#
    )]
    #[test_case(
        Invoke::PerFile,
        WorkingDir::Root,
        PathArgs::None,
        ConfigError::CannotInvokePerFileWithPathArgs { path_args: PathArgs::None } ;
        r#"invoke = "per-file" + path-args = "none""#
    )]
    #[test_case(
        Invoke::PerFile,
        WorkingDir::Root,
        PathArgs::Dot,
        ConfigError::CannotInvokePerFileWithPathArgs { path_args: PathArgs::Dot } ;
        r#"invoke = "per-file" + path-args = "dot""#
    )]
    #[test_case(
        Invoke::PerFile,
        WorkingDir::Root,
        PathArgs::AbsoluteDir,
        ConfigError::CannotInvokePerFileWithPathArgs { path_args: PathArgs::AbsoluteDir } ;
        r#"invoke = "per-file" + path-args = "absolute-dir""#
    )]
    #[test_case(
        Invoke::PerDir,
        WorkingDir::Root,
        PathArgs::None,
        ConfigError::CannotInvokePerDirInRootWithPathArgs { path_args: PathArgs::None } ;
        r#"invoke = "per-dir" + working_dir = "root" + path-args = "none""#
    )]
    #[test_case(
        Invoke::PerDir,
        WorkingDir::Root,
        PathArgs::Dot,
        ConfigError::CannotInvokePerDirInRootWithPathArgs { path_args: PathArgs::Dot } ;
        r#"invoke = "per-dir" + working_dir = "root" + path-args = "dot""#
    )]
    #[test_case(
        Invoke::PerDir,
        WorkingDir::ChdirTo(Utf8PathBuf::from("foo")),
        PathArgs::None,
        ConfigError::CannotInvokePerDirInRootWithPathArgs { path_args: PathArgs::None } ;
        r#"invoke = "per-dir" + working_dir.chdir-to = "foo" + path-args = "none""#
    )]
    #[test_case(
        Invoke::PerDir,
        WorkingDir::ChdirTo(Utf8PathBuf::from("foo")),
        PathArgs::Dot,
        ConfigError::CannotInvokePerDirInRootWithPathArgs { path_args: PathArgs::Dot } ;
        r#"invoke = "per-dir" + working_dir.chdir-to = "foo" + path-args = "dot""#
    )]
    #[test_case(
        Invoke::Once,
        WorkingDir::Dir,
        PathArgs::File,
        ConfigError::CannotInvokeOnceWithWorkingDirEqDir ;
        r#"invoke = "once" + working_dir = "dir""#
    )]
    #[parallel]
    fn invalid_command_config(
        invoke: Invoke,
        working_dir: WorkingDir,
        path_args: PathArgs,
        expect_err: ConfigError,
    ) -> Result<()> {
        let config = CommandConfig {
            typ: CommandType::Lint,
            invoke: Some(invoke),
            working_dir: Some(working_dir),
            path_args: Some(path_args),
            include: vec![String::from("**/*.rs")],
            exclude: vec![],
            shared_include: vec![],
            shared_exclude: vec![],
            cmd: vec![String::from("some-linter")],
            env: Default::default(),
            lint_flags: vec![],
            tidy_flags: vec![],
            path_flag: String::new(),
            ok_exit_codes: vec![],
            lint_failure_exit_codes: vec![],
            expect_stderr: false,
            ignore_stderr: vec![],
            labels: vec![],
        };
        let res = config.try_into_command(Utf8Path::new("."), "some-linter");
        let err = res.unwrap_err().downcast::<ConfigError>().unwrap();
        assert_eq!(err, expect_err);

        Ok(())
    }

    #[test_case(vec![], "default", true)]
    #[test_case(vec!["default".to_string()], "default", true)]
    #[test_case(vec!["default".to_string(), "foo".to_string()], "default", true)]
    #[test_case(vec!["default".to_string(), "foo".to_string()], "foo", true)]
    #[test_case(vec!["foo".to_string()], "foo", true)]
    #[test_case(vec![], "foo", false)]
    #[test_case(vec!["foo".to_string()], "default", false)]
    #[test_case(vec!["default".to_string()], "foo", false)]
    #[parallel]
    fn matches_label(
        labels_in_config: Vec<String>,
        label_to_match: &str,
        expect_match: bool,
    ) -> Result<()> {
        let config = CommandConfig {
            typ: CommandType::Lint,
            invoke: None,
            working_dir: None,
            path_args: None,
            include: vec![String::from("**/*.rs")],
            exclude: vec![],
            shared_include: vec![],
            shared_exclude: vec![],
            cmd: vec![String::from("some-linter")],
            env: Default::default(),
            lint_flags: vec![],
            tidy_flags: vec![],
            path_flag: String::new(),
            ok_exit_codes: vec![],
            lint_failure_exit_codes: vec![],
            expect_stderr: false,
            ignore_stderr: vec![],
            labels: labels_in_config,
        };
        if expect_match {
            assert!(config.matches_label(label_to_match));
        } else {
            assert!(!config.matches_label(label_to_match));
        }

        Ok(())
    }

    #[test_case(
        r#""per-file-or-dir" = 42"#,
        Invoke::PerFileOrDir(42);
        "per-file-or-dir"
    )]
    #[test_case(
        r#""per-file-or-once" = 42"#,
        Invoke::PerFileOrOnce(42);
        "per-file-or-once"
    )]
    #[test_case(
        r#""per-dir-or-once" = 42"#,
        Invoke::PerDirOrOnce(42);
        "per-dir-or-once"
    )]
    #[parallel]
    fn new_invoke_options(invoke: &str, expect: Invoke) -> Result<()> {
        let toml_text = format!(
            r#"
            [commands.omegasort-gitignore]
            type = "both"
            include = "**/.gitignore"
            invoke = {{ {invoke:} }}
            cmd = [ "omegasort", "--sort=path" ]
            lint-flags = "--check"
            tidy-flags = "--in-place"
            ok-exit-codes = 0
            lint-failure-exit-codes = 1
        "#
        );

        let config: Config = toml::from_str(&toml_text)?;
        assert_eq!(config.commands[0].invoke, Some(expect));

        Ok(())
    }

    #[test]
    #[parallel]
    fn shared_include_single_key() -> Result<()> {
        let toml_text = r#"
            [shared]
            js = ["**/*.js", "**/*.jsx"]

            [commands.prettier]
            type = "lint"
            shared-include = "js"
            cmd = ["prettier", "--check"]
            ok-exit-codes = 0
            lint-failure-exit-codes = 1
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let commands = config.into_lint_commands(Utf8Path::new("."), None, None)?;
        assert_eq!(commands.len(), 1);

        let files: Vec1<Utf8PathBuf> = vec1![
            Utf8PathBuf::from("src/foo.js"),
            Utf8PathBuf::from("src/bar.jsx"),
            Utf8PathBuf::from("src/baz.rs"),
        ];
        let (arg_sets, _) = commands[0].files_to_args_sets(&files)?;
        let mut included: Vec<&Utf8Path> = arg_sets.into_iter().flatten().collect();
        included.sort();
        assert_eq!(
            included,
            vec![Utf8Path::new("src/bar.jsx"), Utf8Path::new("src/foo.js")]
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn shared_include_multiple_keys() -> Result<()> {
        let toml_text = r#"
            [shared]
            js = ["**/*.js"]
            styles = ["**/*.css", "**/*.scss"]

            [commands.prettier]
            type = "lint"
            shared-include = ["js", "styles"]
            cmd = ["prettier", "--check"]
            ok-exit-codes = 0
            lint-failure-exit-codes = 1
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let commands = config.into_lint_commands(Utf8Path::new("."), None, None)?;
        assert_eq!(commands.len(), 1);

        let files: Vec1<Utf8PathBuf> = vec1![
            Utf8PathBuf::from("src/app.js"),
            Utf8PathBuf::from("src/main.css"),
            Utf8PathBuf::from("src/theme.scss"),
            Utf8PathBuf::from("src/main.rs"),
        ];
        let (arg_sets, _) = commands[0].files_to_args_sets(&files)?;
        let mut included: Vec<&Utf8Path> = arg_sets.into_iter().flatten().collect();
        included.sort();
        assert_eq!(
            included,
            vec![
                Utf8Path::new("src/app.js"),
                Utf8Path::new("src/main.css"),
                Utf8Path::new("src/theme.scss"),
            ]
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn shared_exclude() -> Result<()> {
        let toml_text = r#"
            [shared]
            generated = ["vendor/**/*", "generated/**/*"]

            [commands.eslint]
            type = "lint"
            include = "**/*.js"
            shared-exclude = "generated"
            cmd = ["eslint"]
            ok-exit-codes = 0
            lint-failure-exit-codes = 1
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let commands = config.into_lint_commands(Utf8Path::new("."), None, None)?;
        assert_eq!(commands.len(), 1);

        let files: Vec1<Utf8PathBuf> = vec1![
            Utf8PathBuf::from("src/app.js"),
            Utf8PathBuf::from("vendor/lib.js"),
            Utf8PathBuf::from("generated/out.js"),
        ];
        let (arg_sets, _) = commands[0].files_to_args_sets(&files)?;
        let mut included: Vec<&Utf8Path> = arg_sets.into_iter().flatten().collect();
        included.sort();
        assert_eq!(included, vec![Utf8Path::new("src/app.js")]);

        Ok(())
    }

    #[test]
    #[parallel]
    fn shared_mixed_with_per_command() -> Result<()> {
        let toml_text = r#"
            [shared]
            js = ["**/*.js"]
            generated = ["vendor/**/*"]

            [commands.eslint]
            type = "lint"
            shared-include = "js"
            include = ["**/*.ts"]
            shared-exclude = "generated"
            exclude = ["**/*.test.ts"]
            cmd = ["eslint"]
            ok-exit-codes = 0
            lint-failure-exit-codes = 1
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let commands = config.into_lint_commands(Utf8Path::new("."), None, None)?;
        assert_eq!(commands.len(), 1);

        let files: Vec1<Utf8PathBuf> = vec1![
            Utf8PathBuf::from("src/app.js"),
            Utf8PathBuf::from("src/app.ts"),
            Utf8PathBuf::from("src/app.test.ts"),
            Utf8PathBuf::from("vendor/lib.js"),
        ];
        let (arg_sets, _) = commands[0].files_to_args_sets(&files)?;
        let mut included: Vec<&Utf8Path> = arg_sets.into_iter().flatten().collect();
        included.sort();
        assert_eq!(
            included,
            vec![Utf8Path::new("src/app.js"), Utf8Path::new("src/app.ts")]
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn shared_unknown_key() -> Result<()> {
        let toml_text = r#"
            [shared]
            js = ["**/*.js"]

            [commands.prettier]
            type = "lint"
            shared-include = "typescript"
            cmd = ["prettier", "--check"]
            ok-exit-codes = 0
            lint-failure-exit-codes = 1
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let err = config
            .into_lint_commands(Utf8Path::new("."), None, None)
            .unwrap_err()
            .downcast::<ConfigError>()?;
        assert_eq!(
            err,
            ConfigError::UnknownSharedKey {
                command: "prettier".to_string(),
                key: "typescript".to_string(),
            }
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn no_include_or_shared_include() -> Result<()> {
        let toml_text = r#"
            [commands.prettier]
            type = "lint"
            cmd = ["prettier", "--check"]
            ok-exit-codes = 0
            lint-failure-exit-codes = 1
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let err = config
            .into_lint_commands(Utf8Path::new("."), None, None)
            .unwrap_err()
            .downcast::<ConfigError>()?;
        assert_eq!(
            err,
            ConfigError::NoInclude {
                command: "prettier".to_string(),
            }
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn shared_include_only() -> Result<()> {
        let toml_text = r#"
            [shared]
            rs = ["**/*.rs"]

            [commands.rustfmt]
            type = "lint"
            shared-include = "rs"
            cmd = ["rustfmt", "--check"]
            ok-exit-codes = 0
            lint-failure-exit-codes = 1
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let commands = config.into_lint_commands(Utf8Path::new("."), None, None)?;
        assert_eq!(commands.len(), 1);

        let files: Vec1<Utf8PathBuf> = vec1![
            Utf8PathBuf::from("src/lib.rs"),
            Utf8PathBuf::from("src/main.rs"),
            Utf8PathBuf::from("src/main.js"),
        ];
        let (arg_sets, _) = commands[0].files_to_args_sets(&files)?;
        let mut included: Vec<&Utf8Path> = arg_sets.into_iter().flatten().collect();
        included.sort();
        assert_eq!(
            included,
            vec![Utf8Path::new("src/lib.rs"), Utf8Path::new("src/main.rs")]
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn no_include_on_non_matching_command_is_still_an_error() -> Result<()> {
        // A command with no include should be caught even when --command
        // selects a different command — validation runs before the name filter.
        let toml_text = r#"
            [commands.rustfmt]
            type = "lint"
            cmd = ["rustfmt", "--check"]
            ok-exit-codes = 0

            [commands.clippy]
            type = "lint"
            include = "**/*.rs"
            cmd = ["clippy"]
            ok-exit-codes = 0
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let err = config
            .into_lint_commands(Utf8Path::new("."), Some("clippy"), None)
            .unwrap_err()
            .downcast::<ConfigError>()?;
        assert_eq!(
            err,
            ConfigError::NoInclude {
                command: "rustfmt".to_string(),
            }
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn no_include_on_non_matching_label_is_still_an_error() -> Result<()> {
        // A command with no include should be caught even when a label filter
        // would exclude it — validation runs before the label filter.
        let toml_text = r#"
            [commands.rustfmt]
            type = "lint"
            cmd = ["rustfmt", "--check"]
            ok-exit-codes = 0

            [commands.clippy]
            type = "lint"
            include = "**/*.rs"
            labels = ["ci"]
            cmd = ["clippy"]
            ok-exit-codes = 0
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let err = config
            .into_lint_commands(Utf8Path::new("."), None, Some("ci"))
            .unwrap_err()
            .downcast::<ConfigError>()?;
        assert_eq!(
            err,
            ConfigError::NoInclude {
                command: "rustfmt".to_string(),
            }
        );

        Ok(())
    }

    #[test]
    #[parallel]
    fn no_include_on_non_matching_type_is_still_an_error() -> Result<()> {
        // A tidy-only command with no include should be caught even when
        // calling into_lint_commands — validation runs before the type filter.
        let toml_text = r#"
            [commands.rustfmt]
            type = "tidy"
            cmd = ["rustfmt"]
            ok-exit-codes = 0
        "#;

        let config: Config = toml::from_str(toml_text)?;
        let err = config
            .into_lint_commands(Utf8Path::new("."), None, None)
            .unwrap_err()
            .downcast::<ConfigError>()?;
        assert_eq!(
            err,
            ConfigError::NoInclude {
                command: "rustfmt".to_string(),
            }
        );

        Ok(())
    }
}
