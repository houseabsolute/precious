use crate::filter;
use failure::Error;
use std::fs;
use std::path::PathBuf;
use toml;

#[derive(Debug)]
pub struct Server {
    port: u16,
}

#[derive(Debug)]
pub struct Command {
    lint_flag: String,
    path_flag: String,
    ok_exit_codes: Vec<u8>,
    lint_failure_exit_codes: Vec<u8>,
    expect_stderr: bool,
}

#[derive(Debug)]
enum FilterImplementation {
    C(Command),
    S(Server),
}

#[derive(Debug)]
pub struct FilterCore {
    name: String,
    typ: filter::FilterType,
    ignore: Vec<String>,
    include: Vec<String>,
    exclude: Vec<String>,
    on_dir: bool,
    cmd: Vec<String>,
    args: Vec<String>,
}

#[derive(Debug)]
pub struct Filter {
    core: FilterCore,
    implementation: FilterImplementation,
}

#[derive(Debug)]
pub struct Config {
    pub ignore_from: Vec<String>,
    pub exclude: Vec<String>,
    filters: Vec<Filter>,
}

#[derive(Debug, Fail)]
pub enum ConfigError {
    #[fail(display = "File at {} does not contain a TOML table", file)]
    FileIsNotTOML { file: String },

    #[fail(
        display = "Found an invalid value for an array value of the {} key. Expected an array of {} but this is a {}.",
        key, want, got
    )]
    InvalidTOMLArrayValue {
        key: &'static str,
        want: &'static str,
        got: String,
    },

    #[fail(
        display = "Found an invalid value for the {} key. Expected {} but this is a {}.",
        key, want, got
    )]
    InvalidTOMLValue {
        key: &'static str,
        want: &'static str,
        got: String,
    },

    #[fail(display = "You must define a {} for the {} entry.", key, name)]
    MissingTOMLKey { key: &'static str, name: String },

    #[fail(display = "Expected a value from {} to {} but got {}.", min, max, val)]
    IntegerConversionError { min: i64, max: i64, val: i64 },

    #[fail(display = "Servers are not yet implemented")]
    ServersAreNotYetImplemented,
}

impl Config {
    pub fn new_from_file(path: PathBuf) -> Result<Config, Error> {
        let bytes = fs::read(path.clone())?;
        let raw = String::from_utf8_lossy(&bytes);
        let root: toml::Value = toml::from_str(&raw)?;
        if !root.is_table() {
            return Err(ConfigError::FileIsNotTOML {
                file: path.to_string_lossy().to_string(),
            })?;
        }

        let table = root.as_table().unwrap();

        Ok(Config {
            ignore_from: Self::toml_string_vec(table, "ignore_from")?,
            exclude: Self::toml_string_vec(table, "exclude")?,
            filters: Self::toml_filters(table)?,
        })
    }

    fn toml_filters(table: &toml::value::Table) -> Result<Vec<Filter>, Error> {
        let mut filters: Vec<Filter> = vec![];
        let mut c = Self::toml_filters_by_key("commands", table, Self::toml_to_command)?;
        filters.append(&mut c);
        let mut s = Self::toml_filters_by_key("servers", table, Self::toml_to_server)?;
        filters.append(&mut s);

        Ok(filters)
    }

    fn toml_filters_by_key(
        key: &'static str,
        table: &toml::value::Table,
        constructor: fn(String, &toml::value::Table) -> Result<Filter, Error>,
    ) -> Result<Vec<Filter>, Error> {
        let mut constructed: Vec<Filter> = vec![];
        if table.contains_key(key) {
            let filters = table.get(key).unwrap();
            if filters.is_table() {
                for (name, f) in filters.as_table().unwrap() {
                    if f.is_table() {
                        constructed.push(constructor(name.to_string(), f.as_table().unwrap())?)
                    } else {
                        return Err(ConfigError::InvalidTOMLArrayValue {
                            key,
                            want: "a table",
                            got: f.type_str().to_string(),
                        })?;
                    }
                }
            } else {
                return Err(ConfigError::InvalidTOMLValue {
                    key,
                    want: "an array of tables",
                    got: filters.type_str().to_string(),
                })?;
            }
        }

        Ok(constructed)
    }

