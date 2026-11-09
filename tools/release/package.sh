#!/bin/sh
# Packages one d2-rs release archive laid out like Go D2's
# (d2-<VERSION>-<OS>-<ARCH>.tar.gz with a d2-<VERSION>/ root), with d2-rs in
# place of d2:
#
#   d2-rs-<VERSION>/bin/d2-rs[.exe]
#   d2-rs-<VERSION>/LICENSE, THIRD_PARTY_NOTICES.md, README.md
#   d2-rs-<VERSION>/Makefile, scripts/{install,uninstall,lib}.sh
#
# usage: package.sh VERSION OS ARCH BINARY OUTDIR
# Needs GNU tar and gzip; archives are reproducible for a given
# SOURCE_DATE_EPOCH (default: the HEAD commit time).
set -eu
VERSION=$1 OS=$2 ARCH=$3 BINARY=$4 OUTDIR=$5
REPO=$(cd -- "$(dirname "$0")/../.." && pwd)
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git -C "$REPO" show -s --format=%ct HEAD)}

case "$OS" in
  windows) EXE=.exe ;;
  *) EXE= ;;
esac

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
ROOT=$STAGE/d2-rs-$VERSION
mkdir -p "$ROOT/bin" "$ROOT/scripts"
install -m 0755 "$BINARY" "$ROOT/bin/d2-rs$EXE"
install -m 0644 "$REPO/LICENSE" "$REPO/THIRD_PARTY_NOTICES.md" "$ROOT/"
install -m 0644 "$REPO/tools/release/template/Makefile" "$ROOT/"
install -m 0644 "$REPO/tools/release/template/scripts/lib.sh" "$ROOT/scripts/"
install -m 0755 "$REPO/tools/release/template/scripts/install.sh" \
  "$REPO/tools/release/template/scripts/uninstall.sh" "$ROOT/scripts/"

{
  cat <<README
# d2-rs

d2-rs is a Rust port of D2, the diagram language, with output byte-identical
to Go D2. The command is \`d2-rs\` and takes the same subcommands and flags as
Go's \`d2\`. For docs and the source code see https://github.com/nelsonjchen/d2-rs

version: $VERSION
os: $OS
arch: $ARCH
README
  if [ "$OS" = windows ]; then
    cat <<'README'

This archive has the same layout as the Unix archives. Put `bin\d2-rs.exe`
on your `PATH`, or use `make install` under MSYS2.
README
  fi
  cat <<'README'

## Install

```sh
make install DRY_RUN=1
# If it looks right, run:
# make install
```

Pass `PREFIX=somepath` to change the installation prefix from the default,
which is `/usr/local`, or `~/.local` if `/usr/local` is not writable.

## Uninstall

```sh
make uninstall DRY_RUN=1
# If it looks right, run:
# make uninstall
```
README
} >"$ROOT/README.md"

mkdir -p "$OUTDIR"
ARCHIVE=$(cd -- "$OUTDIR" && pwd)/d2-rs-$VERSION-$OS-$ARCH.tar.gz
tar -C "$STAGE" --sort=name --mtime="@$SOURCE_DATE_EPOCH" \
  --owner=0 --group=0 --numeric-owner --format=gnu \
  -cf - "d2-rs-$VERSION" | gzip -9n >"$ARCHIVE"
echo "$ARCHIVE"
