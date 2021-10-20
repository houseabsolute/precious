## 0.1.3

* Relaxed some dependencies for the benefit of packaging precious for
  Debian. Implemented by Jonas Smedegaard.


## 0.1.2 - 2021-10-14

* The order of commands in the config file is now preserved, and commands are
  executed in the order in which they appear in the config file. This
  addresses #12, requested by Olaf Alders.

* Fixed the tests so that they set the default branch name when running `git
  init`, rather than setting this via `git config`. This lets anyone run the
  tests, whereas it was only safe to set this via `git config` in CI. This
  fixes #14, reported by Olaf Alders.


## 0.1.1 - 2021-07-12

* Fixed config handling of a global `exclude` key. The previous release did
  not handle a single string as that key's value, only an array.


## 0.1.0 - 2021-07-02

* The verbose and debugging level output now includes timing information on
  each linter and tidier that is run. This is helpful if you want to figure
  out why linting or tidying is slower than expected.

* Fixed a bug in the debug output. It was not showing the correct cwd for
  commands where `chdir = true` was set. It always showed the project root
  directory instead of the directory where the command was run. It _was_
  running these commands in the right directory. This was solely a bug in the
  debug output.


## 0.0.11 - 2021-02-20

* Fixed a bug in 0.0.10 where when *not* running with `--debug`, precious
  would not honor the `expect_stderr = true` configuration, and would instead
  unconditionally treat stderr output as an error.


## 0.0.10 - 2021-02-20

* Errors are now printed out a bit differently, and in particular errors when
  trying to execute a command (not in the path, command fails unexpectedly,
  etc.) should be more readable now.

* When running any commands, precious now explicitly checks to see if the
  executable is in your `PATH`. If it's not it prints a new error for this
  case, as opposed to when running the executable produces an error. This
  partially addresses #10.


## 0.0.9 - 2021-02-12

* Added a --jobs (-j) option for all subcommands. This lets you limit how many
  parallel threads are run. The default is to run one thread per available
  core. Requested by Shane Warden. GH #7.

* Fixed a bug where running precious in "git staged mode" (`precious lint
  --staged`) would cause breakage with merge commits that were the result of
  resolving a merge conflict. Basically, you'd get the commit but git would no
  longer know it was merging a commit, because precious was running `git
  stash` under the hood to only check the staged files, then `git stash pop`
  to restore things back to their original state. But runnin`git stash`
  command. There's some discussion of this on [Stack
  Overflow](https://stackoverflow.com/questions/24637571/merge-status-lost-when-stashing)
  but apparently it's still an issue with git today. Reported by Carey
  Hoffman. GH #9.


## 0.0.8 - 2021-01-30

* Added a summary when there are problems linting or tidying
  files. Previously, when there were any errors, the last thing precious
  printed out would be something like "[precious::precious][ERROR] Error when
  linting files". Now it also includes a summary of what filter failed on each
  path. This is primarily useful for linting, as tidy failures are typically
  failures to execute the tidy command.


## 0.0.7 - 2021-01-02

* Look for a `precious.toml` file in the current directory before trying to
  find one in the root of the current VCS checkout.

* Use the current directory as the root for finding files, rather than the VCS
  checkout root.

* When a filter command does not exist, the error output now shows the full
  command that was run, including any arguments. Fixes GH #6.


## 0.0.6 - 2020-08-01

* Precious can now be run outside of a VCS repo, as long as there is a
  `precious.toml` file in the current directory. There is probably more work
  to be done for precious to not expect to be run inside a VCS repo.

* Fixed a bug where lint failures would still result in precious exiting
  with 0. I'm not sure when this bug was introduced.

* Replaced deprecated failure and failure_derive crates with anyhow and
  thiserror.

* Replaced the `on_dir` and `run_once` config flags with a single `run_mode`
  flag, which can be one of "files" (the default)", "dirs", or "root". If the
  mode is "root" then the command runs exactly once from the root of the
  project.

* Added an `env` config key for filters. This allows you to define env vars
  that will be set when the filter's command is run.


## 0.0.5 - 2019-09-05

* Renamed the config key `lint_flag` to `lint_flags` so it can now be an array
  of strings as well as a single string.

* Added a `tidy_flags` option as well. Now commands which are both must define
  either `lint_flags` or `tidy_flags` (or both).


## 0.0.4 - 2019-08-31

* Fixed a bug where `git stash` would be run multiple times in staged mode if
  you had more than one filter defined. As a bonus this also makes precious
  more efficient by not retrieving the list of files to check more than once.


## 0.0.3 - 2019-08-31

* Add a `run_once` flag for commands. This causes command to be run exactly
  once from the root directory of the project (when it applies). This lets you
  set your `include` and `exclude` rules based on files properly. Previously
  you would have to set `include = "."` and the command would run when any
  files changed, even files which shouldn't trigger a run.

* Fixed a bug where a command with `on_dir` set to `true` would incorrectly be
  run when a file matched both an include _and_ exclude rule. Exclude rules
  should always win in these situations.


## 0.0.2 - 2019-08-13

* Documentation fixes


## 0.0.1 - 2019-08-13

* First release upon an unsuspecting world.
