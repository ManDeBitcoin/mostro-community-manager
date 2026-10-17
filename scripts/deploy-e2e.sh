#!/usr/bin/env bash
# End-to-end test of deploy/, on regtest. Only uses temporary folders and
# containers, and publishes nothing outside them:
#   1. installs deploy/nodo-completo (bitcoind, LND, Manager) with image A,
#      creates the wallet, mines blocks, creates the identity and the rules
#      (with a local relay: nothing reaches public relays) and activates mostrod;
#   2. installs deploy/compose against that same LND, over the network
#      override, and checks the panel's login and its view of LND;
#   3. updates the full node to image B with deploy/actualizar.sh (--prueba
#      first) and goes back with --volver.
# Both images are served from a local registry, as GHCR serves them.
#
# Usage: scripts/deploy-e2e.sh IMAGE_A IMAGE_B
#   IMAGE_A, IMAGE_B: built with MANAGER_VERSION=vX.Y.Z matching their tags,
#   for example mcm-e2e:9.9.1 and mcm-e2e:9.9.2
set -euo pipefail
A="${1:?usage: deploy-e2e.sh IMAGE_A IMAGE_B}"
B="${2:?usage: deploy-e2e.sh IMAGE_A IMAGE_B}"
va="${A##*:}"
vb="${B##*:}"
repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
project="mcm-e2e-$RANDOM"
RELAY_IMAGE=scsibug/nostr-rs-relay:0.10.0@sha256:48d54c2d2781577cf3ed2951112f0953dc2c5e7c9d2ea20c64e8c0fa37d16e4d
REGISTRY_IMAGE=registry:2.8.3

free_port() { python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])'; }
step() { printf '\n### %s\n' "$*"; }
fail() { printf '\nFAIL: %s\n' "$*" >&2; exit 1; }

cleanup() {
  local result=$?
  if [ "$result" -ne 0 ] && [ -d "$work/repo/deploy/nodo-completo" ]; then
    (cd "$work/repo/deploy/nodo-completo" && docker compose logs --tail 40 web mostro lnd 2>&1 | tail -n 120) || true
  fi
  # The relay first: while it is attached, the full node's network cannot go.
  docker rm -f "$project-relay" "$project-registry" >/dev/null 2>&1 || true
  for install in compose nodo-completo; do
    [ -f "$work/repo/deploy/$install/.env" ] || continue
    (cd "$work/repo/deploy/$install" && docker compose down -v --timeout 20 >/dev/null 2>&1) || true
  done
  docker run --rm --user 0:0 --entrypoint sh -v "$work:/w" "$A" -c 'rm -rf /w/repo/deploy' >/dev/null 2>&1 || true
  rm -rf -- "$work"
  return "$result"
}
trap cleanup EXIT

# root_read PATH: a file of the installation that only user 1000 may read.
root_read() { docker run --rm --network none --user 0:0 --entrypoint cat -v "$PWD:/i:ro" "$A" "/i/$1"; }
root_test() { docker run --rm --network none --user 0:0 --entrypoint test -v "$PWD:/i:ro" "$A" -s "/i/$1"; }
set_env() { # set_env KEY VALUE
  if grep -q "^$1=" .env; then sed -i "s|^$1=.*|$1=$2|" .env; else printf '%s=%s\n' "$1" "$2" >>.env; fi
}
wait_for() { # wait_for SECONDS DESCRIPTION COMMAND...
  local seconds="$1" what="$2"
  shift 2
  for _ in $(seq 1 "$seconds"); do
    if "$@" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  fail "timed out waiting for $what"
}

# A panel session: api METHOD PATH [JSON] against $panel_url with $panel_password.
api() {
  python3 - "$panel_url" "$panel_password" "$@" <<'PY'
import http.cookiejar, json, sys, urllib.request
base, password, method, path = sys.argv[1:5]
body = sys.argv[5].encode() if len(sys.argv) > 5 else None
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
headers = {"Content-Type": "application/json", "X-Requested-With": "mostro-community", "Origin": base}
login = urllib.request.Request(base + "/api/auth/login", method="POST", headers=headers,
                               data=json.dumps({"password": password}).encode())
opener.open(login).read()
try:
    with opener.open(urllib.request.Request(base + path, method=method, headers=headers, data=body)) as r:
        print(r.read().decode())
except urllib.error.HTTPError as e:
    sys.exit(f"{method} {path}: {e.code} {e.read().decode()}")
PY
}
json_field() { python3 -c 'import json, sys; v = json.load(sys.stdin); [v := v[k] for k in sys.argv[1:]]; print(v)' "$@"; }

