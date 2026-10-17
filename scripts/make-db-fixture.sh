#!/usr/bin/env bash
# Builds the mostro.db an older official mostrod release leaves behind, for
# scripts/daemon-contract-smoke.sh: runs that release's binary with its own
# settings template and no network (it creates the schema, then fails to
# reach LND), and adds two synthetic rows to every table.
#
# Usage: scripts/make-db-fixture.sh VERSION AMD64_SHA256
# Writes scripts/fixtures/mostro-db/mostro-VERSION.db
set -euo pipefail
version="${1:?usage: make-db-fixture.sh VERSION AMD64_SHA256}"
checksum="${2:?usage: make-db-fixture.sh VERSION AMD64_SHA256}"
cd "$(dirname "$0")/.."
out="scripts/fixtures/mostro-db/mostro-$version.db"
work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT

curl -fsSL "https://github.com/MostroP2P/mostro/releases/download/v$version/mostrod-x86_64-unknown-linux-musl" -o "$work/mostrod"
echo "$checksum  $work/mostrod" | sha256sum -c - >/dev/null
chmod 755 "$work/mostrod"
mkdir "$work/s"
curl -fsSL "https://raw.githubusercontent.com/MostroP2P/mostro/v$version/settings.tpl.toml" -o "$work/s/settings.toml"

docker run --rm --network none --user "$(id -u):$(id -g)" -e TERM=xterm \
  -e MOSTRO_NSEC_PRIVKEY="$(cat scripts/fixtures/throwaway.nsec)" \
  -v "$work:/w" debian:bookworm-slim timeout 60 /w/mostrod -d /w/s >"$work/log" 2>&1 || true
if ! grep -q "Settings correctly loaded" "$work/log" || [ ! -s "$work/s/mostro.db" ]; then
  cat "$work/log" >&2
  echo "mostrod $version did not create its database" >&2
  exit 1
fi

python3 - "$work/s/mostro.db" <<'PY'
import sqlite3
import sys

db = sqlite3.connect(sys.argv[1])
tables = [row[0] for row in db.execute(
    "select name from sqlite_master where type = 'table'"
    " and name not like 'sqlite_%' and name != '_sqlx_migrations'")]
for table in tables:
    columns = db.execute(f'pragma table_info("{table}")').fetchall()
    try:
        for n in range(2):
            names, values = [], []
            for _, name, kind, _, _, pk in columns:
                kind = kind.upper()
                if pk and "INT" in kind:
                    continue  # rowid alias: SQLite numbers it
                names.append(f'"{name}"')
                if "INT" in kind or "BOOL" in kind:
                    values.append(n + 1)
                elif any(k in kind for k in ("REAL", "FLOA", "DOUB", "NUMERIC", "DECIMAL")):
                    values.append(n + 1.5)
                elif "BLOB" in kind:
                    values.append(bytes([n]))
                else:
                    values.append(f"fixture-{table}-{name}-{n}")
            placeholders = ", ".join("?" * len(values))
            db.execute(f'insert into "{table}" ({", ".join(names)}) values ({placeholders})', values)
        print(f"{table}: 2 filas")
    except sqlite3.Error as error:
        print(f"{table}: sin filas ({error})")
db.commit()
PY
mkdir -p "$(dirname "$out")"
cp "$work/s/mostro.db" "$out"
echo "$out"
