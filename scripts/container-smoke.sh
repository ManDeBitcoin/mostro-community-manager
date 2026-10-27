#!/usr/bin/env bash
# Only creates disposable test containers; never points at an operator's data.
set -euo pipefail
IMAGE="${1:?Supply the freshly built image}"
SMOKE_DATA="$(mktemp -d)"
export SMOKE_SECRETS="$(mktemp -d)"
SMOKE_NAME="mostro-manager-smoke-${RANDOM}"
SMOKE_OWNER="$(id -u):$(id -g)"
cleanup() {
  local result=$?
  if [ "$result" -ne 0 ]; then docker logs "$SMOKE_NAME" 2>&1 || true; fi
  docker rm -f "$SMOKE_NAME" "$SMOKE_NAME-open" >/dev/null 2>&1 || true
  docker run --rm --user 0:0 --entrypoint sh -v "$SMOKE_DATA:/data" "$IMAGE" -c 'chown -R "$1" /data' sh "$SMOKE_OWNER" >/dev/null 2>&1 || true
  rm -rf -- "$SMOKE_DATA" "$SMOKE_SECRETS"
  return "$result"
}
trap cleanup EXIT
# A throwaway password; the container's user has to be able to read it.
export SMOKE_PASSWORD="smoke-test-password-${RANDOM}"
printf '%s\n' "$SMOKE_PASSWORD" > "$SMOKE_SECRETS/panel_password"
chmod 0755 "$SMOKE_SECRETS" && chmod 0644 "$SMOKE_SECRETS/panel_password"

# Listening beyond loopback with no way to log in, the panel refuses to start.
if refused="$(timeout 30 docker run --rm --name "$SMOKE_NAME-open" --network none "$IMAGE" 2>&1)"; then
  echo "FAIL: the panel started without PANEL_PASSWORD_FILE or PANEL_AUTH" >&2; exit 1
fi
grep -F PANEL_PASSWORD_FILE <<<"$refused" >/dev/null

docker run --rm --user 0:0 --entrypoint sh -v "$SMOKE_DATA:/data" "$IMAGE" -c 'mkdir -p /data/config && chown 1000:1000 /data /data/config && chmod 700 /data /data/config'
docker run --detach --name "$SMOKE_NAME" --read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges:true -v "$SMOKE_DATA:/data" \
  -v "$SMOKE_SECRETS/panel_password:/run/secrets/panel_password:ro" -e PANEL_PASSWORD_FILE=/run/secrets/panel_password -p 127.0.0.1::3001 "$IMAGE"
SMOKE_PORT="$(docker port "$SMOKE_NAME" 3001/tcp | cut -d: -f2)"
export SMOKE_URL="http://127.0.0.1:$SMOKE_PORT"
for attempt in $(seq 1 30); do
  if curl --max-time 2 --fail --silent "$SMOKE_URL/api/health" >/dev/null; then break; fi
  sleep 1
done
python3 - <<'PY'
import json,os,urllib.error,urllib.request,pathlib
base=os.environ['SMOKE_URL']
def status(request):
 try:
  with urllib.request.urlopen(request) as r: return r.status
 except urllib.error.HTTPError as e: return e.code
def login(password):
 return urllib.request.Request(base+'/api/auth/login', method='POST', data=json.dumps({'password':password}).encode(), headers={'Content-Type':'application/json','X-Requested-With':'mostro-community','Origin':base})
# The page itself and the health check answer without a session; the API does not.
with urllib.request.urlopen(base+'/') as r:
 assert r.status==200 and b'<html' in r.read()
 assert "default-src 'self'" in r.headers['Content-Security-Policy']
with urllib.request.urlopen(base+'/api/health') as r:
 health=json.load(r)
 assert health['auth']=='password' and health['platform']=='docker', health
assert status(base+'/api/community')==401
assert status(login('wrong password'))==401
with urllib.request.urlopen(login(os.environ['SMOKE_PASSWORD'])) as r:
 cookie=r.headers['Set-Cookie'].split(';')[0]
 assert 'HttpOnly' in r.headers['Set-Cookie']
