#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 -m unittest discover -s scripts/tests
npm --prefix web run build
docker compose config --quiet
for install in deploy/compose deploy/nodo; do
  docker compose --project-directory "$install" --env-file "$install/.env.example" -f "$install/docker-compose.yml" config --quiet
done
LND_DOCKER_NETWORK=red-lnd docker compose --project-directory deploy/compose --env-file deploy/compose/.env.example \
  -f deploy/compose/docker-compose.yml -f deploy/compose/docker-compose.red-lnd.yml config --quiet
for script in deploy/actualizar.sh deploy/*/preparar.sh deploy/nodo/lncli deploy/nodo/bitcoin-cli scripts/deploy-e2e.sh; do
  bash -n "$script"
done
sh scripts/mostro-entrypoint-smoke.sh
