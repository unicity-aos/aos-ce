#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT HUP INT TERM
AOS_HOME="$work/home"
staged_version=2026.9.3
mkdir -p "$AOS_HOME/update" "$AOS_HOME/releases" "$work/external"

# Exercise the installer implementation, not a copied cleanup algorithm.
eval "$(sed -n '/^reclaim_abandoned_release_transactions() {$/,/^}$/p' "$repo_root/install.sh")"
eval "$(sed -n '/^acquire_install_lock() {$/,/^}$/p' "$repo_root/install.sh")"
eval "$(sed -n '/^release_install_lock() {$/,/^}$/p' "$repo_root/install.sh")"
# Read by the installer functions loaded above.
# shellcheck disable=SC2034
install_lock_acquired=0
install_lock="$AOS_HOME/update/install.lock"

record() {
  mkdir "$AOS_HOME/update/install.lock.stale.$1"
  printf '%s\n' "$2" > "$AOS_HOME/update/install.lock.stale.$1/pid"
  if [ "$#" -eq 3 ]; then
    printf '%s\n' "$3" > "$AOS_HOME/update/install.lock.stale.$1/version"
  fi
}

dead_owner=999999999
if kill -0 "$dead_owner" 2>/dev/null; then
  echo 'fixture PID unexpectedly alive' >&2
  exit 1
fi
record 1 "$dead_owner"
mkdir "$AOS_HOME/releases/.${staged_version}.new.$dead_owner"
mkdir "$AOS_HOME/releases/.${staged_version}.rollback.$dead_owner"
mkdir "$AOS_HOME/releases/.2026.9.1.rollback.$dead_owner"
record 2 "$$" "$staged_version"
mkdir "$AOS_HOME/releases/.${staged_version}.rollback.$$"
record 3 "$dead_owner" 2026.9.2
mkdir "$AOS_HOME/releases/.2026.9.2.rollback.$dead_owner"
mkdir "$AOS_HOME/releases/.${staged_version}.new.999999998"

reclaim_abandoned_release_transactions
test ! -e "$AOS_HOME/releases/.${staged_version}.new.$dead_owner"
test ! -e "$AOS_HOME/releases/.${staged_version}.rollback.$dead_owner"
test -d "$AOS_HOME/update/install.lock.stale.1"
test -d "$AOS_HOME/releases/.2026.9.1.rollback.$dead_owner"
test -d "$AOS_HOME/releases/.${staged_version}.rollback.$$"
test -d "$AOS_HOME/releases/.2026.9.2.rollback.$dead_owner"
test -d "$AOS_HOME/update/install.lock.stale.3"
test -d "$AOS_HOME/releases/.${staged_version}.new.999999998"

record 4 "$dead_owner" "$staged_version"
ln -s "$work/external" "$AOS_HOME/releases/.${staged_version}.new.$dead_owner"
reclaim_abandoned_release_transactions
test -L "$AOS_HOME/releases/.${staged_version}.new.$dead_owner"
test -d "$work/external"
test -d "$AOS_HOME/update/install.lock.stale.4"

# Recovering the lock is not a commit: the predecessor's only backup must
# survive a failed retry, including release of the new attempt's lock.
rm "$AOS_HOME/releases/.${staged_version}.new.$dead_owner"
mkdir "$AOS_HOME/update/install.lock"
printf '%s\n' "$dead_owner" > "$AOS_HOME/update/install.lock/pid"
printf '%s\n' "$staged_version" > "$AOS_HOME/update/install.lock/version"
mkdir "$AOS_HOME/releases/.${staged_version}.rollback.$dead_owner"
acquire_install_lock
test -d "$AOS_HOME/releases/.${staged_version}.rollback.$dead_owner"
test -f "$AOS_HOME/update/install.lock.stale.$$/pid"
test "$(cat "$install_lock/version")" = "$staged_version"
release_install_lock
test -d "$AOS_HOME/releases/.${staged_version}.rollback.$dead_owner"
test -d "$AOS_HOME/update/install.lock.stale.$$"
acquire_install_lock
rmdir "$AOS_HOME/releases/.2026.9.1.rollback.$dead_owner"
rmdir "$AOS_HOME/releases/.2026.9.2.rollback.$dead_owner"
reclaim_abandoned_release_transactions
test ! -e "$AOS_HOME/releases/.${staged_version}.rollback.$dead_owner"
release_install_lock

# Abruptly kill an owned process after it has acquired the real lock and
# created transaction directories. This checks lock recovery, not package
# verification or a power-loss boundary.
sh -c '
  set -eu
  repo_root=$1 AOS_HOME=$2 staged_version=$3
  install_lock_acquired=0
  eval "$(sed -n '\''/^acquire_install_lock() {$/,/^}$/p'\'' "$repo_root/install.sh")"
  acquire_install_lock
  mkdir "$AOS_HOME/releases/.${staged_version}.new.$$"
  mkdir "$AOS_HOME/releases/.${staged_version}.rollback.$$"
  : > "$AOS_HOME/update/interrupted-owner-ready"
  sleep 20
' sh "$repo_root" "$AOS_HOME" "$staged_version" &
interrupted_pid=$!
ready=0
attempt=0
while [ "$attempt" -lt 5 ]; do
  attempt=$((attempt + 1))
  if [ -f "$AOS_HOME/update/interrupted-owner-ready" ]; then
    ready=1
    break
  fi
  sleep 1
done
if [ "$ready" -ne 1 ]; then
  kill "$interrupted_pid" 2>/dev/null || :
  wait "$interrupted_pid" 2>/dev/null || :
  echo 'interrupted installer fixture did not acquire its lock' >&2
  exit 1
fi
# Capture the owned wait child before killing the installer owner.
wait_pids=$(ps -axo pid,ppid | awk -v parent="$interrupted_pid" '$2 == parent { print $1 }')
kill -9 "$interrupted_pid"
for wait_pid in $wait_pids; do
  kill "$wait_pid" 2>/dev/null || :
done
wait "$interrupted_pid" 2>/dev/null || :
acquire_install_lock
test -d "$AOS_HOME/releases/.${staged_version}.rollback.$interrupted_pid"
reclaim_abandoned_release_transactions
test ! -e "$AOS_HOME/releases/.${staged_version}.new.$interrupted_pid"
test ! -e "$AOS_HOME/releases/.${staged_version}.rollback.$interrupted_pid"
release_install_lock
echo 'abandoned installer cleanup ownership checks PASS'
