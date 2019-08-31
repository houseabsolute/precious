#!/bin/bash

set -eo pipefail

function run () {
    echo $1
    eval $1
}

function install_tools () {
    run "./dev/bin/download-precious.sh"
    run "rustup component add clippy"
}

if [ "$1" == "-v" ]; then
    set -x
fi

mkdir -p bin
install_tools

exit 0
