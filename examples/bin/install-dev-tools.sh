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

mkdir -p $HOME/bin

set +e
echo ":$PATH:" | grep --extended-regexp ":$HOME/bin:" >& /dev/null
if [ "$?" -eq "0" ]; then
    path_has_home_bin=1
fi
set -e

if [ -z "$path_has_home_bin" ]; then
    PATH=$HOME/bin:$PATH
fi

install_any_project_tools
install_go_project_tools
install_rust_project_tools

echo "Tools were installed into $HOME/bin."
if [ -z "$path_has_home_bin" ]; then
     echo "You should add $HOME/bin to your PATH."
fi

# For Perl, you would generally expect to have a cpanfile in the project root
# that included the relevant develop prereqs, so developers could just run
# `cpanm --installdeps --with-develop .`

exit 0
