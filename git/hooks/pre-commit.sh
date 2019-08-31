#!/bin/bash

status=0

./bin/precious lint -s
if (( $? != 0 )); then
    status+=1
fi

exit $status
