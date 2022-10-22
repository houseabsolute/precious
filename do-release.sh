#!/bin/bash

cargo release --package precious-helpers $@
cargo release --package precious-testhelper $@
cargo release --package precious-core $@
cargo release --package precious-integration $@
cargo release --package precious $@
