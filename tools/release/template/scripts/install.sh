#!/bin/sh
set -eu
cd -- "$(dirname "$0")/.."
. ./scripts/lib.sh

choose_prefix
BINARY=$(binary_name)
run mkdir -p "$PREFIX/bin"
run install ./bin/"$BINARY" "$PREFIX/bin/$BINARY"
echo "installed $PREFIX/bin/$BINARY" >&2
