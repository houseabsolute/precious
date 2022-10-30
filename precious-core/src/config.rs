use crate::command::{self, Invoke, PathArgs, WorkingDir};
use anyhow::Result;
use indexmap::IndexMap;
use serde::{de, de::Deserializer, Deserialize};
use std::{
    collections::HashMap,
    fmt, fs,
    marker::PhantomData,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize)]
pub struct Command {
    #[serde(rename = "type")]
    typ: command::CommandType,
    #[serde(deserialize_with = "string_or_seq_string")]
    include: Vec<String>,
    #[serde(default)]
    #[serde(deserialize_with = "string_or_seq_string")]
    exclude: Vec<String>,
    #[serde(default)]
    invoke: Option<Invoke>,
    #[serde(default)]
    #[serde(deserialize_with = "working_dir")]
    working_dir: Option<WorkingDir>,
    #[serde(default)]
    path_args: Option<PathArgs>,
    #[serde(default)]
    run_mode: Option<OldRunMode>,
    #[serde(default)]
    chdir: Option<bool>,
    #[serde(deserialize_with = "string_or_seq_string")]
    cmd: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    #[serde(deserialize_with = "string_or_seq_string")]
    lint_flags: Vec<String>,
    #[serde(default)]
    #[serde(deserialize_with = "string_or_seq_string")]
    tidy_flags: Vec<String>,
    #[serde(default = "empty_string")]
    path_flag: String,
    #[serde(deserialize_with = "u8_or_seq_u8")]
    ok_exit_codes: Vec<u8>,
    #[serde(default)]
    #[serde(deserialize_with = "u8_or_seq_u8")]
    lint_failure_exit_codes: Vec<u8>,
    #[serde(default)]
    expect_stderr: bool,
    #[serde(default)]
    #[serde(deserialize_with = "string_or_seq_string")]
    ignore_stderr: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum OldRunMode {
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
    #[serde(default)]
    #[serde(deserialize_with = "string_or_seq_string")]
    pub exclude: Vec<String>,
    commands: IndexMap<String, Command>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("File at {} cannot be read: {error:}", file.display())]
    FileCannotBeRead {
        file: PathBuf,
        error: std::io::Error,
    },
    #[error(
        "The {name:} command mixes old command params (run_mode or chdir) with new command params (invoke, working_dir, or path_args)"
    )]
    CannotMixOldAndNewCommandParams { name: String },
    #[error(r#"Cannot set invoke = "per-file" and path_args = "{path_args:}""#)]
    CannotInvokePerFileWithPathArgs { path_args: PathArgs },
    #[error(r#"Cannot set invoke = "per-dir" and path_args = "{path_args:}""#)]
    CannotInvokePerDirInRootWithPathArgs { path_args: PathArgs },
    #[error(r#"Cannot set invoke = "once" and working_dir = "dir""#)]
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

