use crate::command::{self, Invoke, LintOrTidyCommandType, PathArgs, WorkingDir};
use anyhow::Result;
use indexmap::IndexMap;
use log::warn;
use serde::{de, de::Deserializer, Deserialize};
use std::{
    collections::HashMap,
    fmt, fs,
    marker::PhantomData,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize)]
#[allow(clippy::module_name_repetitions)]
pub struct CommandConfig {
    #[serde(rename = "type")]
    pub(crate) command_type: LintOrTidyCommandType,
    #[serde(deserialize_with = "string_or_seq_string")]
    pub(crate) include: Vec<String>,
    #[serde(default, deserialize_with = "string_or_seq_string")]
    pub(crate) exclude: Vec<String>,
    #[serde(default)]
    pub(crate) invoke: Option<Invoke>,
    #[serde(default, alias = "working-dir", deserialize_with = "working_dir")]
    pub(crate) working_dir: Option<WorkingDir>,
    #[serde(default, alias = "path-args")]
    pub(crate) path_args: Option<PathArgs>,
    #[serde(default, alias = "run-mode")]
    pub(crate) run_mode: Option<OldRunMode>,
    #[serde(default)]
    pub(crate) chdir: Option<bool>,
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
    #[serde(default = "empty_string", alias = "path-flag")]
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub(crate) enum OldRunMode {
    #[serde(rename = "files")]
    Files,
    #[serde(rename = "dirs")]
    Dirs,
    #[serde(rename = "root")]
    Root,
}

fn empty_string() -> String {
    String::new()
}

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    #[serde(default, deserialize_with = "string_or_seq_string")]
    pub(crate) exclude: Vec<String>,
    commands: IndexMap<String, CommandConfig>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ConfigError {
    #[error("File at {} cannot be read: {error:}", file.display())]
    FileCannotBeRead { file: PathBuf, error: String },
    #[error(
        "The {name:} command mixes old command params (run_mode or chdir) with new command params (invoke, working-dir, or path-args)"
    )]
    CannotMixOldAndNewCommandParams { name: String },
    #[error(r#"Cannot set invoke = "per-file" and path-args = "{path_args:}""#)]
    CannotInvokePerFileWithPathArgs { path_args: PathArgs },
    #[error(r#"Cannot set invoke = "per-dir" and path-args = "{path_args:}""#)]
    CannotInvokePerDirInRootWithPathArgs { path_args: PathArgs },
    #[error(r#"Cannot set invoke = "once" and working-dir = "dir""#)]
    CannotInvokeOnceWithWorkingDirEqDir,
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
}

// Copied from https://stackoverflow.com/a/43627388 - CC-BY-SA 3.0
fn string_or_seq_string<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct StringOrVec(PhantomData<Vec<String>>);

    impl<'de> de::Visitor<'de> for StringOrVec {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("string or list of strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(vec![value.to_owned()])
        }

        fn visit_seq<S>(self, visitor: S) -> Result<Self::Value, S::Error>
        where
            S: de::SeqAccess<'de>,
        {
            Deserialize::deserialize(de::value::SeqAccessDeserializer::new(visitor))
        }
    }

    deserializer.deserialize_any(StringOrVec(PhantomData))
}

#[allow(clippy::too_many_lines)]
fn u8_or_seq_u8<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    struct U8OrVec(PhantomData<Vec<u8>>);

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    impl<'de> de::Visitor<'de> for U8OrVec {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("integer or list of integers, each from 0-255")
        }

        fn visit_i8<E>(self, value: i8) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Signed(i64::from(value)),
                    &"an integer from 0-255",
                ));
            }

            Ok(vec![value as u8])
        }

        fn visit_i16<E>(self, value: i16) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 || value > i16::from(u8::MAX) {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Signed(i64::from(value)),
                    &"an integer from 0-255",
                ));
            }

            Ok(vec![value as u8])
        }

        fn visit_i32<E>(self, value: i32) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 || value > i32::from(u8::MAX) {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Signed(i64::from(value)),
                    &"an integer from 0-255",
                ));
            }

            Ok(vec![value as u8])
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 || value > i64::from(u8::MAX) {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Signed(value),
                    &"an integer from 0-255",
                ));
            }

            Ok(vec![value as u8])
        }

        fn visit_u8<E>(self, value: u8) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(vec![value])
        }

        fn visit_u16<E>(self, value: u16) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value > u16::from(u8::MAX) {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Unsigned(u64::from(value)),
                    &"an integer from 0-255",
                ));
            }

            Ok(vec![value as u8])
        }

        fn visit_u32<E>(self, value: u32) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value > u32::from(u8::MAX) {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Unsigned(u64::from(value)),
                    &"an integer from 0-255",
                ));
            }

            Ok(vec![value as u8])
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value > u64::from(u8::MAX) {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Unsigned(value),
                    &"an integer from 0-255",
                ));
            }

            Ok(vec![value as u8])
        }

        fn visit_seq<S>(self, visitor: S) -> Result<Self::Value, S::Error>
        where
            S: de::SeqAccess<'de>,
        {
            Deserialize::deserialize(de::value::SeqAccessDeserializer::new(visitor))
        }
    }

    deserializer.deserialize_any(U8OrVec(PhantomData))
}

