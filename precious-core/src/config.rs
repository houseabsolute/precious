use crate::filter;
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

#[derive(Debug, Deserialize)]
pub struct FilterCore {
    #[serde(rename = "type")]
    typ: filter::FilterType,
    #[serde(deserialize_with = "string_or_seq_string")]
    include: Vec<String>,
    #[serde(default)]
    #[serde(deserialize_with = "string_or_seq_string")]
    exclude: Vec<String>,
    #[serde(default = "default_run_mode")]
    run_mode: filter::RunMode,
    #[serde(deserialize_with = "string_or_seq_string")]
    cmd: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct Command {
    #[serde(flatten)]
    core: FilterCore,
    #[serde(default)]
    chdir: bool,
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
}

fn default_run_mode() -> filter::RunMode {
    filter::RunMode::Files
}

fn empty_string() -> String {
    String::new()
}

#[derive(Debug, Deserialize)]
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
            formatter.write_str("integer or list of integers")
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

impl Config {
    pub fn new(file: PathBuf) -> Result<Config> {
        match fs::read(&file) {
            Err(e) => Err(ConfigError::FileCannotBeRead { file, error: e }.into()),
            Ok(bytes) => Ok(toml::from_slice(&bytes)?),
        }
    }

    pub fn tidy_filters(&self, root: &Path, command: Option<&str>) -> Result<Vec<filter::Filter>> {
        self.filters(root, command, filter::FilterType::Tidy)
    }

    pub fn lint_filters(&self, root: &Path, command: Option<&str>) -> Result<Vec<filter::Filter>> {
        self.filters(root, command, filter::FilterType::Lint)
    }

    fn filters(
        &self,
        root: &Path,
        command: Option<&str>,
        typ: filter::FilterType,
    ) -> Result<Vec<filter::Filter>> {
        let mut filters: Vec<filter::Filter> = vec![];
        for (name, c) in self.commands.iter() {
            if let Some(c) = command {
                if name != c {
                    continue;
                }
            }
            if c.core.typ != typ && c.core.typ != filter::FilterType::Both {
                continue;
            }

            filters.push(self.make_command(root, name, c)?);
        }

        Ok(filters)
    }

    fn make_command(&self, root: &Path, name: &str, command: &Command) -> Result<filter::Filter> {
        let n = filter::Command::build(filter::CommandParams {
            root: root.to_owned(),
            name: name.to_owned(),
            typ: command.core.typ,
            include: command.core.include.clone(),
            exclude: command.core.exclude.clone(),
            run_mode: command.core.run_mode,
            chdir: command.chdir,
            cmd: command.core.cmd.clone(),
            env: command.core.env.clone(),
            lint_flags: command.lint_flags.clone(),
            tidy_flags: command.tidy_flags.clone(),
            path_flag: command.path_flag.clone(),
            ok_exit_codes: command.ok_exit_codes.clone(),
            lint_failure_exit_codes: command.lint_failure_exit_codes.clone(),
            expect_stderr: command.expect_stderr,
        })?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn filter_order_is_preserved1() -> Result<()> {
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
            expect_stderr = true

            [commands.omegasort-gitignore]
            type = "both"
            include = "**/.gitignore"
            cmd = [ "omegasort", "--sort=path" ]
            lint_flags = "--check"
            tidy_flags = "--in-place"
            ok_exit_codes = 0
            lint_failure_exit_codes = 1
            expect_stderr = true
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
    fn filter_order_is_preserved2() -> Result<()> {
        let toml_text = r#"
            [commands.clippy]
            type     = "lint"
            include  = "**/*.rs"
            run_mode = "root"
            chdir    = true
            cmd      = "$PRECIOUS_ROOT/dev/bin/force-clippy.sh"
            ok_exit_codes = 0
            lint_failure_exit_codes = 101
            expect_stderr = true

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
            expect_stderr = true
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
    fn filter_order_is_preserved3() -> Result<()> {
        let toml_text = r#"
            [commands.omegasort-gitignore]
            type = "both"
            include = "**/.gitignore"
            cmd = [ "omegasort", "--sort=path" ]
            lint_flags = "--check"
            tidy_flags = "--in-place"
            ok_exit_codes = 0
            lint_failure_exit_codes = 1
            expect_stderr = true

            [commands.clippy]
            type     = "lint"
            include  = "**/*.rs"
            run_mode = "root"
            chdir    = true
            cmd      = "$PRECIOUS_ROOT/dev/bin/force-clippy.sh"
            ok_exit_codes = 0
            lint_failure_exit_codes = 101
            expect_stderr = true

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