step "Local registry with $va and $vb"
registry_port="$(free_port)"
docker run -d --name "$project-registry" -p "127.0.0.1:$registry_port:5000" "$REGISTRY_IMAGE" >/dev/null
wait_for 30 "the registry" curl -fsS "http://127.0.0.1:$registry_port/v2/"
repo="localhost:$registry_port/mcm"
for pair in "$A $va" "$B $vb"; do
  set -- $pair
  docker tag "$1" "$repo:$2"
  docker push -q "$repo:$2" >/dev/null
  docker image rm "$repo:$2" >/dev/null
done

step "A clone of the working tree, with tags v$va and v$vb"
mkdir "$work/repo"
git -C "$repo_dir" ls-files -co --exclude-standard -z | tar -C "$repo_dir" --null -T - -cf - | tar -C "$work/repo" -xf -
git -C "$work/repo" init -q
git -C "$work/repo" add -A
git -C "$work/repo" -c user.name=e2e -c user.email=e2e@example.invalid commit -q -m e2e
git -C "$work/repo" tag "v$va"
git -C "$work/repo" tag "v$vb"

step "deploy/nodo-completo on regtest with $va"
cd "$work/repo/deploy/nodo-completo"
cp .env.example .env
set_env MCM_IMAGE "$repo:$va"
set_env COMPOSE_PROJECT_NAME "$project-nodo"
panel_port="$(free_port)"
set_env PANEL_PORT "$panel_port"
set_env LND_P2P_BIND 127.0.0.1
set_env LND_P2P_PORT "$(free_port)"
mkdir -m 700 secrets secrets/lnd
wallet_password="e2e-wallet-$RANDOM$RANDOM"
(umask 077 && printf '%s' "$wallet_password" >secrets/lnd/wallet-password)
./preparar.sh --red regtest --alias e2e </dev/null
grep -q "^MCM_IMAGE=$repo:$va@sha256:" .env || fail "preparar.sh did not pin the image"
panel_password="$(root_read secrets/panel/panel_password)"
panel_url="http://127.0.0.1:$panel_port"
docker compose up -d
network="$project-nodo_default"
docker run -d --name "$project-relay" --network "$network" --network-alias relay "$RELAY_IMAGE" >/dev/null

step "LND wallet"
wait_for 120 "LND's certificate" root_test data/lnd-export/ro/tls.cert
lnd_curl() { docker run --rm --network "$network" -v "$PWD/data/lnd-export/ro:/c:ro" --entrypoint curl "$A" -fsS --cacert /c/tls.cert "$@"; }
wait_for 60 "LND's REST port" lnd_curl https://lnd:8080/v1/genseed
seed="$(lnd_curl https://lnd:8080/v1/genseed)"
init="$(python3 -c 'import base64, json, sys; print(json.dumps({"wallet_password": base64.b64encode(sys.argv[2].encode()).decode(), "cipher_seed_mnemonic": json.loads(sys.argv[1])["cipher_seed_mnemonic"]}))' "$seed" "$wallet_password")"
lnd_curl -X POST -d "$init" https://lnd:8080/v1/initwallet >/dev/null
wait_for 120 "LND's admin macaroon" root_test data/lnd-export/admin.macaroon

step "Blocks"
wait_for 60 "bitcoind" ./bitcoin-cli getblockchaininfo
./bitcoin-cli -named createwallet wallet_name=e2e >/dev/null
./bitcoin-cli generatetoaddress 101 "$(./bitcoin-cli getnewaddress)" >/dev/null
synced() { ./lncli getinfo | json_field synced_to_chain | grep -q True; }
wait_for 180 "LND to sync" synced

step "Identity, rules and activation"
wait_for 120 "the panel" curl -fsS "$panel_url/api/health"
[ "$(curl -fsS "$panel_url/api/health" | json_field version)" = "$va" ] || fail "the panel does not report $va"
[ "$(curl -s -o /dev/null -w '%{http_code}' "$panel_url/api/community")" = 401 ] || fail "the panel answered without a login"
api POST /api/identity/generate '{}' >/dev/null
rules="$(python3 -c 'import json, sys; c = json.load(open(sys.argv[1])); c["nostr"]["relays"] = ["ws://relay:8080"]; print(json.dumps({"revision": 0, "config": c}))' "$repo_dir/api/tests/fixtures/community.json")"
api PUT /api/community "$rules" >/dev/null
api POST /api/daemon/activate '{}' >/dev/null
loaded() { docker compose logs --no-log-prefix mostro 2>&1 | grep -q "Settings correctly loaded"; }
wait_for 180 "mostrod to load its settings" loaded
running() { docker compose exec -T web mostro-community-api daemon-status | json_field state | grep -qx active_running; }
wait_for 180 "the daemon to run" running
# Only the panel's relay monitor sees the announcement, so this goes through the
# API; every call logs in, and the login allows 5 attempts a minute.
announced=0
for _ in $(seq 1 24); do
  if api GET /api/daemon/status | json_field announced_fresh | grep -qx True; then announced=1; break; fi
  sleep 15
