<!-- next-header -->

- Added `shell` and `toml` components for `precious config --component ...` options.

## 0.7.1 - 2024-05-05

- Added an `--auto` flag for `precious config init`. If this is specified then `precious` will look
  at all the files in your project and generate config based on the types of files it finds.
  Suggested by John Vandenberg (@jayvdb). GH #67.
- Fixed a bug when running `precious config init`. The `--component` argument was not required, when
  it should require at least one. If none were given it would create an empty `precious.toml` file.
  Reported by John Vandenberg (@jayvdb). GH #67.
- Changed how `precious config init` generates config for Perl. The `perlimports` command is now the
  first one in the generated config. This is necessary because `perlimports` may change the code in
  a way that `perltidy` will then change further. Running `perlimports` after `perltidy` meant that
  tidying Perl code could leave the code in a state where it fails linting checks. Implemented by
  Olaf Alders (@oalders). GH #68.
- Added/cleaned up some debugging output for the new `invoke.per-x-or-y` options. Fixes GH #65 and
  #66.

## 0.7.0 - 2024-03-30

- Added three new **experimental** `invoke` options:

  - `invoke.per-file-or-dir = n`
  - `invoke.per-file-or-once = n`
  - `invoke.per-dir-or-once = n`

  These will run the command in different ways depending on how many files or directories match the
  command's config. This lets you pick the fastest way to run commands that can be invoked in more
  than one way. For example, if you're in a large repo and have only made changes to files in a few
  directories, `golangci-lint` is much faster when run once per directory. But once the number of
  directories is large enough, it's faster to just run it once on the whole repo.

- All config keys have been changed to use dashes instead of underscores, so for example `path_args`
  is not `path-args` and `ok_exit_codes` is now `ok-exit-codes`. However, the names with underscores
  will continue to work. I do not intend to ever deprecate the underscore version. They simply will
  not be used in the docs and examples.

- Fixed cases where `precious` would exit with an exit code of `1` on errors that were _not_ linting
  failures. Going forward, an exit code of `1` should only be used for linting failures.

- `precious` will now emit a warning if your config file uses any of the deprecated config keys,
  `run_mode` and `chdir`. Support for these options will be removed entirely in a future release.

## 0.6.4 - 2024-03-23

- Added a `--git-diff-from <REF>` option that will find all files changed in the diff between
  `<REF>` and the current `HEAD`. Requested by Michael McClimon. GH #64.

## 0.6.3 - 2024-03-05

- When running `precious config init` and asking for the Perl or Rust components, `precious` would
  tell you to install `omegasort` even though the generated config did not use it. Reported by Olaf
  Alders. GH #61.
- If precious was run with `--staged` and a file that was staged had been deleted from the
  filesystem but not removed with `git rm`, it would exit with a very unhelpful error like
  `Error: No such file or directory (os error 2)`, without any indication of what the file was. Now
  it will simply ignore such deleted file, though it will log a debug-level message saying that the
  file is being ignored. GH #63.

## 0.6.2 - 2023-12-18

- When printing the results of running a command that was invoked with a long list of paths, the
  output will now summarize the paths affected instead of always printing all of them. GH #60.

## 0.6.1 - 2023-11-06

- The `dev/bin/check-go-mod.sh` script created when running `precious config init --component go` is
  now executable. Reported by Olaf Olders. GH #56.
- The generated config for Go now excludes the `vendor` directory for all commands. Implemented by
  Olaf Alders. GH #57.
- When running `precious config init`, it would overwrite an existing file if it was given a
  `--path` argument, but not if the argument was left unset. Now it will always error out instead of
  overwriting an existing file. Reported by Olaf Alders. GH #58.
- When running `precious config init --component go` a `golangci-lint.yml` file will also be
  created. GH #59.
- As of this release there are no longer binaries built for MIPS on Linux. These targets have been
  demoted to tier 3 support by the Rust compiler.

## 0.6.0 - 2023-10-29

- Added a new `precious config init` command that can generate `precious.toml` for you. Suggested by
  Olaf Alders. GH #53.

## 0.5.2 - 2023-10-09

- Help output is now line-wrapped based on your terminal width.
- Added a new `precious config list` command that prints a table showing all the commands in your
  config file. Requested by Olaf Alders. GH #52.

## 0.5.1 - 2023-03-11

- Added a new labels feature. This allows you to group commands in your config file by assigning one
  or more `labels` in their config. Then when running `precious`, you can run commands with a
  specific label: `precious lint --label some-label --all`. Suggested by Greg Oschwald. Addresses
  #8.

## 0.5.0 - 2023-02-04

- The `--git` flag did not include any staged files, only files that were modified but _not_ staged.
  It now includes all modified files, whether or not they're staged.

