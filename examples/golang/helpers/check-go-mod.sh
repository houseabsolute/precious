#!/bin/bash

set -e

ROOT=$( git rev-parse --show-toplevel )
BEFORE_MOD=$( md5sum "$ROOT/go.mod" )
BEFORE_SUM=$( md5sum "$ROOT/go.sum" )

OUTPUT=$( go mod tidy -v 2>&1 )

AFTER_MOD=$( md5sum "$ROOT/go.mod" )
AFTER_SUM=$( md5sum "$ROOT/go.sum" )

red=$'\e[1;31m'
end=$'\e[0m'

if [ "$BEFORE_MOD" != "$AFTER_MOD" ]; then
    printf "${red}Running go mod tidy changed the contents of go.mod${end}\n"
    git diff "$ROOT/go.mod"
    changed=1
fi

if [ "$BEFORE_SUM" != "$AFTER_SUM" ]; then
    printf "${red}Running go mod tidy changed the contents of go.sum${end}\n"
    git diff "$ROOT/go.sum"
    changed=1
fi

if [ -n "$changed" ]; then
    if [ -n "$OUTPUT" ]; then
        printf "\nOutput from running go mod tidy -v:\n${OUTPUT}\n"
    else
        printf "\nThere was no output from running go mod tidy -v\n\n"
    fi

    exit 1
fi

exit 0