done
[ "$announced" = 1 ] || fail "the daemon did not announce itself on the local relay"
npub="$(docker compose exec -T web mostro-community-api daemon-status | json_field npub)"
echo "running and announced as $npub"

step "deploy/compose against the same LND"
cd ../compose
cp .env.example .env
set_env MCM_IMAGE "$repo:$va"
set_env COMPOSE_PROJECT_NAME "$project-compose"
compose_port="$(free_port)"
set_env PANEL_PORT "$compose_port"
set_env COMPOSE_FILE docker-compose.yml:docker-compose.red-lnd.yml
set_env LND_DOCKER_NETWORK "$network"
./preparar.sh </dev/null
docker run --rm --network none --user 0:0 --entrypoint sh -v "$work/repo/deploy:/d" "$A" -c '
  set -e
  cp /d/nodo-completo/data/lnd-export/ro/tls.cert /d/compose/lnd/ro/tls.cert
  cp /d/nodo-completo/data/lnd-export/ro/readonly.macaroon /d/compose/lnd/ro/readonly.macaroon
  cp /d/nodo-completo/data/lnd-export/admin.macaroon /d/compose/lnd/mostro.macaroon'
docker compose up -d
wait_for 120 "the second panel" curl -fsS "http://127.0.0.1:$compose_port/api/health"
[ "$(curl -fsS "http://127.0.0.1:$compose_port/api/health" | json_field auth)" = password ] || fail "deploy/compose does not ask for a password"
docker compose exec -T web mostro-community-api check-lnd >/dev/null || fail "deploy/compose cannot read LND"
docker compose exec -T web mostro-community-api daemon-status | json_field state | grep -qx unconfigured ||
  fail "deploy/compose's daemon should wait unconfigured"
docker compose down --timeout 20 >/dev/null
echo "panel up behind its password, LND readable, daemon waiting"

step "actualizar.sh --prueba $vb"
cd ../nodo-completo
before="$(grep '^MCM_IMAGE=' .env)"
../actualizar.sh --prueba "$vb" | tee "$work/prueba.log"
grep -q "configuración cargada y base migrada sin errores" "$work/prueba.log" || fail "no dry run of the new mostrod"
[ "$(grep '^MCM_IMAGE=' .env)" = "$before" ] || fail "--prueba changed .env"
running || fail "--prueba stopped the daemon"

step "actualizar.sh $vb"
../actualizar.sh "$vb" | tee "$work/update.log"
grep -q "Actualizado a $vb" "$work/update.log" || fail "the update did not finish"
grep -q "^MCM_IMAGE=$repo:$vb@sha256:" .env || fail ".env does not pin $vb"
[ "$(curl -fsS "$panel_url/api/health" | json_field version)" = "$vb" ] || fail "the panel does not report $vb"
wait_for 180 "the daemon to run after the update" running
[ "$(docker compose exec -T web mostro-community-api daemon-status | json_field npub)" = "$npub" ] || fail "the identity changed"
[ "$(git -C "$work/repo" describe --tags --exact-match HEAD)" = "v$va" ] || [ "$(git -C "$work/repo" describe --tags --exact-match HEAD)" = "v$vb" ] ||
  fail "the files are not those of a tag"
ls respaldos | grep -q "^mcm-antes-$vb-" || fail "no backup before the update"

step "actualizar.sh --volver"
../actualizar.sh --volver --si | tee "$work/volver.log"
grep -q "De vuelta en $va" "$work/volver.log" || fail "--volver did not finish"
grep -q "^MCM_IMAGE=$repo:$va@sha256:" .env || fail ".env does not pin $va again"
[ "$(curl -fsS "$panel_url/api/health" | json_field version)" = "$va" ] || fail "the panel does not report $va"
wait_for 180 "the daemon to run after going back" running
[ "$(docker compose exec -T web mostro-community-api daemon-status | json_field npub)" = "$npub" ] || fail "the identity changed"

echo
echo "PASS: deploy/nodo-completo on regtest (wallet, identity, rules, activation, announcement), deploy/compose against its LND, update with a dry run, and going back"