    fn toml_to_command(name: String, table: &toml::value::Table) -> Result<Filter, Error> {
        let lint_flag = Self::toml_string(table, "lint_flag")?;
        let path_flag = Self::toml_string(table, "path_flag")?;
        let ok_exit_codes = Self::toml_u8_vec(table, "ok_exit_codes")?;
        let lint_failure_exit_codes = Self::toml_u8_vec(table, "lint_failure_exit_codes")?;
        let expect_stderr = Self::toml_bool(table, "expect_stderr")?;

        if ok_exit_codes.is_empty() {
            return Err(ConfigError::MissingTOMLKey {
                key: "ok_exit_codes",
                name,
            })?;
        }

        let toml_typ = Self::toml_string(table, "type")?;
        if toml_typ != "tidy" && lint_failure_exit_codes.is_empty() {
            return Err(ConfigError::MissingTOMLKey {
                key: "lint_failure_exit_codes",
                name,
            })?;
        }

        Ok(Filter {
            core: Self::toml_to_filter_core(name, table)?,
            implementation: FilterImplementation::C(Command {
                lint_flag,
                path_flag,
                ok_exit_codes,
                lint_failure_exit_codes,
                expect_stderr,
            }),
        })
    }

    fn toml_to_server(name: String, table: &toml::value::Table) -> Result<Filter, Error> {
        let port = Self::toml_u16(table, "port")?;

        Ok(Filter {
            core: Self::toml_to_filter_core(name, table)?,
            implementation: FilterImplementation::S(Server { port }),
        })
    }

    fn toml_to_filter_core(name: String, table: &toml::value::Table) -> Result<FilterCore, Error> {
        let toml_typ = Self::toml_string(table, "type")?;
        let typ = match toml_typ.as_str() {
            "lint" => filter::FilterType::Lint,
            "tidy" => filter::FilterType::Tidy,
            "both" => filter::FilterType::Both,
            _ => {
                return Err(ConfigError::InvalidTOMLValue {
                    key: "type",
                    want: "one of \"lint\", \"tidy\", or \"both\"",
                    got: toml_typ.as_str().to_string(),
                })?;
            }
        };
        let ignore = Self::toml_string_vec(table, "ignore")?;
        let include = Self::toml_string_vec(table, "include")?;
        let exclude = Self::toml_string_vec(table, "exclude")?;
        let on_dir = Self::toml_bool(table, "on_dir")?;
        let cmd = Self::toml_string_vec(table, "cmd")?;
        let args = Self::toml_string_vec(table, "args")?;

        if include.is_empty() {
            return Err(ConfigError::MissingTOMLKey {
                key: "include",
                name,
            })?;
        }

        if cmd.is_empty() {
            return Err(ConfigError::MissingTOMLKey { key: "cmd", name })?;
        }

        Ok(FilterCore {
            name,
            typ,
            ignore,
            include,
            exclude,
            on_dir,
            cmd,
            args,
        })
    }

    fn toml_string_vec(
        table: &toml::value::Table,
        key: &'static str,
    ) -> Result<Vec<String>, Error> {
        if !table.contains_key(key) {
            return Ok(Vec::new());
        }

        let val = table.get(key).unwrap();
        if val.is_str() {
            return Ok(vec![val.as_str().unwrap().to_string()]);
        } else if val.is_array() {
            let mut i: Vec<String> = vec![];
            for v in val.as_array().unwrap() {
                if v.is_str() {
                    i.push(v.as_str().unwrap().to_string());
                } else {
                    return Err(ConfigError::InvalidTOMLArrayValue {
                        key,
                        want: "a string",
                        got: v.type_str().to_string(),
                    })?;
                }
            }
            return Ok(i);
        }

        Err(ConfigError::InvalidTOMLValue {
            key,
            want: "a string or an array of strings",
            got: val.type_str().to_string(),
        })?
    }

    fn toml_string(table: &toml::value::Table, key: &'static str) -> Result<String, Error> {
        if !table.contains_key(key) {
            return Ok(String::from(""));
        }

        let val = table.get(key).unwrap();
        if val.is_str() {
            return Ok(val.as_str().unwrap().to_string());
        }

        Err(ConfigError::InvalidTOMLValue {
            key,
            want: "a string",
            got: val.type_str().to_string(),
        })?
    }

