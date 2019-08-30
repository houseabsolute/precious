## 0.0.3

* Add a `run_once` flag for commands. This causes command to be run exactly
  once from the root directory of the project (when it applies). This lets you
  set your `include` and `exclude` rules based on files properly. Previously
  you would have to set `include = "."` and the command would run when any
  files changed, even files which shouldn't trigger a run.

* Fixed a bug where a command with `on_dir` set to `true` would incorrectly be
  run when it a file matched both an include _and_ exclude rule. Exclude rules
  should always win in these situations.


## 0.0.2

* Documentation fixes


## 0.0.1

* First release upon an unsuspecting world.
