#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

case "$(uname -m)" in
  x86_64) platform=linux/amd64 ;;
  aarch64) platform=linux/arm64 ;;
  *) printf '%s\n' 'Arquitectura no compatible (se requiere amd64 o arm64).' >&2; exit 1 ;;
esac

image="mostro-community-verification:$(date +%s)-$$"
output=$(mktemp)
cleanup() {
  rm -f "$output"
  docker image rm "$image" >/dev/null 2>&1 || true
}
trap cleanup EXIT

sh scripts/mostro-entrypoint-smoke.sh
docker build --platform "$platform" --file docker/Dockerfile.mostro --tag "$image" .

# No network, ports, mounts or credentials are provided to either container.
docker run --rm --network none --read-only --cap-drop ALL \
  --security-opt no-new-privileges "$image" --version > "$output" 2>&1
if ! grep -Fq 'mostro p2p 0.19.2' "$output"; then
  printf '%s\n' 'La imagen no devolvió la versión esperada.' >&2
  cat "$output" >&2
  exit 1
fi

if docker run --rm --network none --read-only --cap-drop ALL \
  --security-opt no-new-privileges "$image" > "$output" 2>&1; then
  printf '%s\n' 'La imagen arrancó sin identidad ni configuración; verificación fallida.' >&2
  exit 1
fi
if ! grep -Fq 'Mostro startup refused: private identity file is missing or is a symlink' "$output"; then
  printf '%s\n' 'El arranque falló por una causa inesperada.' >&2
  cat "$output" >&2
  exit 1
fi

printf '%s\n' 'Verificación aprobada: imagen oficial v0.19.2 y arranque protegido.'
