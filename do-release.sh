#!/bin/bash

cargo release --package precious-exec $@
cargo release --package precious-testhelper $@
cargo release --package precious-core $@
cargo release --package precious-integration $@
cargo release --package precious $@
