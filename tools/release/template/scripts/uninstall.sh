#!/bin/sh
set -eu
cd -- "$(dirname "$0")/.."
. ./scripts/lib.sh

choose_prefix
BINARY=$(binary_name)
run rm -f "$PREFIX/bin/$BINARY"
