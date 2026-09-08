#!/bin/sh
# Product-facing installation name; signed Astrid identities remain unchanged.
set -eu
manager_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
if [ -z "${ASTRID_FSKIT_APP_DEST:-}" ]; then
  # Reuse the existing provider rather than register its identity twice.
  ASTRID_FSKIT_APP_DEST=/Applications/AOS.app
  if [ -e /Applications/AstridFS.app ]; then
    if [ -e /Applications/AOS.app ]; then
      echo 'Both AOS.app and AstridFS.app exist; select the managed provider with ASTRID_FSKIT_APP_DEST.' >&2
      exit 1
    fi
    ASTRID_FSKIT_APP_DEST=/Applications/AstridFS.app
  fi
fi
ASTRID_FSKIT_BIN_DIR=${ASTRID_FSKIT_BIN_DIR:-"$manager_dir/.."}
export ASTRID_FSKIT_APP_DEST ASTRID_FSKIT_BIN_DIR
case "${1:-}" in
  install|update)
    if [ -e "$ASTRID_FSKIT_APP_DEST" ] || [ -L "$ASTRID_FSKIT_APP_DEST" ]; then
      /bin/bash "$manager_dir/manage-macos-fskit.sh" validate
    fi
    ;;
esac
exec /bin/bash "$manager_dir/manage-macos-fskit.sh" "$@"
