#!/bin/sh
printf 'bigstart'
head -c 4097 /dev/zero | tr '\000' x
