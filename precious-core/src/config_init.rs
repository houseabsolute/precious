use std::path::PathBuf;

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

const GO_COMMANDS: [(&str, &str); 3] = [
    (
        "golangci-lint",
        r#"
type = "lint"
include = "**/*.go"
invoke = "once"
path-args = "dir"
cmd = [
    "golangci-lint",
    "run",
    "-c",
    "$PRECIOUS_ROOT/golangci-lint.yml",
    "--allow-parallel-runners",
]
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
cmd = [
    "golangci-lint",
    "run",
    "--fix",
    "--disable-all",
    "--enable", "gci",
    "--enable", "goimports",
    "--allow-parallel-runners",
]
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

const GOLANGCI_LINT_YML: &str = r#"
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
"#;

const CHECK_GO_MOD: &str = r#"
#!/bin/bash

set -e

ROOT=$( git rev-parse --show-toplevel )

if [ ! -f "$ROOT/go.sum" ]; then
    exit 0
fi

BEFORE_MOD=$( md5sum "$ROOT/go.mod" )
BEFORE_SUM=$( md5sum "$ROOT/go.sum" )

OUTPUT=$( go mod tidy -v 2>&1 )

AFTER_MOD=$( md5sum "$ROOT/go.mod" )
AFTER_SUM=$( md5sum "$ROOT/go.sum" )

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
                path: PathBuf::from("golangci-lint.yml"),
                content: GOLANGCI_LINT_YML,
                is_executable: false,
            },
        ],
        tool_urls: &["https://golangci-lint.run/"],
    }
}

const PERL_COMMANDS: [(&str, &str); 5] = [
    (
        "perlcritic",
        r#"
type = "lint"
include = [ "**/*.{pl,pm,t,psgi}" ]
cmd = [ "perlcritic", "--profile=$PRECIOUS_ROOT/perlcriticrc" ]
ok-exit-codes = 0
lint-failure-exit-codes = 2
"#,
    ),
    (
        "perltidy",
        r#"
type = "both"
include = [ "**/*.{pl,pm,t,psgi}" ]
cmd = [ "perltidy", "--profile=$PRECIOUS_ROOT/perltidyrc" ]
lint-flags = [ "--assert-tidy", "--no-standard-output", "--outfile=/dev/null" ]
tidy-flags = [ "--backup-and-modify-in-place", "--backup-file-extension=/" ]
ok-exit-codes = 0
lint-failure-exit-codes = 2
ignore-stderr = "Begin Error Output Stream"
"#,
    ),
    (
        "perlimports",
        r#"
type = "both"
include = [ "**/*.{pl,pm,t,psgi}" ]
cmd = [ "perlimports" ]
lint-flags = ["--lint" ]
tidy-flags = ["-i" ]
ok-exit-codes = 0
expect-stderr = true
"#,
    ),
    (
        "podchecker",
        r#"
type = "lint"
include = [ "**/*.{pl,pm,pod}" ]
cmd = [ "podchecker", "--warnings", "--warnings" ]
ok-exit-codes = [ 0, 2 ]
lint-failure-exit-codes = 1
ignore-stderr = [
    ".+ pod syntax OK",
    ".+ does not contain any pod commands",
]
"#,
    ),
    (
        "podtidy",
        r#"
type = "tidy"
include = [ "**/*.{pl,pm,pod}" ]
cmd = [ "podtidy", "--columns", "80", "--inplace", "--nobackup" ]
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
type    = "both"
include = "**/*.rs"
cmd     = [ "rustfmt", "--edition", "2021" ]
lint-flags = "--check"
ok-exit-codes = 0
lint-failure-exit-codes = 1
"#,
    ),
    (
        "clippy",
        r#"
type      = "lint"
include   = "**/*.rs"
invoke    = "once"
path-args = "none"
cmd = [
    "cargo",
    "clippy",
    "--locked",
    "--all-targets",
    "--all-features",
    "--workspace",
    "--",
    "-D", "clippy::all",
]
ok-exit-codes = 0
lint-failure-exit-codes = 101
ignore-stderr = [ "Checking.+precious", "Finished.+dev", "could not compile" ]
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

const GITIGNORE_COMMANDS: [(&str, &str); 1] = [(
    "omegasort-gitignore",
    r#"
type = "both"
include = "**/.gitignore"
cmd = [ "omegasort", "--sort", "path", "--unique" ]
lint-flags = "--check"
tidy-flags = "--in-place"
ok-exit-codes = 0
lint-failure-exit-codes = 1
ignore-stderr = [
    "The .+ file is not sorted",
    "The .+ file is not unique",
]
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
type    = "both"
include = "**/*.md"
cmd     = [
    "./node_modules/.bin/prettier",
    "--no-config",
    "--print-width", "100",
    "--prose-wrap", "always",
]
lint-flags = "--check"
tidy-flags = "--write"
ok-exit-codes = 0
lint-failure-exit-codes = 1
ignore-stderr = [ "Code style issues" ]
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

const YAML_COMMANDS: [(&str, &str); 1] = [(
    "prettier-yaml",
    r#"
type    = "both"
include = "**/*.yml"
cmd     = [
    "./node_modules/.bin/prettier",
    "--no-config",
]
lint-flags = "--check"
tidy-flags = "--write"
ok-exit-codes = 0
lint-failure-exit-codes = 1
ignore-stderr = [ "Code style issues" ]
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
