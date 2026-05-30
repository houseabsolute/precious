use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use clap::ValueEnum;
use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;
use log::debug;
use std::{
    collections::{HashMap, HashSet},
    fs::{create_dir_all, File},
    io::Write,
};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub(crate) struct Init {
    pub(crate) excludes: &'static [&'static str],
    pub(crate) shared: &'static [(&'static str, &'static [&'static str])],
    pub(crate) commands: &'static [(&'static str, &'static str)],
    pub(crate) extra_files: Vec<ConfigInitFile>,
    pub(crate) tool_urls: &'static [&'static str],
}

pub(crate) struct ConfigInitFile {
    pub(crate) path: Utf8PathBuf,
    pub(crate) content: &'static str,
    pub(crate) is_executable: bool,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, ValueEnum)]
pub(crate) enum InitComponent {
    Gitignore,
    Go,
    Markdown,
    Perl,
    Python,
    Ruby,
    Rust,
    Shell,
    Toml,
    Typescript,
    Yaml,
}

#[derive(Debug, Error)]
enum ConfigInitError {
    #[error("A file already exists at the given path: {path}")]
    FileExists { path: Utf8PathBuf },
}

const GO_COMMANDS: [(&str, &str); 3] = [
    (
        "golangci-lint",
        r#"
type = "both"
include = "**/*.go"
# For large projects with many packages, you may want to set
# `invoke.per-dir-or-once = 7`. You can experiment with different numbers of
# directories to see what works best for your project.
invoke = "once"
path-args = "dir"
# The `--allow-parallel-runners` flag is only relevant when `invoke` is not
# set to `once`.
#
# Allowing golangci-lint to run in parallel reduces the effectiveness of its
# cache when it has to parse the same code repeatedly. Depending on the
# structure of your repo, you may get a better result by using the
# `--allow-serial-runners` flag instead. However, if `invoke` is not `once`,
# you must use one of these, as by default golangci-lint can simply timeout
# and fail when multiple instances of the executable are invoked at the same
# time for the same project.
#
# Alternatively, for smaller projects you can set `invoke = "once"` and
# `path-args = "none"` to run it once for all code in the project, in which
# case you can remove this flag.
cmd = ["golangci-lint", "run", "-c", "--allow-parallel-runners"]
tidy-flags = "--fix"
env = { "FAIL_ON_WARNINGS" = "1" }
ok-exit-codes = [0]
lint-failure-exit-codes = [1]
"#,
    ),
    (
        "tidy go files",
        r#"
type = "tidy"
include = "**/*.go"
cmd = ["gofumpt", "-w"]
ok-exit-codes = [0]
"#,
    ),
    (
        "check-go-mod",
        r#"
type = "lint"
include = "**/*.go"
invoke = "once"
path-args = "none"
cmd = ["$PRECIOUS_ROOT/dev/bin/check-go-mod.sh"]
ok-exit-codes = [0]
lint-failure-exit-codes = [1]
"#,
    ),
];

const GOLANGCI_YML: &str = "
linters:
  disable-all: true
  enable:
    - bidichk
    - bodyclose
    - decorder
    - dupl
    - dupword
    - durationcheck
    - errcheck
    - errchkjson
    - errname
    - errorlint
    - exhaustive
    - exportloopref
    - gci
    - gocheckcompilerdirectives
    - goconst
    - gocritic
    - godot
    - gofumpt
    - gomnd
    - gosimple
    - govet
    - importas
    - ineffassign
    - misspell
    - nolintlint
    - lll
    - mirror
    - nonamedreturns
    - paralleltest
    - revive
    - rowserrcheck
    - sloglint
    - sqlclosecheck
    - staticcheck
    - tenv
    - testifylint
    - thelper
    - typecheck
    - unconvert
    - unused
    - usestdlibvars
    - wastedassign
    - whitespace
    - wrapcheck
  fast: false

linters-settings:
  errcheck:
    check-type-assertions: true
  gci:
    sections:
      - standard
      - default
  govet:
    check-shadowing: true
  importas:
    no-extra-aliases: true
