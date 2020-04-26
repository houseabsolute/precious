# Precious - One Code Quality Tool to Rule Them All

Who doesn't love linters and tidiers? I sure love them. I love them so much
that in many of my projects I might easily have five or ten of them enabled!

Wouldn't it be great if you could run all of them with just one command?
Wouldn't it be great if that command just had one config file to define what
tools to run on each part of your project? Wouldn't it be great if Sauron were
our ruler?

Now with Precious you can say "yes" to all of those questions.

## Why Precious?

In all seriousness, managing code quality tools can be a bit of a pain. It
becomes **much** more painful when you have a multi-language project. You may
have multiple tools per language, each of which runs on some subset of your
codebase. Then you need to hook these tools into your commit hooks and CI
system.

With Precious you can configure all of your code quality tool rules in one
place and easily run `precious` from your commit hooks and in CI.

## Installation

There are several ways to install this tool.

### Binary Releases

The easiest way to install it is to grab a binary release from the [releases
page](https://github.com/houseabsolute/precious/releases). Simply put this
somewhere in your path and you're good to go.

### Cargo

You can also install this via `cargo` by running `cargo install precious`. See
[the cargo
documentation](https://doc.rust-lang.org/cargo/commands/cargo-install.html)
for the rules on where the binary is installed.

## Configuration

Precious is configured via a single `precious.toml` file that lives in your
project root. The file is in [TOML format](https://github.com/toml-lang/toml).

There is just one key that can be set in the top level table of the config file:

| Key | Type | Required? | Description |
| --- | ---- | --------- | ----------- |
| `exclude` | array of strings | no | Each array member is a pattern that will be matched against potential files when `precious` is run. These patterns are matched in the same way patterns in a [gitignore file](https://git-scm.com/docs/gitignore). However, you cannot have a pattern starting with a `!` as you can in a gitignore file. |

All other configuration is on a per-filter basis. A filter is something that
either tidies (aka pretty prints or beautifies) or lints your code (or
both). Currently all filters are defined as commands, external programs which
precious will execute as needed.

Each filter should be defined in a block named something like
`[commands.filter-name]`. Each name after the `commands.` prefix must be
unique. Note that you **can** have multiple filters defined for the same
executable as long as each one has a unique name.

The keys that are allowed for each command are as follows:

| Key | Type | Required? | Applies To | Default | Description |
| --- | ---- | --------- | ---------- | ------- | ----------- |
| `type` | strings | **yes** | all | | This must be either `lint`, `tidy`, or `both`. This defines what type of filter this is. Note that a filter which is `both` **must** define `lint_flags` or `tidy_flags` as well. |
| `include` | array of strings | **yes** | all | | Each array member is a [gitignore file](https://git-scm.com/docs/gitignore) style pattern that tells `precious` what files this filter applies to. However, you cannot have a pattern starting with a `!` as you can in a gitignore file. |
| `exclude` | array of strings | no | all | | Each array member is a [gitignore file](https://git-scm.com/docs/gitignore) style pattern that tells `precious` what files this filter should not be applied to. However, you cannot have a pattern starting with a `!` as you can in a gitignore file. |
| `cmd` | array of strings | **yes** | all | | This is the executable to be run followed by any arguments that should always be passed. |
| `env` | table of strings->string | no | all | | This key allows you to set one or more environment variables that will be set when the command is run. Both the keys and values of this table must be strings. |
| `path_flag` | string | no | all | | By default, `precious` will pass each path being operated on to the command it executes as a final, positional, argument. However, if the command takes paths via a flag you need to specify that flag with this key.
| `lint_flags` | array of strings | no | combined linter & tidier | | If a command is both a linter and tidier than it may take extra flags to operate in linting mode. This is how you set that flag. |
| `tidy_flags` | array of strings | no | combined linter & tidier | | If a command is both a linter and tidier than it may take extra flags to operate in tidying mode. This is how you set that flag. |
| `run_mode` | "files", "dirs", "root" | no | all | "files" | This determines how the command is run. The default, "files", means that the command is given a list of files that matched its include/exclude settings to run against. If this is set to "dirs", then the command is given a list of directories containing files that matched its include/exclude settings. If it's set to "root", then it is run exactly once from the root of the project. |
| `chdir` |  boolean | no | all | false | If this is true, then the command will be run with a chdir to the relevant path. If the command operates on files, `precious` chdir's to the file's directory. If it operates on directories than it changes to each directory. Note that if both `on_dir` and `chdir` are true then `precious` will not pass the path to the executable as an argument. |
| `ok_exit_codes` | array of integers | **yes** | all | | Any exit code that **does not** indicate an abnormal exit should be here. For most commands this is just `0` but some commands may use other exit codes even for a normal exit. |
| `lint_failure_exit_codes` | array of integers | no | linters |  | If the command is a linter then these are the status codes that indicate a lint failure. These need to be specified so `precious` can distinguish an exit because of a lint failure versus an exit because of some unexpected issue. |
| `expect_stderr` | boolean | all | false | | By default, `precious` assumes that when a command sends output to `stderr` that indicates a failure to lint or tidy. If this is not the case, set this to true. |

### Referencing the Project Root

For tools that can be run from a subdirectory, you may need to specify config
files in terms of the project root. You can do this by using the string
`$PRECIOUS_ROOT` in any element of the `cmd` configuration key. So for example
you might write something like this:

```toml
cmd = ["some-tidier", "--config", "$PRECIOUS_ROOT/some-tidier.conf"]
```

The `$PRECIOUS_ROOT` string will be replaced by the absolute path to the
project root.

## Running Precious

To get help run `precious --help`.

The root command takes the following options:

| Flag | Description |
| ---- | ----------- |
| `-h`, `--help` | Prints help information |
| `-q`, `--quiet` | Suppresses most output |
| `-V`, `--version` | Prints version information |
| `-v`, `--verbose` | Enable verbose output |
| `-d`, `--debug` | Enable debugging output |
| `-t`, `--trace` | Enable tracing output (maximum logging) |
| `--ascii` | Replace super-fun Unicode symbols with terribly boring ASCII |
| `-c`, `--config` `<config>` | Path to config file |

### Subcommands

The `precious` command has two subcommands, `lint` and `tidy`. You must always
specify one of these. These subcommands take the same options, all of which
are for selecting paths to operate on.

### Selecting Paths to Operate On

When you run `precious` you must tell it what paths to operate on. Precious
supports several ways of setting these via command line arguments:

| Mode | Flag | Description |
| ---- | ---- | ----------- |
| All paths | `-a`, `--all` | Run on all paths in the project. |
| Modified files according to git | `-g`, `--git` | Run on all files that git reports as having been modified. |
| Staged files according to git | `-s`, `--staged` | Run on all files that git reports as having been staged. This will stash unstaged changes while it runs and pop the stash at the end. This ensures that filters only run against the staged version of your codebase. |
| Paths given on CLI | | If you don't pass any of the above flags then `precious` will expect one or more paths to be passed on the command line after all other options. If any of these paths are directories then that entire directory tree will be included. |

#### Default Exclusions

When selecting paths `precious` *always* respects your ignore files. Right now
it only knows how this works for git, and it will respect all of the following
ignore files:

* Per-directory `.ignore` and  `.gitignore` files.
* The `.git/info/exclude` file.
* Global gitignore globs, usually found in `$XDG_CONFIG_HOME/git/ignore`.

This is implemented using the [rust `ignore`
crate](https://crates.io/crates/ignore), so adding support for other VCS
systems should be proposed there.

In addition, you can specify excludes for all filters by setting a global
`exclude` key.

Finally, you can specify per-filter `include` and `exclude` keys.

When `precious` runs it does the following to determine which filters apply to
which paths.

* The base paths are selected based on the command line option specified.
* VCS ignore rules are applied to remove paths from this list.
* Each filter is given either the files or directories from the list of paths,
  depending on the `on_dir` setting for that filter.
  * Except for `run_once` filters, which will get all of the files in all
    directories and will use those to determine whether to run or not. These
    filters are always run exactly once.
* The filter will check its include and exclude rules. The path must match at
  least one include rule *and* not match any exclude rules to be accepted.
  * If the filter is per-file, it matches each path against its rules as is.
  * If the filter is per-directory, it matches the files in the directory
    against its include and exclude rules. If *any* of the files match the
    filter is run. If *none* of the files match the filter is not run.

## Examples

Here are some example command configurations:

### [rustfmt](https://github.com/rust-lang/rustfmt)

```toml
[commands.rustfmt]
type    = "both"
include = "**/*.rs"
cmd     = ["rustfmt"]
lint_flags = "--check"
ok_exit_codes = [0]
lint_failure_exit_codes = [1]
```

### [rust-clippy](https://github.com/rust-lang/rust-clippy)

```toml
[commands.clippy]
type     = "lint"
include  = "**/*.rs"
on_dir   = true
chdir    = true
run_once = true
cmd      = ["cargo", "clippy", "-q", "--", "-D", "clippy::all"]
ok_exit_codes = [0]
lint_failure_exit_codes = [1]
```

### [goimports](https://godoc.org/golang.org/x/tools/cmd/goimports)

```toml
[commands.goimports]
type    = "tidy"
include = "**/*.go"
cmd     = ["goimports", "-w"]
ok_exit_codes = 0
```

### [golangci-lint](https://github.com/golangci/golangci-lint)

```toml
[commands.golangci-lint]
type = "lint"
include = "**/*.go"
run_mode = "root"
cmd = [
    "golangci-lint",
    "run",
    "-c",
    "$PRECIOUS_ROOT/golangci-lint.yml",
]
# This is an undocumented env var that golangci-lint looks for.
env = { "FAIL_ON_WARNINGS": "1" }
ok_exit_codes = [0]
lint_failure_exit_codes = [1]
```

## Common Scenarios

There are some configuration scenarios that you may need to handle. Here are
some examples:

### Linter runs just once for the entire source tree

Some linters, such as [rust-clippy](https://github.com/rust-lang/rust-clippy),
expect to run just once across the entire source tree, rather than once per
file or directory.

In order to make that happen you should use the following config:

```toml
include = "."
on_dir  = true
run_once = true
```

This combination of flags will cause `precious` to run the command exactly
once in the project root. 

### Linter runs in the same directory as the files it lints and does not accept path as arguments

If you want to run the command without passing the path being operated on to
the command, add the `chdir` flag:

```toml
include = "**/*.rs"
on_dir  = true
chdir   = true
```

You will probably want to set the `on_dir` flag to true in such cases, but
these two flags are independent in case there are tools where setting just
`chdir` makes sense.

### You want a command to exclude an entire directory (tree) except for one file

There's no good way to do this with a single filter's `include` and `exclude`,
as `excluding` a directory means that any attempt to `include` a file under
that directory will be ignored. Instead, you can configure the same command
twice:

```toml
[commands.rustfmt-most]
type    = "both"
include = "**/*.rs"
exclude = "path/to/dir"
cmd     = ["rustfmt"]
lint_flags = "--check"
ok_exit_codes = [0]
lint_failure_exit_codes = [1]

[commands.rustfmt-that-file]
type    = "both"
include = "path/to/dir/that.rs"
cmd     = ["rustfmt"]
lint_flags = "--check"
ok_exit_codes = [0]
lint_failure_exit_codes = [1]
```

### You want to run Precious as a commit hook

Simply run `precious lint -s` in your hook. It will exit with a non-zero
status if any of the lint filters indicate a linting problem.

## Build Status

[![Build Status](https://travis-ci.com/houseabsolute/precious.svg?branch=master)](https://travis-ci.com/houseabsolute/precious)
