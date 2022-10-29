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
either tidies (aka pretty prints or beautifies), lints, or does both. These
commands are external programs which precious will execute as needed.

Each command is defined in a block named something like
`[commands.command-name]`. Each name after the `commands.` prefix must be
unique. You **can** have run the same executable differently with different
commands as long as each command has a unique name.

Commands are run in the same order as they appear in the config file.

### Command Invocation

There are three configuration keys for command invocation. All of them are
optional. If none are specified, `precious` defaults to this:

```toml
invoke      = "per-file"
working-dir = "root"
path-args   = "file"
```

This runs the command once per file, passing the file as a single argument to
the command. The working directory for the command will be the project root.

#### `invoke`

The `invoke` key tells `precious` how the command should be invoked.

| Value        | Description                                                                                                              |
| ------------ | ------------------------------------------------------------------------------------------------------------------------ |
| `"per-file"` | Run this command once for each matching file. **This is the default.**                                                   |
| `"per-dir"`  | Run this command once for each matching directory.                                                                       |
| `"once"`     | Run this command either once for the project or once per sub-root. See the `working-dir` documentation below for details |

#### `working-dir`

The `working-dir` key tells precious what the working directory should be when the
command is run.

| Value                   | Description                                                                                                                                                               |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `"root"`                | The working directory is the project root. **This is the default.**                                                                                                       |
| `"dir"`                 | The working directory is the directory containing the matching files. This means `precious` will `chdir` into each matching directory in turn as it executes the command. |
| `{ sub-roots = [...] }` | See below                                                                                                                                                                 |

##### `working-dir = { sub-roots = [ ... ] }`

The final option for `working-dir` is to pass a table (aka map) instead of
`"root"` or `"dir"` as a string. In that case, the table should have one key
of its own, `sub-roots`. The value of that key can either be a single string
or an array of strings. Each of these strings should be a _relative_ path to a
directory under your project root.

When a command is configured with `sub-roots`, `precious` will do the
following based on other aspects of your config.

- The working directory will be set to each root in turn.
- Relative paths will be relative to each root, rather than the project root.
- If `invoke = "once"`, then the command will be run _once per root_ instead
  of just once from the project root.

#### `path-args`

The `path-args` key tells precious how paths should be passed when the command
is run.

| Value             | Description                                                                                                                                                                          |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `"file"`          | Passes the path to the matching file relative to the root. **This is the default.** <br> If using `sub-roots`, then the path is relative to the current working directory, per-root. |
| `"dir"`           | Passes the path to the directory containing the matching files relative to the root. <br> If using `sub-roots`, then the path is relative to the current working directory, per-root |
| `"none"`          | No file args are passed to the command at all.                                                                                                                                       |
| `"dot"`           | Always pass `.` as the path. This is useful when `working-dir = "dir"` and the command still requires a path to be passed.                                                           |
| `"absolute-file"` | Passes the path to the matching file as an absolute path from the filesystem's root directory.                                                                                       |
| `"absolute-dir"`  | Passes the path to the directory containing the matching files as an absolute path from the filesystem's root directory.                                                             |

#### Nonsensical Combinations

Most combinations of these configuration keys are allowed, but there are some
nonsensical combinations that will cause `precious` to exit with an error.

```
invoke = "per-file"
path-args = "dir", "none", "dot", or "absolute-dir"
```

You cannot invoke a command once per file without passing the filename.

```
invoke = "per-dir"
path-args = "none" or "dot"
working-dir = "root"
# ... or ...
working-dir = { sub-roots = [ "whatever" ] }
```

You cannot invoke a command once per dir from a root without passing the
directory name or a list of file names. If you want to run a command once per
directory with no path arguments or using `.` as the path then you _must_ set
`working-dir = "dir"`.

```
invoke = "once"
working-dir = "dir"
```

You cannot invoke a command once if the working directory is set to each
matching directory in turn.

#### Invocation Examples

See the [Invocation Examples documentation](docs/invocation-examples.md) for
comprehensive examples of every possible set of options.

### Other Per-Command Configuration Keys

The other keys allowed for each command are as follows:

