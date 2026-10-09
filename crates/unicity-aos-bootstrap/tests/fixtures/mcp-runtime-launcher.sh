#!/bin/sh
# Test-only interpreter dispatch; never part of a product archive.
set -eu
program=${AOS_TEST_RUNTIME_PROGRAM:?missing test runtime program}
IFS= read -r shebang < "$program"
case "$shebang" in
    '#!/bin/sh') exec /bin/sh "$program" "$@" ;;
    '#!/usr/bin/env python3') exec /usr/bin/env python3 "$program" "$@" ;;
    *) printf '%s\n' 'unsupported test runtime interpreter' >&2; exit 97 ;;
esac
