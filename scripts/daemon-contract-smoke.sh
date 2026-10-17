#!/usr/bin/env bash
# Runs the image's own mostrod, with no network, against what the Manager
# would give it, and against databases older releases left behind:
#   - the settings rendered from the test rules and from each preset must
#     load ("Settings correctly loaded") and create the database;
#   - each database in scripts/fixtures/mostro-db/ must migrate: every
#     migration applied and successful, integrity intact, no row lost.
# mostrod then stops when it cannot reach LND, which is expected here.
# This catches what the Rust tests cannot: a setting the daemon rejects, a
# new startup check, a migration that fails on existing data.
#
# Usage: scripts/daemon-contract-smoke.sh IMAGE
set -euo pipefail
IMAGE="${1:?usage: daemon-contract-smoke.sh IMAGE}"
cd "$(dirname "$0")/.."
work="$(mktemp -d)"
owner="$(id -u):$(id -g)"
cleanup() {
  docker run --rm --user 0:0 --entrypoint sh -v "$work:/w" "$IMAGE" -c 'chown -R "$1" /w' sh "$owner" >/dev/null 2>&1 || true
  rm -rf -- "$work"
}
trap cleanup EXIT

# LND files only have to exist: the daemon stops when it tries to connect.
mkdir "$work/lnd"
printf 'not a certificate\n' >"$work/lnd/tls.cert"
printf 'not a macaroon' >"$work/lnd/mostro.macaroon"

render() { # render SOURCE DIR: settings.toml for rules (a file under api/tests/fixtures) or preset:ID
  mkdir "$2"
  docker run --rm --network none -v "$PWD/api/tests/fixtures:/fixtures:ro" "$IMAGE" \
    render-settings "$1" >"$2/settings.toml"
}

run_daemon() { # run_daemon DIR: runs mostrod in DIR, keeps its log in DIR/log
  docker run --rm --user 0:0 --entrypoint sh -v "$1:/s" "$IMAGE" -c 'chown -R 1000:1000 /s' >/dev/null
  docker run --rm --network none --read-only --tmpfs /tmp --cap-drop ALL \
    --security-opt no-new-privileges -e TERM=xterm \
    -e MOSTRO_NSEC_PRIVKEY="$(cat scripts/fixtures/throwaway.nsec)" \
    -v "$1:/s" -v "$work/lnd:/lnd:ro" --entrypoint timeout "$IMAGE" 90 mostrod -d /s \
    >"$work/daemon.log" 2>&1 || true
  sed 's/\x1b\[[0-9;]*[a-zA-Z]//g' "$work/daemon.log" >"$work/$(basename "$1").log"
}

check_loaded() { # check_loaded NAME DIR
  local log="$work/$(basename "$2").log"
  if ! grep -q "Settings correctly loaded" "$log" \
     || grep -E -q "Could not load settings|REFUSING TO START|missing field|invalid type|unknown variant|unknown field|Failed to run migrations|DbAccessError" "$log"; then
    cat "$log" >&2
    echo "FAIL $1: mostrod refused these settings or this database" >&2
    exit 1
  fi
  docker run --rm --entrypoint test -v "$2:/s:ro" "$IMAGE" -s /s/mostro.db \
    || { echo "FAIL $1: no database created" >&2; exit 1; }
  echo "ok $1: settings loaded"
}

version="$(docker run --rm --entrypoint mostrod "$IMAGE" --version 2>/dev/null | sed 's/\x1b\[[0-9;]*[a-zA-Z]//g' | tail -n 1)"
echo "$version"

for source in /fixtures/community.json preset:latam preset:europe preset:global; do
  case="$work/case-$(echo "$source" | tr -c 'a-z0-9\n' '-')"
  render "$source" "$case"
  run_daemon "$case"
  check_loaded "$source" "$case"
done

# The migrations a fresh database gets are the ones every old one must reach.
fresh="$work/case--fixtures-community-json"
docker run --rm --user 0:0 --entrypoint sh -v "$fresh:/s" "$IMAGE" -c 'chmod 0644 /s/mostro.db' >/dev/null
cp "$fresh/mostro.db" "$work/fresh.db"

for fixture in scripts/fixtures/mostro-db/*.db; do
  [ -e "$fixture" ] || continue
  name="$(basename "$fixture" .db)"
  case="$work/case-db-$name"
  render /fixtures/community.json "$case"
  cp "$fixture" "$case/mostro.db"
  run_daemon "$case"
  check_loaded "$name" "$case"
  docker run --rm --user 0:0 --entrypoint sh -v "$case:/s" "$IMAGE" -c 'chmod 0644 /s/mostro.db' >/dev/null
  python3 - "$fixture" "$case/mostro.db" "$work/fresh.db" "$name" <<'PY'
import sqlite3
import sys

before_path, after_path, fresh_path, name = sys.argv[1:]
before = sqlite3.connect(f"file:{before_path}?mode=ro", uri=True)
after = sqlite3.connect(f"file:{after_path}?mode=ro", uri=True)
fresh = sqlite3.connect(f"file:{fresh_path}?mode=ro", uri=True)
problems = []
applied = {row[0] for row in after.execute("select version from _sqlx_migrations where success = 1")}
failed = after.execute("select count(*) from _sqlx_migrations where success = 0").fetchone()[0]
expected = {row[0] for row in fresh.execute("select version from _sqlx_migrations")}
if failed:
    problems.append(f"{failed} failed migrations")
if missing := expected - applied:
    problems.append(f"migrations not applied: {sorted(missing)}")
integrity = after.execute("pragma integrity_check").fetchone()[0]
if integrity != "ok":
    problems.append(f"integrity: {integrity}")
tables = [row[0] for row in before.execute(
    "select name from sqlite_master where type = 'table' and name not like 'sqlite_%'")]
for table in tables:
    rows_before = before.execute(f'select count(*) from "{table}"').fetchone()[0]
    try:
        rows_after = after.execute(f'select count(*) from "{table}"').fetchone()[0]
    except sqlite3.Error:
        rows_after = 0
    if table != "_sqlx_migrations" and rows_after < rows_before:
        problems.append(f"{table}: {rows_before} rows before, {rows_after} after")
if problems:
    sys.exit(f"FAIL {name}: " + "; ".join(problems))
old = before.execute("select count(*) from _sqlx_migrations").fetchone()[0]
print(f"ok {name}: {old} -> {len(applied)} migrations, integrity ok, no rows lost")
PY
done
echo "Daemon contract smoke passed"