";

const CHECK_GO_MOD: &str = r#"
#!/bin/bash

set -e

ROOT=$(git rev-parse --show-toplevel)

if [ ! -f "$ROOT/go.sum" ]; then
    exit 0
fi

BEFORE_MOD=$(md5sum "$ROOT/go.mod")
BEFORE_SUM=$(md5sum "$ROOT/go.sum")

OUTPUT=$(go mod tidy -v 2>&1)

AFTER_MOD=$(md5sum "$ROOT/go.mod")
AFTER_SUM=$(md5sum "$ROOT/go.sum")

red=$'\e[1;31m'
end=$'\e[0m'

if [ "$BEFORE_MOD" != "$AFTER_MOD" ]; then
    printf "${red}Running go mod tidy changed the contents of go.mod${end}\n"
    git diff "$ROOT/go.mod"
    changed=1
fi

if [ "$BEFORE_SUM" != "$AFTER_SUM" ]; then
    printf "${red}Running go mod tidy changed the contents of go.sum${end}\n"
    git diff "$ROOT/go.sum"
    changed=1
fi

if [ -n "$changed" ]; then
    if [ -n "$OUTPUT" ]; then
        printf "\nOutput from running go mod tidy -v:\n${OUTPUT}\n"
    else
        printf "\nThere was no output from running go mod tidy -v\n\n"
    fi

    exit 1
fi

exit 0
"#;

pub(crate) fn go_init() -> Init {
    Init {
        excludes: &["vendor/**/*"],
        shared: &[],
        commands: &GO_COMMANDS,
        extra_files: vec![
            ConfigInitFile {
                path: Utf8PathBuf::from("dev/bin/check-go-mod.sh"),
                content: CHECK_GO_MOD,
                is_executable: true,
            },
            ConfigInitFile {
                path: Utf8PathBuf::from(".golangci.yml"),
                content: GOLANGCI_YML,
                is_executable: false,
            },
        ],
        tool_urls: &[
            "https://golangci-lint.run/",
            "https://github.com/mvdan/gofumpt",
        ],
    }
}

const PERL_SHARED: [(&str, &[&str]); 2] = [
    ("perl-code", &["**/*.{pl,pm,t,psgi}"]),
    ("perl-docs", &["**/*.{pl,pm,pod}"]),
];

const PERL_COMMANDS: [(&str, &str); 5] = [
    (
        "perlimports",
        r#"
type = "both"
shared-include = "perl-code"
cmd = ["perlimports"]
lint-flags = ["--lint"]
tidy-flags = ["-i"]
ok-exit-codes = 0
expect-stderr = true
"#,
    ),
    (
        "perlcritic",
        r#"
type = "lint"
shared-include = "perl-code"
cmd = ["perlcritic", "--profile=$PRECIOUS_ROOT/perlcriticrc"]
ok-exit-codes = 0
lint-failure-exit-codes = 2
"#,
    ),
    (
        "perltidy",
        r#"
type = "both"
shared-include = "perl-code"
cmd = ["perltidy", "--profile=$PRECIOUS_ROOT/perltidyrc"]
lint-flags = ["--assert-tidy", "--no-standard-output", "--outfile=/dev/null"]
tidy-flags = ["--backup-and-modify-in-place", "--backup-file-extension=/"]
ok-exit-codes = 0
lint-failure-exit-codes = 2
ignore-stderr = "Begin Error Output Stream"
"#,
    ),
    (
        "podchecker",
        r#"
type = "lint"
shared-include = "perl-docs"
cmd = ["podchecker", "--warnings", "--warnings"]
ok-exit-codes = [0, 2]
lint-failure-exit-codes = 1
ignore-stderr = [".+ pod syntax OK", ".+ does not contain any pod commands"]
"#,
    ),
    (
        "podtidy",
        r#"
type = "tidy"
shared-include = "perl-docs"
cmd = ["podtidy", "--columns", "100", "--inplace", "--nobackup"]
ok-exit-codes = 0
lint-failure-exit-codes = 1
"#,
    ),
];

