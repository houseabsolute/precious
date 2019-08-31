#!/bin/bash

set -e

go mod tidy
STATUS=$( git status --porcelain go.mod go.sum )
if [ ! -z "$STATUS" ]; then
    echo "Running go mod tidy modified go.mod and/or go.sum"
    exit 1
fi

exit 0