    fn toml_bool(table: &toml::value::Table, key: &'static str) -> Result<bool, Error> {
        if !table.contains_key(key) {
            return Ok(false);
        }

        let val = table.get(key).unwrap();
        if val.is_bool() {
            return Ok(val.as_bool().unwrap());
        }

        Err(ConfigError::InvalidTOMLValue {
            key,
            want: "a bool",
            got: val.type_str().to_string(),
        })?
    }

    fn toml_u8_vec(table: &toml::value::Table, key: &'static str) -> Result<Vec<u8>, Error> {
        if !table.contains_key(key) {
            return Ok(Vec::new());
        }

        let val = table.get(key).unwrap();
        if val.is_integer() {
            return Ok(vec![Self::toml_int_to_u8(val.as_integer().unwrap())?]);
        } else if val.is_array() {
            let mut i: Vec<u8> = vec![];
            for v in val.as_array().unwrap() {
                if v.is_integer() {
                    i.push(Self::toml_int_to_u8(v.as_integer().unwrap())?);
                } else {
                    return Err(ConfigError::InvalidTOMLArrayValue {
                        key,
                        want: "value from 0-255",
                        got: v.type_str().to_string(),
                    })?;
                }
            }
            return Ok(i);
        }

        Err(ConfigError::InvalidTOMLValue {
            key,
            want: "an integer of array of integers",
            got: val.type_str().to_string(),
        })?
    }

    fn toml_u16(table: &toml::value::Table, key: &'static str) -> Result<u16, Error> {
        if !table.contains_key(key) {
            return Ok(0);
        }

        let val = table.get(key).unwrap();
        if val.is_integer() {
            return Ok(Self::toml_int_to_u16(val.as_integer().unwrap())?);
        }

        Err(ConfigError::InvalidTOMLValue {
            key,
            want: "an integer from 0-65535",
            got: val.type_str().to_string(),
        })?
    }

    fn toml_int_to_u8(i: i64) -> Result<u8, Error> {
        if i > i64::from(std::u8::MAX) {
            return Err(ConfigError::IntegerConversionError {
                min: 0 as i64,
                max: i64::from(std::u8::MAX),
                val: i,
            })?;
        }

        Ok(i as u8)
    }

    fn toml_int_to_u16(i: i64) -> Result<u16, Error> {
        if i > i64::from(std::u16::MAX) {
            return Err(ConfigError::IntegerConversionError {
                min: 0 as i64,
                max: i64::from(std::u16::MAX),
                val: i,
            })?;
        }

        Ok(i as u16)
    }

    pub fn tidy_filters(&self, root: &PathBuf) -> Result<Vec<filter::Filter>, Error> {
        let mut tidiers: Vec<filter::Filter> = vec![];
        for f in self.filters.iter() {
            if let filter::FilterType::Lint = f.core.typ {
                continue;
            }

            tidiers.push(self.make_filter(root, &f)?);
        }

        Ok(tidiers)
    }

    pub fn lint_filters(&self, root: &PathBuf) -> Result<Vec<filter::Filter>, Error> {
        let mut linters: Vec<filter::Filter> = vec![];
        for f in self.filters.iter() {
            if let filter::FilterType::Tidy = f.core.typ {
                continue;
            }

            linters.push(self.make_filter(root, &f)?);
        }

        Ok(linters)
    }

    fn make_filter(&self, root: &PathBuf, filter: &Filter) -> Result<filter::Filter, Error> {
        match &filter.implementation {
            FilterImplementation::C(c) => {
                let n = filter::Command::build(
                    root,
                    filter.core.name.clone(),
                    filter.core.typ.clone(),
                    filter.core.include.clone(),
                    filter.core.ignore.clone(),
                    filter.core.exclude.clone(),
                    filter.core.on_dir,
                    filter.core.cmd.clone(),
                    c.lint_flag.clone(),
                    c.path_flag.clone(),
                    c.ok_exit_codes.clone(),
                    c.lint_failure_exit_codes.clone(),
                    c.expect_stderr,
                )?;
                Ok(n)
            }
            FilterImplementation::S(_) => Err(ConfigError::ServersAreNotYetImplemented)?,
        }
    }
}