pub(crate) fn perl_init() -> Init {
    Init {
        excludes: &[".build/**", "blib/**"],
        shared: &PERL_SHARED,
        commands: &PERL_COMMANDS,
        extra_files: vec![],
        tool_urls: &[
            "https://metacpan.org/dist/Perl-Critic",
            "https://metacpan.org/dist/Perl-Tidy",
            "https://metacpan.org/dist/App-perlimports",
            "https://metacpan.org/dist/Pod-Checker",
            "https://metacpan.org/dist/Pod-Tidy",
        ],
    }
}

const PYTHON_COMMANDS: [(&str, &str); 3] = [
    (
        "ruff-check",
        r#"
type = "both"
include = "**/*.py"
cmd = ["ruff", "check"]
tidy-flags = "--fix"
ok-exit-codes = 0
lint-failure-exit-codes = 1
"#,
    ),
    (
        "ruff-format",
        r#"
type = "both"
include = "**/*.py"
cmd = ["ruff", "format"]
lint-flags = "--check"
ok-exit-codes = 0
lint-failure-exit-codes = 1
"#,
    ),
    (
        "mypy",
        r#"
type = "lint"
include = "**/*.py"
invoke = "once"
path-args = "none"
cmd = ["mypy", "."]
ok-exit-codes = 0
lint-failure-exit-codes = 1
"#,
    ),
];

pub(crate) fn python_init() -> Init {
    Init {
        excludes: &[".venv/**", "**/__pycache__/**", "dist/**", "build/**"],
        shared: &[],
        commands: &PYTHON_COMMANDS,
        extra_files: vec![],
        tool_urls: &[
            "https://docs.astral.sh/ruff/",
            "https://mypy.readthedocs.io/",
        ],
    }
}

const TYPESCRIPT_SHARED: [(&str, &[&str]); 1] = [("ts-and-js", &["**/*.{ts,tsx,js,jsx}"])];

const TYPESCRIPT_COMMANDS: [(&str, &str); 2] = [
    (
        "eslint",
        r#"
type = "both"
shared-include = "ts-and-js"
cmd = ["./node_modules/.bin/eslint"]
tidy-flags = "--fix"
ok-exit-codes = 0
lint-failure-exit-codes = 1
"#,
    ),
    (
        "prettier-typescript",
        r#"
type = "both"
shared-include = "ts-and-js"
cmd = ["./node_modules/.bin/prettier"]
lint-flags = "--check"
tidy-flags = "--write"
ok-exit-codes = 0
lint-failure-exit-codes = 1
ignore-stderr = ["Code style issues"]
"#,
    ),
];

pub(crate) fn typescript_init() -> Init {
    Init {
        excludes: &["node_modules/**", "dist/**", "build/**"],
        shared: &TYPESCRIPT_SHARED,
        commands: &TYPESCRIPT_COMMANDS,
        extra_files: vec![],
        tool_urls: &["https://eslint.org/", "https://prettier.io/"],
    }
}

const RUBY_COMMANDS: [(&str, &str); 1] = [(
    "rubocop",
    r#"
type = "both"
include = "**/*.rb"
# Common rubocop extensions — add to your Gemfile as needed:
#   rubocop-performance, rubocop-rspec, rubocop-rails
cmd = ["rubocop"]
tidy-flags = "-A"
ok-exit-codes = 0
lint-failure-exit-codes = 1
"#,
)];

pub(crate) fn ruby_init() -> Init {
    Init {
        excludes: &[".bundle/**", "vendor/bundle/**"],
        shared: &[],
        commands: &RUBY_COMMANDS,
        extra_files: vec![],
        tool_urls: &["https://rubocop.org/"],
    }
}