fn u8_or_seq_u8<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    struct U8OrVec(PhantomData<Vec<u8>>);

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
                    de::Unexpected::Signed(value as i64),
                    &"an integer from 0-255",
                ));
            }

            Ok(vec![value as u8])
        }

        fn visit_i16<E>(self, value: i16) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 || value > std::u8::MAX as i16 {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Signed(value as i64),
                    &"an integer from 0-255",
                ));
            }

            Ok(vec![value as u8])
        }

        fn visit_i32<E>(self, value: i32) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 || value > std::u8::MAX as i32 {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Signed(value as i64),
                    &"an integer from 0-255",
                ));
            }

            Ok(vec![value as u8])
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 || value > std::u8::MAX as i64 {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Signed(value as i64),
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
            if value > std::u8::MAX as u16 {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Unsigned(value as u64),
                    &"an integer from 0-255",
                ));
            }

            Ok(vec![value as u8])
        }

        fn visit_u32<E>(self, value: u32) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value > std::u8::MAX as u32 {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Unsigned(value as u64),
                    &"an integer from 0-255",
                ));
            }

            Ok(vec![value as u8])
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value > std::u8::MAX as u64 {
                return Err(de::Error::invalid_type(
                    de::Unexpected::Unsigned(value as u64),
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
    struct WorkingDirOrSubRoots(PhantomData<Option<WorkingDir>>);

    impl<'de> de::Visitor<'de> for WorkingDirOrSubRoots {
        type Value = Option<WorkingDir>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str(r#"one of "root", "dir", or a sub_roots map"#)
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
            deserializer.deserialize_any(WorkingDirOrSubRoots(PhantomData))
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
            let mut kv_pairs: Vec<(&'de str, Vec<&'de str>)> = vec![];
            while let Some((k, v)) = map.next_entry::<&str, Vec<&str>>()? {
                if k != "sub_roots" {
                    return Err(<A::Error as de::Error>::invalid_value(
                        de::Unexpected::Str(k),
                        &r#"the only valid key for a working_dir map is "sub_roots""#,
                    ));
                }
                if v.is_empty() {
                    return Err(<A::Error as de::Error>::invalid_value(
                        de::Unexpected::Seq,
                        &r#"the "sub_roots" key cannot be empty"#,
                    ));
                }
                kv_pairs.push((k, v));
            }

            if kv_pairs.is_empty() {
                return Err(<A::Error as de::Error>::invalid_value(
                    de::Unexpected::Map,
                    &r#"the "working_dir" cannot be an empty map"#,
                ));
            }

            if kv_pairs.len() > 1 {
                return Err(<A::Error as de::Error>::invalid_value(
                    de::Unexpected::Map,
                    &r#"the "working_dir" map must contain one key, "sub_roots""#,
                ));
            }

            Ok(Some(WorkingDir::SubRoots(
                kv_pairs[0].1.iter().map(PathBuf::from).collect::<Vec<_>>(),
            )))
        }
    }

    deserializer.deserialize_any(WorkingDirOrSubRoots(PhantomData))
}

impl Config {
    pub fn new(file: PathBuf) -> Result<Config> {
        match fs::read(&file) {
            Err(e) => Err(ConfigError::FileCannotBeRead { file, error: e }.into()),
            Ok(bytes) => Ok(toml::from_slice(&bytes)?),
        }
    }

    pub fn into_tidy_commands(
        self,
        project_root: &Path,
        command: Option<&str>,
    ) -> Result<Vec<command::Command>> {
        self.into_commands(project_root, command, command::CommandType::Tidy)
    }

    pub fn into_lint_commands(
        self,
        project_root: &Path,
        command: Option<&str>,
    ) -> Result<Vec<command::Command>> {
        self.into_commands(project_root, command, command::CommandType::Lint)
    }

    fn into_commands(
        self,
        project_root: &Path,
        command: Option<&str>,
        typ: command::CommandType,
    ) -> Result<Vec<command::Command>> {
        let mut commands: Vec<command::Command> = vec![];
        for (name, c) in self.commands.into_iter() {
            if let Some(c) = command {
                if name != c {
                    continue;
                }
            }
            if c.typ != typ && c.typ != command::CommandType::Both {
                continue;
            }

            commands.push(c.into_command(project_root, name)?);
        }

        Ok(commands)
    }
}

impl Command {
    fn into_command(self, project_root: &Path, name: String) -> Result<command::Command> {
        let (invoke, working_dir, path_args) = Self::invoke_args(
            &name,
            self.run_mode,
            self.chdir,
            self.invoke,
            self.working_dir,
            self.path_args,
        )?;

        let n = command::Command::new(command::CommandParams {
            project_root: project_root.to_owned(),
            name,
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
        })?;
        Ok(n)
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
        match (run_mode, chdir) {
            (Some(OldRunMode::Files), Some(false)) => {
                return Ok((Invoke::PerFile, WorkingDir::Root, PathArgs::File))
            }
            (Some(OldRunMode::Files), Some(true)) => {
                return Ok((Invoke::PerFile, WorkingDir::Dir, PathArgs::File))
            }
            (Some(OldRunMode::Dirs), Some(false)) => {
                return Ok((Invoke::PerDir, WorkingDir::Root, PathArgs::File))
            }
            (Some(OldRunMode::Dirs), Some(true)) => {
                return Ok((Invoke::PerDir, WorkingDir::Dir, PathArgs::None))
            }
            (Some(OldRunMode::Root), Some(false)) => {
                return Ok((Invoke::PerDir, WorkingDir::Root, PathArgs::Dot))
            }
            (Some(OldRunMode::Root), Some(true)) => {
                return Ok((Invoke::PerDir, WorkingDir::Dir, PathArgs::None))
            }
            _ => (),
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
            (Invoke::PerDir, &WorkingDir::Root | &WorkingDir::SubRoots(_), path_args) => {
                if path_args != PathArgs::Dir && path_args != PathArgs::AbsoluteDir {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serial_test::parallel;

    #[test]
    #[parallel]
    fn command_order_is_preserved1() -> Result<()> {
        let toml_text = r#"
            [commands.rustfmt]
            type    = "both"
            include = "**/*.rs"
            cmd     = [ "rustfmt", "--skip-children", "--unstable-features" ]
            lint_flags = "--check"
            ok_exit_codes = 0
            lint_failure_exit_codes = 1

            [commands.clippy]
            type     = "lint"
            include  = "**/*.rs"
            run_mode = "root"
            chdir    = true
            cmd      = "$PRECIOUS_ROOT/dev/bin/force-clippy.sh"
            ok_exit_codes = 0
            lint_failure_exit_codes = 101

            [commands.omegasort-gitignore]
            type = "both"
            include = "**/.gitignore"
            cmd = [ "omegasort", "--sort=path" ]
            lint_flags = "--check"
            tidy_flags = "--in-place"
            ok_exit_codes = 0
            lint_failure_exit_codes = 1
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
            run_mode = "root"
            chdir    = true
            cmd      = "$PRECIOUS_ROOT/dev/bin/force-clippy.sh"
            ok_exit_codes = 0
            lint_failure_exit_codes = 101

            [commands.rustfmt]
            type    = "both"
            include = "**/*.rs"
            cmd     = [ "rustfmt", "--skip-children", "--unstable-features" ]
            lint_flags = "--check"
            ok_exit_codes = 0
            lint_failure_exit_codes = 1

            [commands.omegasort-gitignore]
            type = "both"
            include = "**/.gitignore"
            cmd = [ "omegasort", "--sort=path" ]
            lint_flags = "--check"
            tidy_flags = "--in-place"
            ok_exit_codes = 0
            lint_failure_exit_codes = 1
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
            lint_flags = "--check"
            tidy_flags = "--in-place"
            ok_exit_codes = 0
            lint_failure_exit_codes = 1

            [commands.clippy]
            type     = "lint"
            include  = "**/*.rs"
            run_mode = "root"
            chdir    = true
            cmd      = "$PRECIOUS_ROOT/dev/bin/force-clippy.sh"
            ok_exit_codes = 0
            lint_failure_exit_codes = 101

            [commands.rustfmt]
            type    = "both"
            include = "**/*.rs"
            cmd     = [ "rustfmt", "--skip-children", "--unstable-features" ]
            lint_flags = "--check"
            ok_exit_codes = 0
            lint_failure_exit_codes = 1
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
}