## 0.4.1 - 2022-11-26

- The previous release didn't handle all of the old config keys correctly. If just `run_mode` or
  `chdir` was set, but not both, it may not have replicated the behavior of precious v0.3.0 and
  earlier with the same settings.

## 0.4.0 - 2022-11-19

- **This release has huge changes to how commands are invoked. The old `run_mode` and `chdir`
  configuration keys have been deprecated. In the future, using these will cause precious to print a
  warning, and later support will be removed entirely** See [the documentation](README.md) for more
  details. There are also some
  [docs on upgrading from previous versions](docs/upgrade-from-0.3.0-to-0.4.0.md).
- Fixed path handling for `--git` and `--staged` when the project root (the directory containing the
  precious config file) is a subdirectory of the git repo root. Previously this would just attempt
  to run against incorrect paths.
- Precious now supports patterns starting with `!` in `include` and `exclude` keys. This allow you
  to exclude the given pattern, even if matches previous rules in the list. See
  [the Git docs on `.gitignore` patterns](https://git-scm.com/docs/gitignore#_pattern_format) for
  more details. Fixes GH #39.
- When run in GitHub Actions, `precious` will now emit
  [GitHub annotations](https://docs.github.com/en/actions/using-workflows/workflow-commands-for-github-actions#setting-an-error-message)
  for linting errors.

## 0.3.0 - 2022-10-02

- The `expect_stderr` config parameter has been replaced by `ignore_stderr`. This new parameter
  accepts one or more strings, which are turned into regexes. If the command's `stderr` output
  matches _any_ of the regexes then it is ignore. The old `expect_stderr` parameter will continue to
  work for now, but it is no longer documented. To replicate the old behavior simply set
  `ignore_stderr = ".*"`.

## 0.2.3 - 2022-10-01

- When given the , `--git`, `--staged`, or `--staged-with-stash` flags, precious would error out if
  all the relevant files were excluded. This is likely to break commit hooks so this is no longer an
  error. However, if given either the `--all` flag or an explicit list of files, it will still error
  if all of them are excluded.

## 0.2.2 - 2022-09-24

- Added a `--command` flag to the `lint` and `tidy` subcommands. If this is passed, then only the
  command with the given name will be run. This addresses #31, requested by Olaf Alders.

## 0.2.1 - 2022-09-18

- The way precious works when run in a subdirectory of the project root has changed.
  - When given the `--all`, `--git`, `--staged`, or `--staged-with-stash` flags, it will look for
    all files in the project, regardless of what directory you execute `precious` in.
  - When given relative paths to files it will do the right thing. Previously it would error out
    with "No such file or directory". Reported by Greg Oschwald. Fixes #29.

## 0.2.0 - 2022-09-15

- The `--staged` mode no longer tries to stash unstaged content before linting or tidying files.
  This can cause a number of issues, and shouldn't be the default. There is a new
  `--staged-with-stash` mode that provides the old `--staged` behavior. Reported by Greg Oschwald.
  Fixes #30.

## 0.1.7 - 2022-09-03

- If a command sent output to stdout, but not stderr, and exited with an unexpected error code, then
  the output to stdout would not be shown by precious in the error message. Reported by Greg
  Oschwald. Fixes #28.

## 0.1.6 - 2022-09-02

- All binaries now statically link musl instead of the system libc.
- Added a number of new platforms for released binaries: Linux ARM 32-bit and 64-bit, and macOS ARM
  64-bit.

## 0.1.5 - 2022-08-27

- When a command unexpectedly prints to stderr the error message we print now includes both stdout
  and stderr from that command. Reported by Greg Oschwald. Fixes #26.
- When a command was configured with the `run_mode` as `files` and `chdir` as `true`, the paths
  passed to the command would still include parent directories. Reported by Greg Oschwald. Fixes
  #25.

## 0.1.4 - 2022-08-14

- Running precious with the `--staged` flag would exit with an error if a post-checkout hook wrote
  any output to stderr. It appears that any output from a hook to stdout ends up on stderr for some
  reason, probably related to
  https://github.com/git/git/commit/e258eb4800e30da2adbdb2df8d8d8c19d9b443e4. Based on PR#24 by Olaf
  Alders. Fixes #23.

## 0.1.3 - 2022-02-19

- Relaxed some dependencies for the benefit of packaging precious for Debian. Implemented by Jonas
  Smedegaard.
- Added support for `.precious.toml` as a config name. Based on PR#21 by Olaf Alders. Fixes #13.

## 0.1.2 - 2021-10-14

- The order of commands in the config file is now preserved, and commands are executed in the order
  in which they appear in the config file. This addresses #12, requested by Olaf Alders.
- Fixed the tests so that they set the default branch name when running `git init`, rather than
  setting this via `git config`. This lets anyone run the tests, whereas it was only safe to set
  this via `git config` in CI. This fixes #14, reported by Olaf Alders.

## 0.1.1 - 2021-07-12

- Fixed config handling of a global `exclude` key. The previous release did not handle a single
  string as that key's value, only an array.

## 0.1.0 - 2021-07-02

- The verbose and debugging level output now includes timing information on each linter and tidier
  that is run. This is helpful if you want to figure out why linting or tidying is slower than
  expected.
- Fixed a bug in the debug output. It was not showing the correct cwd for commands where
  `chdir = true` was set. It always showed the project root directory instead of the directory where
  the command was run. It _was_ running these commands in the right directory. This was solely a bug
  in the debug output.

## 0.0.11 - 2021-02-20

- Fixed a bug in 0.0.10 where when _not_ running with `--debug`, precious would not honor the
  `expect_stderr = true` configuration, and would instead unconditionally treat stderr output as an
  error.

## 0.0.10 - 2021-02-20

- Errors are now printed out a bit differently, and in particular errors when trying to execute a
  command (not in the path, command fails unexpectedly, etc.) should be more readable now.
- When running any commands, precious now explicitly checks to see if the executable is in your
  `PATH`. If it's not it prints a new error for this case, as opposed to when running the executable
  produces an error. This partially addresses #10.

## 0.0.9 - 2021-02-12

- Added a --jobs (-j) option for all subcommands. This lets you limit how many parallel threads are
  run. The default is to run one thread per available core. Requested by Shane Warden. GH #7.
- Fixed a bug where running precious in "git staged mode" (`precious lint --staged`) would cause
  breakage with merge commits that were the result of resolving a merge conflict. Basically, you'd
  get the commit but git would no longer know it was merging a commit, because precious was running
  `git stash` under the hood to only check the staged files, then `git stash pop` to restore things
  back to their original state. But running `git stash` command. There's some discussion of this on
  [Stack Overflow](https://stackoverflow.com/questions/24637571/merge-status-lost-when-stashing) but
  apparently it's still an issue with git today. Reported by Carey Hoffman. GH #9.

## 0.0.8 - 2021-01-30

- Added a summary when there are problems linting or tidying files. Previously, when there were any
  errors, the last thing precious printed out would be something like
  `[precious::precious][ERROR] Error when linting files`. Now it also includes a summary of what
  filter failed on each path. This is primarily useful for linting, as tidy failures are typically
  failures to execute the tidy command.

## 0.0.7 - 2021-01-02

- Look for a `precious.toml` file in the current directory before trying to find one in the root of
  the current VCS checkout.
- Use the current directory as the root for finding files, rather than the VCS checkout root.
- When a filter command does not exist, the error output now shows the full command that was run,
  including any arguments. Fixes GH #6.

## 0.0.6 - 2020-08-01

- Precious can now be run outside of a VCS repo, as long as there is a `precious.toml` file in the
  current directory. There is probably more work to be done for precious to not expect to be run
  inside a VCS repo.
- Fixed a bug where lint failures would still result in precious exiting with 0. I'm not sure when
  this bug was introduced.
- Replaced deprecated failure and failure_derive crates with anyhow and thiserror.
- Replaced the `on_dir` and `run_once` config flags with a single `run_mode` flag, which can be one
  of "files" (the default)", "dirs", or "root". If the mode is "root" then the command runs exactly
  once from the root of the project.
- Added an `env` config key for filters. This allows you to define env vars that will be set when
  the filter's command is run.

## 0.0.5 - 2019-09-05

- Renamed the config key `lint_flag` to `lint_flags` so it can now be an array of strings as well as
  a single string.
- Added a `tidy_flags` option as well. Now commands which are both must define either `lint_flags`
  or `tidy_flags` (or both).

## 0.0.4 - 2019-08-31

- Fixed a bug where `git stash` would be run multiple times in staged mode if you had more than one
  filter defined. As a bonus this also makes precious more efficient by not retrieving the list of
  files to check more than once.

## 0.0.3 - 2019-08-31

- Add a `run_once` flag for commands. This causes command to be run exactly once from the root
  directory of the project (when it applies). This lets you set your `include` and `exclude` rules
  based on files properly. Previously you would have to set `include = "."` and the command would
  run when any files changed, even files which shouldn't trigger a run.
- Fixed a bug where a command with `on_dir` set to `true` would incorrectly be run when a file
  matched both an include _and_ exclude rule. Exclude rules should always win in these situations.

## 0.0.2 - 2019-08-13

- Documentation fixes

## 0.0.1 - 2019-08-13

- First release upon an unsuspecting world.
