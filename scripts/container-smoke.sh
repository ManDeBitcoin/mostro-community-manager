#!/usr/bin/env bash
# Only creates disposable test containers; never points at an operator's data.
set -euo pipefail
IMAGE="${1:?Supply the freshly built image}"
SMOKE_DATA="$(mktemp -d)"
SMOKE_NAME="mostro-manager-smoke-${RANDOM}"
SMOKE_OWNER="$(id -u):$(id -g)"
cleanup() {
  local result=$?
  if [ "$result" -ne 0 ]; then docker logs "$SMOKE_NAME" 2>&1 || true; fi
  docker rm -f "$SMOKE_NAME" >/dev/null 2>&1 || true
  docker run --rm --user 0:0 --entrypoint sh -v "$SMOKE_DATA:/data" "$IMAGE" -c 'chown -R "$1" /data' sh "$SMOKE_OWNER" >/dev/null 2>&1 || true
  rm -rf -- "$SMOKE_DATA"
  return "$result"
}
trap cleanup EXIT
docker run --rm --user 0:0 --entrypoint sh -v "$SMOKE_DATA:/data" "$IMAGE" -c 'mkdir -p /data/config && chown 1000:1000 /data /data/config && chmod 700 /data /data/config'
docker run --detach --name "$SMOKE_NAME" --read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges:true -v "$SMOKE_DATA:/data" -p 127.0.0.1::3001 "$IMAGE"
SMOKE_PORT="$(docker port "$SMOKE_NAME" 3001/tcp | cut -d: -f2)"
export SMOKE_URL="http://127.0.0.1:$SMOKE_PORT"
for attempt in $(seq 1 30); do
  if curl --max-time 2 --fail --silent "$SMOKE_URL/api/health" >/dev/null; then break; fi
  sleep 1
done
python3 - <<'PY'
import json,os,urllib.request,pathlib
base=os.environ['SMOKE_URL']
with urllib.request.urlopen(base+'/') as r:
 assert r.status==200 and b'<html' in r.read()
 assert "default-src 'self'" in r.headers['Content-Security-Policy']
config=json.loads(pathlib.Path('api/tests/fixtures/community.json').read_text())
request=urllib.request.Request(base+'/api/community', method='PUT', data=json.dumps({'revision':0,'config':config}).encode(), headers={'Content-Type':'application/json','X-Requested-With':'mostro-community','Origin':base})
with urllib.request.urlopen(request) as r: assert json.load(r)['revision']==1
with urllib.request.urlopen(base+'/api/dashboard') as r:
 data=json.load(r)
 assert data['market_started'] is False and data['mostro']['status']=='unconfigured'
PY
docker exec "$SMOKE_NAME" mostro-community-api check-mostro | python3 -c 'import json,sys; data=json.load(sys.stdin); assert data["draft"]["status"] == "valid" and data["identity"]["status"] == "missing" and data["can_start_market"] is False'
docker exec "$SMOKE_NAME" mostro-community-api daemon-status | python3 -c 'import json,sys; data=json.load(sys.stdin); assert data["state"] == "unconfigured" and data["can_activate"] is False'
docker exec "$SMOKE_NAME" mostrod --version | grep -F 'mostro p2p 0.19.2' >/dev/null
docker restart "$SMOKE_NAME" >/dev/null
# Docker may assign a different ephemeral host port when restarting the container.
SMOKE_PORT="$(docker port "$SMOKE_NAME" 3001/tcp | cut -d: -f2)"
export SMOKE_URL="http://127.0.0.1:$SMOKE_PORT"
for attempt in $(seq 1 30); do
  if curl --max-time 2 --fail --silent "$SMOKE_URL/api/health" >/dev/null; then break; fi
  sleep 1
done
python3 - <<'PY'
import json,os,urllib.request
with urllib.request.urlopen(os.environ['SMOKE_URL']+'/api/community') as r:
 data=json.load(r)
 assert data['revision']==1 and data['config']['community']['name']=='Comunidad de prueba'
print('PASS: static UI, CSP, same-origin save, isolated dashboard and persistence after container restart')
PY