const RUST_COMMANDS: [(&str, &str); 2] = [
    (
        "rustfmt",
        r#"
type = "both"
include = "**/*.rs"
cmd = ["rustfmt", "--edition", "2021"]
lint-flags = "--check"
ok-exit-codes = 0
lint-failure-exit-codes = 1
"#,
    ),
    (
        "clippy",
        r#"
type = "lint"
include = "**/*.rs"
invoke = "once"
path-args = "none"
cmd = [
    "cargo",
    "clippy",
    "--locked",
    "--all-targets",
    "--all-features",
    "--workspace",
    "--",
    "-D",
    "clippy::all",
]
ok-exit-codes = 0
lint-failure-exit-codes = 101
ignore-stderr = ["Checking.+precious", "Finished.+dev", "could not compile"]
"#,
    ),
];

pub(crate) fn rust_init() -> Init {
    Init {
        excludes: &["target"],
        shared: &[],
        commands: &RUST_COMMANDS,
        extra_files: vec![],
        tool_urls: &["https://doc.rust-lang.org/clippy/"],
    }
}

const SHELL_COMMANDS: [(&str, &str); 2] = [
    (
        "shellcheck",
        r#"
type = "lint"
include = "**/*.sh"
cmd = "shellcheck"
ok_exit_codes = 0
lint_failure_exit_codes = 1
"#,
    ),
    (
        "shfmt",
        r#"
type = "both"
include = "**/*.sh"
cmd = ["shfmt", "--simplify", "--indent", "4"]
lint_flags = "--diff"
tidy_flags = "--write"
ok_exit_codes = 0
lint_failure_exit_codes = 1
"#,
    ),
];

pub(crate) fn shell_init() -> Init {
    Init {
        excludes: &["target"],
        shared: &[],
        commands: &SHELL_COMMANDS,
        extra_files: vec![],
        tool_urls: &["https://www.shellcheck.net/", "https://github.com/mvdan/sh"],
    }
}

const GITIGNORE_COMMANDS: [(&str, &str); 1] = [(
    "omegasort-gitignore",
    r#"
type = "both"
include = "**/.gitignore"
cmd = ["omegasort", "--sort", "path", "--unique"]
lint-flags = "--check"
tidy-flags = "--in-place"
ok-exit-codes = 0
lint-failure-exit-codes = 1
ignore-stderr = ["The .+ file is not sorted", "The .+ file is not unique"]
"#,
)];

pub(crate) fn gitignore_init() -> Init {
    Init {
        excludes: &[],
        shared: &[],
        commands: &GITIGNORE_COMMANDS,
        extra_files: vec![],
        tool_urls: &["https://github.com/houseabsolute/omegasort"],
    }
}

const MARKDOWN_COMMANDS: [(&str, &str); 1] = [(
    "prettier-markdown",
    r#"
type = "both"
include = "**/*.md"
cmd = [
    "./node_modules/.bin/prettier",
    "--no-config",
    "--print-width",
    "100",
    "--prose-wrap",
    "always",
]
lint-flags = "--check"
tidy-flags = "--write"
ok-exit-codes = 0
lint-failure-exit-codes = 1
ignore-stderr = ["Code style issues"]
"#,
)];

pub(crate) fn markdown_init() -> Init {
    Init {
        excludes: &[],
        shared: &[],
        commands: &MARKDOWN_COMMANDS,
        extra_files: vec![],
        tool_urls: &["https://prettier.io/"],
    }
}

const TOML_COMMANDS: [(&str, &str); 1] = [(
    "taplo",
    r#"
type = "both"
include = "**/*.toml"
cmd = ["taplo", "format", "--option", "indent_string=    ", "--option", "column_width=100"]
lint_flags = "--check"
ok_exit_codes = 0
lint_failure_exit_codes = 1
ignore_stderr = "INFO taplo.+"
"#,
)];

pub(crate) fn toml_init() -> Init {
    Init {
        excludes: &[],
        shared: &[],
        commands: &TOML_COMMANDS,
        extra_files: vec![],
        tool_urls: &["https://taplo.tamasfe.dev/"],
    }
}

