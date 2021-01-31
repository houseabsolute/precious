#!/bin/bash

set -e

# This seems to be just enough to force a recompilation, so clippy is actually
# executed. But it doesn't require rebuilding every dep, so it's pretty fast.
#
# You'll need to change this to delete the right file for your project.
rm -fr target/debug/deps/*precious*
cargo clippy --all-targets --all-features -- -D clippy::all
