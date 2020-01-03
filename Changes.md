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
