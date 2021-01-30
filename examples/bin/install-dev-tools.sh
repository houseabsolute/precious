#!/bin/bash

set -eo pipefail

function run () {
    echo $1
    eval $1
}

function install_any_project_tools () {
    curl --silent --location \
        https://raw.githubusercontent.com/houseabsolute/ubi/master/bootstrap/bootstrap-ubi.sh |
        sh
    run "ubi --project houseabsolute/precious --in ~/bin"
    run "ubi --project houseabsolute/omegasort --in ~/bin"
}

function install_go_project_tools () {
    run "ubi --project golangci/golangci-lint --in ~/bin"
    # If we run this in the checkout dir it can mess with out go.mod and
    # go.sum.
    pushd /tmp
    # This will end up in $GOBIN, which defaults to $HOME/go/bin.
    run "go get golang.org/x/tools/cmd/goimports"
    popd
}

function install_rust_project_tools () {
    run "rustup component add clippy"
}

if [ "$1" == "-v" ]; then
    set -x
fi

install_any_project_tools
install_go_project_tools
install_rust_project_tools

# For Perl, you would generally expect to have a cpanfile in the project root
# that included the relevant develop prereqs, so developers could just run
# `cpanm --installdeps --with-develop .`

exit 0
