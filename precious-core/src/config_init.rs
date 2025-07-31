use anyhow::Result;
use clap::ValueEnum;
use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;
use log::debug;
use std::{
    collections::{HashMap, HashSet},
    env,
    fs::{create_dir_all, File},
    io::Write,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub(crate) struct Init {
    pub(crate) excludes: &'static [&'static str],
    pub(crate) commands: &'static [(&'static str, &'static str)],
    pub(crate) extra_files: Vec<ConfigInitFile>,
    pub(crate) tool_urls: &'static [&'static str],
}

pub(crate) struct ConfigInitFile {
    pub(crate) path: PathBuf,
    pub(crate) content: &'static str,
    pub(crate) is_executable: bool,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, ValueEnum)]
pub(crate) enum InitComponent {
    Go,
    Perl,
    Rust,
    Gitignore,
    Markdown,
    Shell,
    Toml,
    Yaml,
}

#[derive(Debug, Error)]
enum ConfigInitError {
    #[error("A file already exists at the given path: {path}")]
    FileExists { path: PathBuf },
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
        commands: &GO_COMMANDS,
        extra_files: vec![
            ConfigInitFile {
                path: PathBuf::from("dev/bin/check-go-mod.sh"),
                content: CHECK_GO_MOD,
                is_executable: true,
            },
            ConfigInitFile {
                path: PathBuf::from(".golangci.yml"),
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

const PERL_COMMANDS: [(&str, &str); 5] = [
    (
        "perlimports",
        r#"
type = "both"
include = ["**/*.{pl,pm,t,psgi}"]
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
include = ["**/*.{pl,pm,t,psgi}"]
cmd = ["perlcritic", "--profile=$PRECIOUS_ROOT/perlcriticrc"]
ok-exit-codes = 0
lint-failure-exit-codes = 2
"#,
    ),
    (
        "perltidy",
        r#"
type = "both"
include = ["**/*.{pl,pm,t,psgi}"]
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
include = ["**/*.{pl,pm,pod}"]
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
include = ["**/*.{pl,pm,pod}"]
cmd = ["podtidy", "--columns", "100", "--inplace", "--nobackup"]
ok-exit-codes = 0
lint-failure-exit-codes = 1
"#,
    ),
];

pub(crate) fn perl_init() -> Init {
    Init {
        excludes: &[".build/**", "blib/**"],
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
        commands: &YAML_COMMANDS,
        extra_files: vec![],
        tool_urls: &["https://prettier.io/"],
    }
}

struct ConfigElements {
    excludes: HashSet<&'static str>,
    commands: IndexMap<&'static str, &'static str>,
    extra_files: HashMap<PathBuf, ConfigInitFile>,
    tool_urls: IndexSet<&'static str>,
}

pub(crate) fn write_config_files(
    auto: bool,
    components: &[InitComponent],
    path: &Path,
) -> Result<()> {
    if env::current_dir()?.join(path).exists() {
        return Err(ConfigInitError::FileExists {
            path: path.to_owned(),
        }
        .into());
    }

    let elements = config_elements(auto, components)?;

    let mut toml = excludes_toml(&elements.excludes);

    if !toml.is_empty() {
        toml.push_str("\n\n");
    }

    toml.push_str(&commands_toml(elements.commands));

    println!();
    println!("Writing {}", path.display());

    let mut precious_toml = File::create(path)?;
    precious_toml.write_all(toml.as_bytes())?;

    write_extra_files(&elements.extra_files)?;

    println!();
    println!("The generated precious.toml requires the following tools to be installed:");
    for u in elements.tool_urls {
        println!("  {u}");
    }
    println!();

    Ok(())
}

fn config_elements(auto: bool, components: &[InitComponent]) -> Result<ConfigElements> {
    let mut excludes: HashSet<&'static str> = HashSet::new();
    let mut commands = IndexMap::new();
    let mut extra_files = HashMap::new();
    let mut tool_urls: IndexSet<&'static str> = IndexSet::new();

    for l in auto_or_component(auto, components)? {
        let init = match l {
            InitComponent::Go => go_init(),
            InitComponent::Perl => perl_init(),
            InitComponent::Rust => rust_init(),
            InitComponent::Shell => shell_init(),
            InitComponent::Gitignore => gitignore_init(),
            InitComponent::Markdown => markdown_init(),
            InitComponent::Toml => toml_init(),
            InitComponent::Yaml => yaml_init(),
        };
        excludes.extend(init.excludes);
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
        commands,
        extra_files,
        tool_urls,
    })
}

fn auto_or_component(auto: bool, components: &[InitComponent]) -> Result<Vec<InitComponent>> {
    if !auto {
        return Ok(components.to_vec());
    }

    let mut components: HashSet<InitComponent> = HashSet::new();
    let cwd = env::current_dir()?;
    debug!(
        "Looking at all files under {} to determine which components to include.",
        cwd.display(),
    );

    for result in ignore::WalkBuilder::new(&cwd).hidden(false).build() {
        let entry = result?;
        // The only time this is `None` is when the entry is for stdin, which
        // will never happen here.
        if !entry.file_type().unwrap().is_file() {
            continue;
        }

        if entry.file_name() == ".gitignore" {
            components.insert(InitComponent::Gitignore);
            continue;
        }

        let component = match entry
            .path()
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or_default()
        {
            "go" => InitComponent::Go,
            "md" => InitComponent::Markdown,
            "pl" | "pm" => InitComponent::Perl,
            "rs" => InitComponent::Rust,
            "sh" => InitComponent::Shell,
            "toml" => InitComponent::Toml,
            "yml" | "yaml" => InitComponent::Yaml,
            _ => continue,
        };
        debug!(
            "File {} matches component {:?}",
            entry.path().display(),
            component,
        );
        components.insert(component);
    }

    Ok(components.into_iter().collect())
}

fn excludes_toml(excludes: &HashSet<&str>) -> String {
    if excludes.is_empty() {
        return String::new();
    }

    if excludes.len() == 1 {
        format!("exclude = [\"{}\"]", excludes.iter().next().unwrap(),)
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

fn write_extra_files(extra_files: &HashMap<PathBuf, ConfigInitFile>) -> Result<()> {
    if extra_files.is_empty() {
        return Ok(());
    }

    println!();
    println!("Generating support files");
    println!();

    let paths = extra_files.keys().sorted().collect::<Vec<_>>();

    for p in paths {
        print!("{} ...", p.display());
        if p.exists() {
            println!("  already exists, skipping - delete this file if you want to regenerate it");
            continue;
        }
        println!(" generated");

        if let Some(parent) = p.parent() {
            create_dir_all(parent)?;
        }
        let mut file = File::create(p)?;
        let f = extra_files.get(p).unwrap();
        file.write_all(f.content.trim_start().as_bytes())?;

        #[cfg(unix)]
        if f.is_executable {
            let mut perms = file.metadata()?.permissions();
            perms.set_mode(0o755);
            file.set_permissions(perms)?;
        }
    }

    Ok(())
}
