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

### Use ubi

Install my [universal binary installer
(ubi)](https://github.com/houseabsolute/ubi) tool and you can use it to
download `precious` and many other tools.

```
$> ubi --project houseabsolute/precious --in ~/bin
```

### Binary Releases

You can grab a binary release from the [releases
page](https://github.com/houseabsolute/precious/releases). Untar the tarball
and put the executable it contains somewhere in your path and you're good to
go.

### Cargo

You can also install this via `cargo` by running `cargo install precious`. See
[the cargo
documentation](https://doc.rust-lang.org/cargo/commands/cargo-install.html) to
understand where the binary will be installed.

## Examples

Check out this repo's [examples directory](examples) for example
`precious.toml` config files for several languages. Contributions for other
languages are welcome!

Also check out [the example
`install-dev-tools.sh`](examples/bin/install-dev-tools.sh) script. You can
customize this as needed to install only the tools you need for your project.

## Configuration

Precious is configured via a single `precious.toml` or `.precious.toml` file
that lives in your project root. The file is in [TOML
format](https://github.com/toml-lang/toml).

There is just one key that can be set in the top level table of the config file:

| Key       | Type             | Required? | Description                                                                                                                                                                                                                                                                                               |
| --------- | ---------------- | --------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `exclude` | array of strings | no        | Each array member is a pattern that will be matched against potential files when `precious` is run. These patterns are matched in the same way patterns in a [gitignore file](https://git-scm.com/docs/gitignore). However, you cannot have a pattern starting with a `!` as you can in a gitignore file. |

All other configuration is on a per-command basis. A command is something that
either tidies (aka pretty prints or beautifies) or lints your code (or does
both). These commands are external programs which precious will execute as
needed.

Each command should be defined in a block named something like
`[commands.command-name]`. Each name after the `commands.` prefix must be
unique. Note that you **can** have multiple commands defined for the same
executable as long as each one has a unique name.

Commands are run in the same order as they appear in the config file.

The keys that are allowed for each command are as follows:

| Key                       | Type                     | Required? | Applies To               | Default | Description                                                                                                                                                                                                                                                                                                                                                                                      |
| ------------------------- | ------------------------ | --------- | ------------------------ | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `type`                    | strings                  | **yes**   | all                      |         | This must be either `lint`, `tidy`, or `both`. This defines what type of command this is. Note that a command which is `both` **must** define `lint_flags` or `tidy_flags` as well.                                                                                                                                                                                                              |
| `include`                 | array of strings         | **yes**   | all                      |         | Each array member is a [gitignore file](https://git-scm.com/docs/gitignore) style pattern that tells `precious` what files this command applies to. However, you cannot have a pattern starting with a `!` as you can in a gitignore file.                                                                                                                                                       |
| `exclude`                 | array of strings         | no        | all                      |         | Each array member is a [gitignore file](https://git-scm.com/docs/gitignore) style pattern that tells `precious` what files this command should not be applied to. However, you cannot have a pattern starting with a `!` as you can in a gitignore file.                                                                                                                                         |
| `cmd`                     | array of strings         | **yes**   | all                      |         | This is the executable to be run followed by any arguments that should always be passed.                                                                                                                                                                                                                                                                                                         |
| `env`                     | table of strings->string | no        | all                      |         | This key allows you to set one or more environment variables that will be set when the command is run. Both the keys and values of this table must be strings.                                                                                                                                                                                                                                   |
| `path_flag`               | string                   | no        | all                      |         | By default, `precious` will pass each path being operated on to the command it executes as a final, positional, argument. However, if the command takes paths via a flag you need to specify that flag with this key.                                                                                                                                                                            |
| `lint_flags`              | array of strings         | no        | combined linter & tidier |         | If a command is both a linter and tidier than it may take extra flags to operate in linting mode. This is how you set that flag.                                                                                                                                                                                                                                                                 |
| `tidy_flags`              | array of strings         | no        | combined linter & tidier |         | If a command is both a linter and tidier than it may take extra flags to operate in tidying mode. This is how you set that flag.                                                                                                                                                                                                                                                                 |
| `run_mode`                | "files", "dirs", "root"  | no        | all                      | "files" | This determines how the command is run. The default, "files", means that the command is run once per file that matches its include/exclude settings. If this is set to "dirs", then the command is run once per directory _containing_ files that matches its include/exclude settings. If it's set to "root", then it is run exactly once from the root of the project if it matches any files. |
| `chdir`                   | boolean                  | no        | all                      | false   | If this is true, then the command will be run with a chdir to the relevant path. If the command operates on files, `precious` chdir's to the file's directory. If it operates on directories than it changes to each directory. Note that if `run_mode` is `dirs` and `chdir` is true then `precious` will not pass the path to the executable as an argument.                                   |
| `ok_exit_codes`           | array of integers        | **yes**   | all                      |         | Any exit code that **does not** indicate an abnormal exit should be here. For most commands this is just `0` but some commands may use other exit codes even for a normal exit.                                                                                                                                                                                                                  |
| `lint_failure_exit_codes` | array of integers        | no        | linters                  |         | If the command is a linter then these are the status codes that indicate a lint failure. These need to be specified so `precious` can distinguish an exit because of a lint failure versus an exit because of some unexpected issue.                                                                                                                                                             |
| `expect_stderr`           | boolean                  | all       | false                    |         | By default, `precious` assumes that when a command sends output to `stderr` that indicates a failure to lint or tidy. If this is not the case, set this to true.                                                                                                                                                                                                                                 |

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

| Flag                        | Description                                                         |
| --------------------------- | ------------------------------------------------------------------- |
| `-h`, `--help`              | Prints help information                                             |
| `-q`, `--quiet`             | Suppresses most output                                              |
| `-V`, `--version`           | Prints version information                                          |
| `-v`, `--verbose`           | Enable verbose output                                               |
| `-d`, `--debug`             | Enable debugging output                                             |
| `-t`, `--trace`             | Enable tracing output (maximum logging)                             |
| `--ascii`                   | Replace super-fun Unicode symbols with terribly boring ASCII        |
| `-c`, `--config` `<config>` | Path to config file                                                 |
| `-j`, `--jobs` `<jobs>`     | Number of parallel jobs (threads) to run (defaults to one per core) |

### Subcommands

The `precious` command has two subcommands, `lint` and `tidy`. You must always
specify one of these. These subcommands take the same options.

#### Selecting Paths to Operate On

When you run `precious` you must tell it what paths to operate on. Precious
supports several ways of setting these via command line arguments:

| Mode                                                         | Flag                  | Description                                                                                                                                                                                                                                                                                                       |
| ------------------------------------------------------------ | --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| All paths                                                    | `-a`, `--all`         | Run on all paths in the project.                                                                                                                                                                                                                                                                                  |
| Modified files according to git                              | `-g`, `--git`         | Run on all files that git reports as having been modified.                                                                                                                                                                                                                                                        |
| Staged files according to git                                | `-s`, `--staged`      | Run on all files that git reports as having been staged.                                                                                                                                                                                                                                                          |
| Staged files according to git, with unstaged changes stashed | `--staged-with-stash` | This is liked `--stashed`, but it will stash unstaged changes while it runs and pop the stash at the end. This ensures that commands only run against the staged version of your codebase. This can cause issues with many editors or other tools that watch for file changes, so exercise care with this option. |
| Paths given on CLI                                           |                       | If you don't pass any of the above flags then `precious` will expect one or more paths to be passed on the command line after all other options. If any of these paths are directories then that entire directory tree will be included.                                                                          |

#### Running One Command

You can tidy or lint with just a single command by passing the `--command` flag:

```
$> precious lint --command some-command --all
```

The name passed to `--command` must match the name of the command in your
config file. So in the above example, this would look for a command defined as
`[commands.some-command]` in your config.

#### Default Exclusions

When selecting paths `precious` _always_ respects your ignore files. Right now
it only knows how this works for git, and it will respect all of the following
ignore files:

- Per-directory `.ignore` and `.gitignore` files.
- The `.git/info/exclude` file.
- Global gitignore globs, usually found in `$XDG_CONFIG_HOME/git/ignore`.

This is implemented using the [rust `ignore`
crate](https://crates.io/crates/ignore), so adding support for other VCS
systems should be proposed there.

In addition, you can specify excludes for all commands by setting a global
`exclude` key.

Finally, you can specify per-command `include` and `exclude` keys.

When `precious` runs it does the following to determine which commands apply to
which paths.

- The base paths are selected based on the command line option specified.
- VCS ignore rules are applied to remove paths from this list.
- Each command is given either the files or directories from the list of paths,
  depending on the `run_mode` setting for that command.
  - If the command's `run_mode` is `root`, then it will get all of the files in
    all directories and will use those to determine whether to run or
    not. These commands are always run exactly once if any of the files match.
- The command will check its include and exclude rules. The path must match at
  least one include rule _and_ not match any exclude rules to be accepted.
  - If the command is per-file, it matches each path against its rules as is.
  - If the command is per-directory, it matches the files in the directory
    against its include and exclude rules. If _any_ of the files match the
    command is run. If _none_ of the files match the command is not run.

## Configuration Recommendations

Here are some recommendations for how to get the best experience with precious.

### Choosing a Run Mode

Some tools might work equally well with "root" or "dirs" as a the run
mode. The right run mode to choose depends on how you are using precious.

In general, if you either have a very small set of directories, _or_ you are
running precious on most or all of the directories at once, then the "root"
mode will be faster.

However, if you have a larger set of directories and you only need to lint or
tidy a small subset of these at once, then "dirs" mode will be faster.

### Quiet Flags

Many tools will accept a "quiet" flag of some sort. In general, you probably
_do not_ want to run tools in a quiet mode with precious.

In the case of a successful tidy or lint command execution, precious already
captures all stdout from the command that it runs. If the command fails
somehow, precious will print out stdout (and stderr) output.

By default, precious treats _any_ output to stderr as an error in the command
(as opposed to a linting failure). If you set `expect_stderr = true`, then
precious treats stderr just like stdout.

In addition, you can see all stdout and stderr output when running precious in
`--debug` mode.

All of which is to say that in general there's no value to running a command
in quiet mode with precious. All that does is make it harder to debug issues
with that command.

## Common Scenarios

There are some configuration scenarios that you may need to handle. Here are
some examples:

### Linter runs just once for the entire source tree

Some linters, such as [rust-clippy](https://github.com/rust-lang/rust-clippy),
expect to run just once across the entire source tree, rather than once per
file or directory.

In order to make that happen you should use the following config:

```toml
include = "**/*.rs"
run_mode = "root"
```

This combination of flags will cause `precious` to run the command exactly
once in the project root.

The above config will pass a path to the command, `.`. If the command does not
need a path, set `chdir` to `true`:

```toml
include = "**/*.rs"
run_mode = "root"
chdir = true
```

### Linter runs in the same directory as the files it lints and does not accept path as arguments

If you want to run the command without passing the path being operated on to
the command, set `run_mode` to `dirs` and add the `chdir` flag:

```toml
include  = "**/*.rs"
run_mode = "dirs"
chdir    = true
```

### You want a command to exclude an entire directory (tree) except for one file

There's no good way to do this with a single command's `include` and `exclude`,
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
status if any of the lint commands indicate a linting problem.

### You want to run commands in a specific order

As of version 0.1.2, commands are run in the same order as they appear in the
config file.

## Build Status

### Build and Test

![Build Status](https://github.com/houseabsolute/precious/actions/workflows/ci.yml/badge.svg)

### Cargo Audit Nightly

![Cargo Audit Nightly](https://github.com/houseabsolute/precious/actions/workflows/audit-nightly.yml/badge.svg)

### Cargo Audit On Push

![Cargo Audit On Push](https://github.com/houseabsolute/precious/actions/workflows/audit-on-push.yml/badge.svg)
