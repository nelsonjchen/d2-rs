# Shared by install.sh and uninstall.sh, following the layout of Go D2's
# release archives (ci/release/template/scripts in terrastruct/d2).

run() {
  echo "$*" >&2
  if [ -z "${DRY_RUN-}" ]; then
    "$@"
  fi
}

choose_prefix() {
  if [ -n "${PREFIX-}" ]; then
    return
  fi
  if [ -w /usr/local ] || { [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; }; then
    PREFIX=/usr/local
  else
    PREFIX=$HOME/.local
  fi
}

binary_name() {
  if [ -e ./bin/d2-rs.exe ]; then
    echo d2-rs.exe
  else
    echo d2-rs
  fi
}