| Key                       | Type                         | Required? | Applies To               | Default | Description                                                                                                                                                                                                                                                                                                              |
| ------------------------- | ---------------------------- | --------- | ------------------------ | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `type`                    | string                       | **yes**   | all                      |         | This must be either `lint`, `tidy`, or `both`. This defines what type of command this is. A command which is `both` **must** define `lint_flags` or `tidy_flags` as well.                                                                                                                                                |
| `include`                 | string or array of strings   | **yes**   | all                      |         | Each array member is a [gitignore file](https://git-scm.com/docs/gitignore) style pattern that tells `precious` what files this command applies to. <br> However, you cannot have a pattern starting with a `!` as you can in a gitignore file.                                                                          |
| `exclude`                 | string or array of strings   | no        | all                      |         | Each array member is a [gitignore file](https://git-scm.com/docs/gitignore) style pattern that tells `precious` what files this command should not be applied to. <br> However, you cannot have a pattern starting with a `!` as you can in a gitignore file.                                                            |
| `cmd`                     | string or array of strings   | **yes**   | all                      |         | This is the executable to be run followed by any arguments that should always be passed.                                                                                                                                                                                                                                 |
| `env`                     | table - values are strings   | no        | all                      |         | This key allows you to set one or more environment variables that will be set when the command is run. Both the keys and values of this table must be strings.                                                                                                                                                           |
| `path_flag`               | string                       | no        | all                      |         | By default, `precious` will pass the path being operated on to the command it executes as the final, positional, argument(s). If the command takes paths via a flag you need to specify that flag with this key.                                                                                                         |
| `lint_flags`              | string or array of strings   | no        | combined linter & tidier |         | If a command is both a linter and tidier then it may take extra flags to operate in linting mode. This is how you set that flag.                                                                                                                                                                                         |
| `tidy_flags`              | string or array of strings   | no        | combined linter & tidier |         | If a command is both a linter and tidier then it may take extra flags to operate in tidying mode. This is how you set that flag.                                                                                                                                                                                         |
| `ok_exit_codes`           | integer or array of integers | **yes**   | all                      |         | Any exit code that **does not** indicate an abnormal exit should be here. For most commands this is just `0` but some commands may use other exit codes even for a normal exit.                                                                                                                                          |
| `lint_failure_exit_codes` | integer or array of integers | no        | linters                  |         | If the command is a linter then these are the status codes that indicate a lint failure. These need to be specified so `precious` can distinguish an exit because of a lint failure versus an exit because of some unexpected issue.                                                                                     |
| `ignore_stderr`           | string or array of strings   | all       | all                      |         | By default, `precious` assumes that when a command sends output to `stderr` that indicates a failure to lint or tidy. This parameter can specify one or more strings. These strings are treated as regexes and matched against the command's stderr output. If _any_ of the regexes match, the stderr output is ignored. |

### Referencing the Project Root

For commands that can be run from a subdirectory, you may need to specify
config files in terms of the project root. You can do this by using the string
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

#### How Include and Exclude Are Applied

When `precious` runs it does the following to determine which commands apply to
which paths.

- The base files to operate on are selected based on the command line option
  specified. This is one of:
  - `--all` - All files in the current working directory's tree.
  - `--git` - All files in the git repo that have been modified.
  - `--staged` - All files in the git repo that have been staged.
  - paths passed on the CLI - If a path is a file it is added to the list
    as-is. If the path is a directory then all the files under that directory
    (recursively) are found.
- VCS ignore rules are applied to remove files from this list.
- The global exclude rules are applied to remove files from this list.
- Based on the command's `invoke` key, a list of files to be checked is
  generated and the command's include/exclude rules are applied. To be
  included, a file must match at least one include rule _and_ not match any
  exclude rules to be accepted.
  - If `invoke` is `per-file`, then the rules are applied one file at a time.
  - If `invoke` is `per-dir`, then if any file in the directory matches the
    rules, the command will be run on that directory.
  - If `invoke` is `once`, then the are applied to all of the files at
    once. If any one of those files matches the include rule, the command will
    be run.

## Configuration Recommendations

Here are some recommendations for how to get the best experience with precious.

### Choosing How to `invoke` the Command

Some commands might work equally well with `invoke` set to either `per-dir` or
`invoke`. The right run mode to choose depends on how you are using precious.

In general, if you either have a very small set of directories, _or_ you are
running precious on most or all of the directories at once, then `once` will
be faster.

However, if you have a larger set of directories and you only need to lint or
tidy a small subset of these at once, then `per-dir` mode will be faster.

### Quiet Flags

Many commands will accept a "quiet" flag of some sort. In general, you
probably _do not_ want to run commands in a quiet mode with precious.

In the case of a successful tidy or lint command execution, precious already
hides all stdout from the command that it runs. If the command fails somehow,
precious will print out the command's stdout and stderr output.

By default, precious treats _any_ output to stderr as an error in the command
(as opposed to a linting failure). You can use the `ignore_stderr` to specify
one or more regexes for allowed stderr output.

In addition, you can see all stdout and stderr output when running precious in
`--debug` mode.

All of which is to say that in general there's no value to running a command
in quiet mode with precious. All that does is make it harder to debug issues
with that command when lint checks fail or other issues occur.

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
invoke = "once"
path-args = "dot" # or "none"
```

This will cause `precious` to run the command exactly once in the project
root.

### Linter runs in the same directory as the files it lints and does not accept path as arguments

If you want to run the command without passing the path being operated on to
the command, set `invoke = "per-dir"` and `path-args = "none"`:

```toml
include   = "**/*.rs"
invoke    = "per-dir"
path-args = "none"
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
