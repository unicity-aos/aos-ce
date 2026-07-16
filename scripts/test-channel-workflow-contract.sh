#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
workflow="$repo_root/.github/workflows/promote-channel.yml"

grep -Fq "if: github.ref == 'refs/heads/main'" "$workflow"
grep -Fq "repos/\$GITHUB_REPOSITORY/git/ref/tags/\$RELEASE_TAG" "$workflow"
grep -Fq "repos/\$GITHUB_REPOSITORY/git/tags/\$TAG_COMMIT" "$workflow"
grep -Fq "[[ \"\$TAG_TYPE\" == commit ]]" "$workflow"

if grep -Fq "repos/\$GITHUB_REPOSITORY/commits/\$RELEASE_TAG" "$workflow"; then
  echo "channel promotion resolves an ambiguous branch-or-tag revision" >&2
  exit 1
fi