fn working_dir<'de, D>(deserializer: D) -> Result<Option<WorkingDir>, D::Error>
where
    D: Deserializer<'de>,
{
    struct WorkingDirOrChdirTo(PhantomData<Option<WorkingDir>>);

    impl<'de> de::Visitor<'de> for WorkingDirOrChdirTo {
        type Value = Option<WorkingDir>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str(r#"one of "root", "dir", or a chdir-to map"#)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_any(WorkingDirOrChdirTo(PhantomData))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            match value {
                "root" => Ok(Some(WorkingDir::Root)),
                "dir" => Ok(Some(WorkingDir::Dir)),
                _ => Err(E::invalid_value(
                    de::Unexpected::Str(value),
                    &r#""root" or "dir""#,
                )),
            }
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: de::MapAccess<'de>,
        {
            let mut kv_pairs: Vec<(String, String)> = vec![];
            while let Some((k, v)) = map.next_entry::<String, String>()? {
                if !(&k == "chdir_to" || &k == "chdir-to") {
                    return Err(<A::Error as de::Error>::invalid_value(
                        de::Unexpected::Str(&k),
                        &r#"the only valid key for a working-dir map is "chdir-to""#,
                    ));
                }
                if v.is_empty() {
                    return Err(<A::Error as de::Error>::invalid_value(
                        de::Unexpected::Seq,
                        &r#"the "chdir-to" key cannot be empty"#,
                    ));
                }
                kv_pairs.push((k, v));
            }

            if kv_pairs.is_empty() {
                return Err(<A::Error as de::Error>::invalid_value(
                    de::Unexpected::Map,
                    &r#"the "working-dir" cannot be an empty map"#,
                ));
            }

            if kv_pairs.len() > 1 {
                return Err(<A::Error as de::Error>::invalid_value(
                    de::Unexpected::Map,
                    &r#"the "working-dir" map must contain one key, "chdir-to""#,
                ));
            }

            Ok(Some(WorkingDir::ChdirTo(PathBuf::from(&kv_pairs[0].1))))
        }
    }

    deserializer.deserialize_any(WorkingDirOrChdirTo(PhantomData))
}

const DEFAULT_LABEL: &str = "default";

impl Config {
    pub(crate) fn new(file: &Path) -> Result<Config> {
        match fs::read(file) {
            Err(e) => Err(ConfigError::FileCannotBeRead {
                file: file.to_path_buf(),
                error: e.to_string(),
            }
            .into()),
            Ok(bytes) => {
                let s = String::from_utf8(bytes)?;
                Ok(toml::from_str(&s)?)
            }
        }
    }

    pub(crate) fn into_tidy_commands(
        self,
        project_root: &Path,
        command: Option<&str>,
        label: Option<&str>,
    ) -> Result<Vec<command::LintOrTidyCommand>> {
        self.into_commands(project_root, command, label, LintOrTidyCommandType::Tidy)
    }

    pub(crate) fn into_lint_commands(
        self,
        project_root: &Path,
        command: Option<&str>,
        label: Option<&str>,
    ) -> Result<Vec<command::LintOrTidyCommand>> {
        self.into_commands(project_root, command, label, LintOrTidyCommandType::Lint)
    }

