_dce := "devcontainer exec --workspace-folder ."
# When we're in a git worktree, the workspace's .git is a file pointing at a
# gitdir outside the workspace, so git doesn't work in the container unless we
# also mount the main repo's git dir at the same path. Note that `devcontainer
# up` reuses an existing container without comparing mounts, so a container
# created before this mount existed needs a `just rebuild` once.
_git_common_dir := `test -f .git && realpath "$(git rev-parse --git-common-dir)" || true`
_git_mount := if _git_common_dir != "" { "--mount 'type=bind,source=" + _git_common_dir + ",target=" + _git_common_dir + "'" } else { "" }

_up:
    devcontainer up --workspace-folder . {{ _git_mount }}

rebuild:
    devcontainer up --workspace-folder . {{ _git_mount }} --remove-existing-container

shell: _up
    {{ _dce }} bash -i

# Set RUST_LOG in the environment to turn on logging, e.g.
# `RUST_LOG=debug just test`. It can't be a recipe parameter, because just
# binds parameters positionally, so the first argument would always be taken
# as the log level instead of being passed on to cargo.
test *args: _up
    {{ _dce }} \
      {{ if env("RUST_LOG", "") != "" { "--remote-env RUST_LOG=" + env("RUST_LOG", "") } else { "" } }} \
      cargo test {{ args }}

lint *args: _up
    {{ _dce }} mise exec -- precious lint {{ args }}

tidy *args: _up
    {{ _dce }} mise exec -- precious tidy {{ args }}

# Cut a release. The level is anything cargo-release accepts, so "patch", "minor", "major", or an
# explicit version like "0.12.0". This bumps the version everywhere, stamps the "NEXT" section in
# Changes.md with the version and date, commits, tags, and pushes. Pushing the tag is what makes
# CI build the binaries, publish the crates to crates.io, and draft the GitHub release.
#
# Unlike the other recipes, cargo-release runs on the host rather than in the dev container,
# because the commit and tag are signed and the signing key lives outside the container.
release level: (test "--workspace --locked") (lint "-a")
    mise exec -- cargo-release release {{ level }} --workspace --execute
