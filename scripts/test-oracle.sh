#!/bin/sh
# Oracle test tier (docs/FIXTURES.md): regenerate stale Go-oracle fixtures
# into oracle-cache/fixtures, then run the workspace tests with every crate's
# `oracle-fixtures` feature enabled. Arguments are passed to `cargo test`.
#
#   scripts/test-oracle.sh                      # all oracle-tier tests
#   scripts/test-oracle.sh -p d2-rs-text        # one crate (still regenerates all)
#   D2RS_REGEN_ARGS="--only 'goldmark-*' --jobs 4" scripts/test-oracle.sh -p d2-rs-text
set -eu
repo_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo_root"

# shellcheck disable=SC2086 # D2RS_REGEN_ARGS is intentionally word-split.
eval "uv run --no-project python scripts/regen-fixtures.py ${D2RS_REGEN_ARGS:---jobs 4}"

oracle_features=""
for manifest_path in crates/*/Cargo.toml vendor/*/Cargo.toml; do
    if grep -q '^oracle-fixtures *=' "$manifest_path"; then
        crate_name=$(sed -n 's/^name = "\(.*\)"$/\1/p' "$manifest_path" | head -n 1)
        oracle_features="${oracle_features}${oracle_features:+,}${crate_name}/oracle-fixtures"
    fi
done
if [ -z "$oracle_features" ]; then
    echo "no crate declares an oracle-fixtures feature" >&2
    exit 1
fi

if [ "$#" -eq 0 ]; then
    set -- --workspace
fi
exec cargo test "$@" --features "$oracle_features"