    fn into_commands(
        self,
        project_root: &Path,
        command: Option<&str>,
        label: Option<&str>,
        command_type: LintOrTidyCommandType,
    ) -> Result<Vec<command::LintOrTidyCommand>> {
        let mut commands: Vec<command::LintOrTidyCommand> = vec![];
        for (name, c) in self.commands {
            if let Some(c) = command {
                if name != c {
                    continue;
                }
            }

            if !c.matches_label(label.unwrap_or(DEFAULT_LABEL)) {
                continue;
            }

            if c.command_type != command_type && c.command_type != LintOrTidyCommandType::Both {
                continue;
            }

            commands.push(c.into_command(project_root, name)?);
        }

        Ok(commands)
    }

    pub(crate) fn command_info(self) -> Vec<(String, CommandConfig)> {
        self.commands.into_iter().collect()
    }
}

impl CommandConfig {
    fn into_command(self, project_root: &Path, name: String) -> Result<command::LintOrTidyCommand> {
        let n = command::LintOrTidyCommand::new(self.into_command_params(project_root, name)?)?;
        Ok(n)
    }

    fn into_command_params(
        self,
        project_root: &Path,
        name: String,
    ) -> Result<command::LintOrTidyCommandParams> {
        let (invoke, working_dir, path_args) = Self::invoke_args(
            &name,
            self.run_mode,
            self.chdir,
            self.invoke,
            self.working_dir,
            self.path_args,
        )?;
        Ok(command::LintOrTidyCommandParams {
            project_root: project_root.to_owned(),
            name,
            command_type: self.command_type,
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
        name: &str,
        run_mode: Option<OldRunMode>,
        chdir: Option<bool>,
        invoke: Option<Invoke>,
        working_dir: Option<WorkingDir>,
        path_args: Option<PathArgs>,
    ) -> Result<(Invoke, WorkingDir, PathArgs)> {
        if (run_mode.is_some() || chdir.is_some())
            && (invoke.is_some() || working_dir.is_some() || path_args.is_some())
        {
            return Err(ConfigError::CannotMixOldAndNewCommandParams {
                name: name.to_owned(),
            }
            .into());
        }

        // This translates the old config options into their equivalent new
        // options.
        if run_mode.is_some() || chdir.is_some() {
            let (article, plural, options) = match (run_mode, chdir) {
                (Some(_), None) => ("a ", "", "run-mode"),
                (None, Some(_)) => ("a ", "", "chdir"),
                _ => ("", "s", "run-mode and chdir"),
            };
            warn!("The {name} command is using {article:}deprecated config option{plural:}: {options}");

            match (run_mode, chdir) {
                (Some(OldRunMode::Files) | None, Some(false) | None) => {
                    return Ok((Invoke::PerFile, WorkingDir::Root, PathArgs::File))
                }
                (Some(OldRunMode::Files) | None, Some(true)) => {
                    return Ok((Invoke::PerFile, WorkingDir::Dir, PathArgs::File))
                }
                (Some(OldRunMode::Dirs), Some(false) | None) => {
                    return Ok((Invoke::PerDir, WorkingDir::Root, PathArgs::Dir))
                }
                (Some(OldRunMode::Dirs), Some(true)) => {
                    return Ok((Invoke::PerDir, WorkingDir::Dir, PathArgs::None))
                }
                (Some(OldRunMode::Root), Some(false) | None) => {
                    return Ok((Invoke::Once, WorkingDir::Root, PathArgs::Dot))
                }
                (Some(OldRunMode::Root), Some(true)) => {
                    return Ok((Invoke::Once, WorkingDir::Root, PathArgs::None))
                }
            }
        }

        let invoke = invoke.unwrap_or(Invoke::PerFile);
        let working_dir = working_dir.unwrap_or(WorkingDir::Root);
        let path_args = path_args.unwrap_or(PathArgs::File);

        match (invoke, &working_dir, path_args) {
            (Invoke::PerFile, _, path_args) => {
                if path_args != PathArgs::File && path_args != PathArgs::AbsoluteFile {
                    return Err(ConfigError::CannotInvokePerFileWithPathArgs { path_args }.into());
                }
            }
            (Invoke::PerDir, &WorkingDir::Root | &WorkingDir::ChdirTo(_), path_args) => {
                if path_args == PathArgs::Dot || path_args == PathArgs::None {
                    return Err(
                        ConfigError::CannotInvokePerDirInRootWithPathArgs { path_args }.into(),
                    );
                }
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
    use pretty_assertions::assert_eq;
    use serial_test::parallel;
    use test_case::test_case;

    #[test_case(
        Some("files"),
        Some(false),
        Invoke::PerFile,
        WorkingDir::Root,
        PathArgs::File ;
        "files + false"
    )]
    #[test_case(
        Some("files"),
        Some(true),
        Invoke::PerFile,
        WorkingDir::Dir,
        PathArgs::File ;
        "files + true"
    )]
    #[test_case(
        Some("dirs"),
        Some(false),
        Invoke::PerDir,
        WorkingDir::Root,
        PathArgs::Dir ;
        "dirs + false"
    )]
    #[test_case(
        Some("dirs"),
        Some(true),
        Invoke::PerDir,
        WorkingDir::Dir,
        PathArgs::None ;
        "dirs + true"
    )]
    #[test_case(
        Some("root"),
        Some(false),
        Invoke::Once,
        WorkingDir::Root,
        PathArgs::Dot ;
        "root + false"
    )]
    #[test_case(
        Some("root"),
        Some(true),
        Invoke::Once,
        WorkingDir::Root,
        PathArgs::None ;
        "root + true"
    )]
    #[test_case(
        Some("files"),
        None,
        Invoke::PerFile,
        WorkingDir::Root,
        PathArgs::File ;
        "files + None"
    )]
    #[test_case(
        Some("dirs"),
        None,
        Invoke::PerDir,
        WorkingDir::Root,
        PathArgs::Dir ;
        "dirs + None"
    )]
    #[test_case(
        Some("root"),
        None,
        Invoke::Once,
        WorkingDir::Root,
        PathArgs::Dot ;
        "root + None"
    )]
    #[test_case(
        None,
        Some(true),
        Invoke::PerFile,
        WorkingDir::Dir,
        PathArgs::File ;
        "None + true"
    )]
    #[parallel]
    fn pre_0_4_0_command_config(
        run_mode: Option<&str>,
        chdir: Option<bool>,
        invoke: Invoke,
        working_dir: WorkingDir,
        path_args: PathArgs,
    ) -> Result<()> {
        let root = Path::new("/does-not-matter");
        let mut toml_text = String::from(
            r#"
                [commands.c1]
                type    = "tidy"
                include = "**/*.rs"
                cmd     = "cmd"
                ok-exit-codes = 0
            "#,
        );
        if let Some(run_mode) = run_mode {
            toml_text.push_str(&format!("run-mode = \"{run_mode}\"\n"));
        }
        if let Some(chdir) = chdir {
            toml_text.push_str(&format!("chdir = {chdir}\n"));
        }

        let config: Config = toml::from_str(&toml_text)?;
        let params = config
            .commands
            .into_iter()
            .next()
            .map(|(name, conf)| conf.into_command_params(root, name))
            .unwrap()?;
        assert_eq!(params.invoke, invoke, "invoke");
        assert_eq!(params.working_dir, working_dir, "working_dir");
        assert_eq!(params.path_args, path_args, "path_args");

        Ok(())
    }

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
            type     = "lint"
            include  = "**/*.rs"
            run-mode = "root"
            chdir    = true
            cmd      = "$PRECIOUS_ROOT/dev/bin/force-clippy.sh"
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
            type     = "lint"
            include  = "**/*.rs"
            run-mode = "root"
            chdir    = true
            cmd      = "$PRECIOUS_ROOT/dev/bin/force-clippy.sh"
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
            type     = "lint"
            include  = "**/*.rs"
            run-mode = "root"
            chdir    = true
            cmd      = "$PRECIOUS_ROOT/dev/bin/force-clippy.sh"
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
        WorkingDir::ChdirTo(PathBuf::from("foo")),
        PathArgs::None,
        ConfigError::CannotInvokePerDirInRootWithPathArgs { path_args: PathArgs::None } ;
        r#"invoke = "per-dir" + working_dir.chdir-to = "foo" + path-args = "none""#
    )]
    #[test_case(
        Invoke::PerDir,
        WorkingDir::ChdirTo(PathBuf::from("foo")),
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
            command_type: LintOrTidyCommandType::Lint,
            invoke: Some(invoke),
            working_dir: Some(working_dir),
            path_args: Some(path_args),
            include: vec![String::from("**/*.rs")],
            exclude: vec![],
            run_mode: None,
            chdir: None,
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
        let res = config.into_command(Path::new("."), String::from("some-linter"));
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
            command_type: LintOrTidyCommandType::Lint,
            invoke: None,
            working_dir: None,
            path_args: None,
            include: vec![String::from("**/*.rs")],
            exclude: vec![],
            run_mode: None,
            chdir: None,
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
}