config=json.loads(pathlib.Path('api/tests/fixtures/community.json').read_text())
request=urllib.request.Request(base+'/api/community', method='PUT', data=json.dumps({'revision':0,'config':config}).encode(), headers={'Content-Type':'application/json','X-Requested-With':'mostro-community','Origin':base,'Cookie':cookie})
with urllib.request.urlopen(request) as r: assert json.load(r)['revision']==1
with urllib.request.urlopen(urllib.request.Request(base+'/api/dashboard', headers={'Cookie':cookie})) as r:
 data=json.load(r)
 assert data['market_started'] is False and data['mostro']['status']=='unconfigured'
pathlib.Path(os.environ['SMOKE_SECRETS'],'cookie').write_text(cookie)
PY
docker exec "$SMOKE_NAME" mostro-community-api check-mostro | python3 -c 'import json,sys; data=json.load(sys.stdin); assert data["draft"]["status"] == "valid" and data["identity"]["status"] == "missing" and data["can_start_market"] is False'
docker exec "$SMOKE_NAME" mostro-community-api daemon-status | python3 -c 'import json,sys; data=json.load(sys.stdin); assert data["state"] == "unconfigured" and data["can_activate"] is False'
docker exec "$SMOKE_NAME" mostrod --version | grep -F 'mostro p2p 0.20.1' >/dev/null
docker restart "$SMOKE_NAME" >/dev/null
# Docker may assign a different ephemeral host port when restarting the container.
SMOKE_PORT="$(docker port "$SMOKE_NAME" 3001/tcp | cut -d: -f2)"
export SMOKE_URL="http://127.0.0.1:$SMOKE_PORT"
for attempt in $(seq 1 30); do
  if curl --max-time 2 --fail --silent "$SMOKE_URL/api/health" >/dev/null; then break; fi
  sleep 1
done
python3 - <<'PY'
import json,os,urllib.error,urllib.request,pathlib
base=os.environ['SMOKE_URL']
# Sessions live in memory: a restart ends them.
cookie=pathlib.Path(os.environ['SMOKE_SECRETS'],'cookie').read_text()
try:
 urllib.request.urlopen(urllib.request.Request(base+'/api/community', headers={'Cookie':cookie}))
 raise SystemExit('FAIL: a session survived the restart')
except urllib.error.HTTPError as e: assert e.code==401
login=urllib.request.Request(base+'/api/auth/login', method='POST', data=json.dumps({'password':os.environ['SMOKE_PASSWORD']}).encode(), headers={'Content-Type':'application/json','X-Requested-With':'mostro-community','Origin':base})
with urllib.request.urlopen(login) as r: cookie=r.headers['Set-Cookie'].split(';')[0]
with urllib.request.urlopen(urllib.request.Request(base+'/api/community', headers={'Cookie':cookie})) as r:
 data=json.load(r)
 assert data['revision']==1 and data['config']['community']['name']=='Comunidad de prueba'
PY
# With PANEL_AUTH=external (Umbrel's app_proxy asks for the login) the API answers without one.
docker rm -f "$SMOKE_NAME" >/dev/null
docker run --detach --name "$SMOKE_NAME" --read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges:true -v "$SMOKE_DATA:/data" \
  -e PANEL_AUTH=external -e MCM_PLATFORM=umbrel -p 127.0.0.1::3001 "$IMAGE" >/dev/null
SMOKE_PORT="$(docker port "$SMOKE_NAME" 3001/tcp | cut -d: -f2)"
export SMOKE_URL="http://127.0.0.1:$SMOKE_PORT"
for attempt in $(seq 1 30); do
  if curl --max-time 2 --fail --silent "$SMOKE_URL/api/health" >/dev/null; then break; fi
  sleep 1
done
python3 - <<'PY'
import json,os,urllib.request
base=os.environ['SMOKE_URL']
with urllib.request.urlopen(base+'/api/health') as r:
 health=json.load(r)
 assert health['auth']=='external' and health['platform']=='umbrel', health
with urllib.request.urlopen(base+'/api/community') as r:
 assert json.load(r)['revision']==1
print('PASS: refuses to start open, password login and sessions, static UI, CSP, same-origin save, isolated dashboard, persistence after container restart, external auth')
PY