const YAML_COMMANDS: [(&str, &str); 1] = [(
    "prettier-yaml",
    r#"
type = "both"
include = "**/*.yml"
cmd = ["./node_modules/.bin/prettier", "--no-config"]
lint-flags = "--check"
tidy-flags = "--write"
ok-exit-codes = 0
lint-failure-exit-codes = 1
ignore-stderr = ["Code style issues"]
"#,
)];

pub(crate) fn yaml_init() -> Init {
    Init {
        excludes: &[],
        shared: &[],
        commands: &YAML_COMMANDS,
        extra_files: vec![],
        tool_urls: &["https://prettier.io/"],
    }
}

struct ConfigElements {
    excludes: HashSet<&'static str>,
    shared: IndexMap<&'static str, &'static [&'static str]>,
    commands: IndexMap<&'static str, &'static str>,
    extra_files: HashMap<Utf8PathBuf, ConfigInitFile>,
    tool_urls: IndexSet<&'static str>,
}

pub(crate) fn write_config_files(
    auto: bool,
    components: &[InitComponent],
    path: &Utf8Path,
    cwd: &Utf8Path,
) -> Result<()> {
    if cwd.join(path).exists() {
        return Err(ConfigInitError::FileExists {
            path: path.to_owned(),
        }
        .into());
    }

    let elements = config_elements(auto, components, cwd)
        .with_context(|| "Failed to generate config elements")?;

    let mut toml = excludes_toml(&elements.excludes);

    if !toml.is_empty() {
        toml.push_str("\n\n");
    }

    let shared = shared_toml(&elements.shared);
    if !shared.is_empty() {
        toml.push_str(&shared);
        toml.push('\n');
    }

    toml.push_str(&commands_toml(elements.commands));

    println!();
    println!("Writing {path}");

    let mut precious_toml =
        File::create(path).with_context(|| format!("Failed to create config file at {path}"))?;
    precious_toml
        .write_all(toml.as_bytes())
        .with_context(|| format!("Failed to write config data to {path}"))?;

    write_extra_files(&elements.extra_files)
        .with_context(|| format!("Failed to write extra files for {components:?} components"))?;

    println!();
    println!("The generated precious.toml requires the following tools to be installed:");
    for u in elements.tool_urls {
        println!("  {u}");
    }
    println!();

    Ok(())
}

fn config_elements(
    auto: bool,
    components: &[InitComponent],
    cwd: &Utf8Path,
) -> Result<ConfigElements> {
    let mut excludes: HashSet<&'static str> = HashSet::new();
    let mut shared: IndexMap<&'static str, &'static [&'static str]> = IndexMap::new();
    let mut commands = IndexMap::new();
    let mut extra_files: HashMap<Utf8PathBuf, ConfigInitFile> = HashMap::new();
    let mut tool_urls: IndexSet<&'static str> = IndexSet::new();

    for l in auto_or_component(auto, components, cwd)? {
        let init = match l {
            InitComponent::Gitignore => gitignore_init(),
            InitComponent::Go => go_init(),
            InitComponent::Markdown => markdown_init(),
            InitComponent::Perl => perl_init(),
            InitComponent::Python => python_init(),
            InitComponent::Ruby => ruby_init(),
            InitComponent::Rust => rust_init(),
            InitComponent::Shell => shell_init(),
            InitComponent::Toml => toml_init(),
            InitComponent::Typescript => typescript_init(),
            InitComponent::Yaml => yaml_init(),
        };
        excludes.extend(init.excludes);
        for (key, globs) in init.shared {
            shared.insert(key, globs);
        }
        for (name, c) in init.commands {
            commands.insert(*name, *c);
        }
        for f in init.extra_files {
            extra_files.insert(f.path.clone(), f);
        }
        tool_urls.extend(init.tool_urls);
    }

    Ok(ConfigElements {
        excludes,
        shared,
        commands,
        extra_files,
        tool_urls,
    })
}

fn auto_or_component(
    auto: bool,
    components: &[InitComponent],
    cwd: &Utf8Path,
) -> Result<Vec<InitComponent>> {
    if !auto {
        return Ok(components.to_vec());
    }

    let mut components: HashSet<InitComponent> = HashSet::new();
    debug!("Looking at all files under {cwd} to determine which components to include.");

    for result in ignore::WalkBuilder::new(cwd).hidden(false).build() {
        let entry = result.with_context(|| format!("Failed to walk directory {cwd}"))?;
        // The only time this is `None` is when the entry is for stdin, which
        // will never happen here.
        if !entry.file_type().unwrap().is_file() {
            continue;
        }

        let path = Utf8PathBuf::from_path_buf(entry.into_path()).map_err(|raw| {
            crate::paths::utf8::NonUtf8PathError {
                raw,
                source: crate::paths::utf8::NonUtf8Source::FilesystemWalk,
            }
        })?;

        if path.file_name() == Some(".gitignore") {
            components.insert(InitComponent::Gitignore);
            continue;
        }

        let component = match path.extension().unwrap_or_default() {
            "go" => InitComponent::Go,
            "md" => InitComponent::Markdown,
            "pl" | "pm" => InitComponent::Perl,
            "py" => InitComponent::Python,
            "rb" => InitComponent::Ruby,
            "rs" => InitComponent::Rust,
            "sh" => InitComponent::Shell,
            "toml" => InitComponent::Toml,
            "ts" | "tsx" | "js" | "jsx" => InitComponent::Typescript,
            "yml" | "yaml" => InitComponent::Yaml,
            _ => continue,
        };
        debug!("File {path} matches component {component:?}");
        components.insert(component);
    }

    Ok(components.into_iter().collect())
}

fn excludes_toml(excludes: &HashSet<&str>) -> String {
    if excludes.is_empty() {
        return String::new();
    }

    if excludes.len() == 1 {
        format!(r#"exclude = ["{}"]"#, excludes.iter().next().unwrap())
    } else {
        format!(
            "exclude = [\n{}\n]",
            excludes
                .iter()
                .sorted()
                .map(|e| format!(r#"    "{e}","#))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

fn shared_toml(shared: &IndexMap<&str, &[&str]>) -> String {
    use std::fmt::Write;

    if shared.is_empty() {
        return String::new();
    }
    let mut lines = String::from("[shared]\n");
    for (key, globs) in shared {
        let globs_str = globs
            .iter()
            .map(|g| format!(r#""{g}""#))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(lines, "{key} = [{globs_str}]").expect("this should never return an error");
    }
    lines
}

fn commands_toml(commands: IndexMap<&str, &str>) -> String {
    let mut command_strs: Vec<String> = Vec::new();

    for (name, c) in commands {
        let name_str = if name.contains(' ') {
            format!(r#""{name}""#)
        } else {
            name.to_string()
        };
        command_strs.push(format!("[commands.{name_str}]\n{}\n", c.trim()));
    }

    command_strs.join("\n")
}

fn write_extra_files(extra_files: &HashMap<Utf8PathBuf, ConfigInitFile>) -> Result<()> {
    if extra_files.is_empty() {
        return Ok(());
    }

    println!();
    println!("Generating support files");
    println!();

    let paths = extra_files.keys().sorted().collect::<Vec<_>>();

    for p in paths {
        print!("{p} ...");
        if p.exists() {
            println!("  already exists, skipping - delete this file if you want to regenerate it");
            continue;
        }
        println!(" generated");

        if let Some(parent) = p.parent() {
            create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {parent}"))?;
        }
        let mut file = File::create(p).with_context(|| format!("Failed to create file {p}"))?;
        let f = extra_files.get(p).unwrap();
        file.write_all(f.content.trim_start().as_bytes())
            .with_context(|| format!("Failed to write content to file {p}"))?;

        #[cfg(unix)]
        if f.is_executable {
            let mut perms = file
                .metadata()
                .with_context(|| format!("Failed to get metadata for {p}"))?
                .permissions();
            perms.set_mode(0o755);
            file.set_permissions(perms)
                .with_context(|| format!("Failed to set executable permissions on {p}"))?;
        }
    }

    Ok(())
}
