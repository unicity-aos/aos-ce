#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
workflow="$repo_root/.github/workflows/promote-channel.yml"

grep -Fq "if: github.ref == 'refs/heads/main'" "$workflow"
grep -Fq "repos/\$GITHUB_REPOSITORY/git/ref/tags/\$RELEASE_TAG" "$workflow"
grep -Fq "repos/\$GITHUB_REPOSITORY/git/tags/\$TAG_COMMIT" "$workflow"
grep -Fq "[[ \"\$TAG_TYPE\" == commit ]]" "$workflow"
grep -Fq "TRANSACTION=\"channel-\${CHANNEL}-\${GENERATION}.transaction.json\"" "$workflow"
grep -Fq "cmp \"\$TRANSACTION\" \"existing-transaction/\$TRANSACTION\"" "$workflow"
grep -Fq 'python3 scripts/channel_transaction.py unpack' "$workflow"
grep -Fq -- "--now \"\$NOW\"" "$workflow"
grep -Fq "cp transaction-floor-pair/channel.toml \"\$FLOOR_HISTORY\"" "$workflow"
grep -Fq "history generation \$history_floor has no authenticated transaction" "$workflow"

transaction_upload=$(grep -nF "\"\$TRANSACTION\"" "$workflow" | tail -n1 | cut -d: -f1)
history_upload=$(grep -nF "\"\$HISTORY\"" "$workflow" | tail -n1 | cut -d: -f1)
[[ "$transaction_upload" -lt "$history_upload" ]]

if grep -Fq "repos/\$GITHUB_REPOSITORY/commits/\$RELEASE_TAG" "$workflow"; then
  echo "channel promotion resolves an ambiguous branch-or-tag revision" >&2
  exit 1
fi
