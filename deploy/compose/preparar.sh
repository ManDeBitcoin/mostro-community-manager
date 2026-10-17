#!/usr/bin/env bash
# Prepara esta carpeta para instalar el Manager junto a un LND que ya tienes:
# crea .env, las carpetas y la contraseña del panel, copia las credenciales de
# LND si le dices dónde están y fija la imagen por digest. No arranca nada.
# Se puede repetir: no pisa lo que ya existe.
#
# Uso: ./preparar.sh [--lnd CARPETA_DE_LND] [--red mainnet|testnet|signet|regtest]
#   --lnd   la carpeta de datos de LND (la que tiene tls.cert y data/chain/...)
#   --red   la red de esa billetera, por defecto mainnet
set -euo pipefail
cd "$(dirname "$0")"
. ../comun.sh

lnd_dir=""
red=mainnet
while [ $# -gt 0 ]; do
  case "$1" in
    --lnd) lnd_dir="${2:?falta la carpeta de --lnd}"; shift ;;
    --red) red="${2:?falta la red de --red}"; shift ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) falla "opción desconocida: $1" ;;
  esac
  shift
done

docker compose version >/dev/null 2>&1 || falla "hace falta Docker con el plugin compose (docker compose version)"

paso "Configuración"
if [ -f .env ]; then
  echo ".env ya existe"
else
  cp .env.example .env
  chmod 600 .env
  echo ".env creado a partir de .env.example: revisa LND_GRPC_HOST y LND_REST_URL"
fi
for d in data lnd secrets respaldos; do [ -e "$d" ] || mkdir -m 700 "$d"; done
if [ -x lnd ] && [ ! -e lnd/ro ]; then mkdir -m 700 lnd/ro; fi

paso "Contraseña del panel"
# Tras el primer arranque la carpeta es del usuario 1000 y modo 700: si no se
# puede entrar, es que ya está preparada.
if ! [ -x secrets ] || [ -e secrets/panel_password ]; then
  echo "secrets/panel_password ya existe"
else
  contrasena="$(contrasena_nueva)"
  (umask 077 && printf '%s\n' "$contrasena" >secrets/panel_password)
  echo "Contraseña del panel: $contrasena"
  echo "Guárdala en tu gestor de contraseñas. Queda en secrets/panel_password."
fi

paso "Imagen"
imagen="$(env_valor MCM_IMAGE)"
fijada="$(imagen_fijada "$imagen")" || falla "no se pudo descargar $imagen"
env_poner MCM_IMAGE "$fijada"
echo "$fijada"

paso "Credenciales de LND"
if [ -n "$lnd_dir" ]; then
  [ -d "$lnd_dir" ] || falla "no existe la carpeta $lnd_dir"
  macaroons="data/chain/bitcoin/$red"
  # Como root: la carpeta de LND suele ser de otro usuario y modo 700.
  docker run --rm --network none --user 0:0 --entrypoint sh \
    -v "$(cd "$lnd_dir" && pwd):/src:ro" -v "$PWD/lnd:/dst" "$fijada" -c "
      set -e
      cp /src/tls.cert /dst/ro/tls.cert
      cp /src/$macaroons/readonly.macaroon /dst/ro/readonly.macaroon
      cp /src/$macaroons/admin.macaroon /dst/mostro.macaroon
      chown -R 1000:1000 /dst && chmod 700 /dst /dst/ro && chmod 600 /dst/mostro.macaroon /dst/ro/*" ||
    falla "no se pudieron copiar tls.cert y los macaroons de $lnd_dir/$macaroons"
  echo "copiados a lnd/: ro/tls.cert, ro/readonly.macaroon y mostro.macaroon (admin)"
elif ! [ -x lnd ] || [ -s lnd/ro/tls.cert ]; then
  echo "lnd/ ya tiene credenciales"
else
  cat <<EOF
Faltan. Vuelve a ejecutar con la carpeta de LND, por ejemplo:
  ./preparar.sh --lnd ~/.lnd
o copia a mano tls.cert a lnd/ro/tls.cert, readonly.macaroon a
lnd/ro/readonly.macaroon y admin.macaroon a lnd/mostro.macaroon.
EOF
fi

paso "Siguiente"
cat <<EOF
1. Revisa .env (sobre todo LND_GRPC_HOST, LND_REST_URL y PANEL_BIND).
2. docker compose up -d
3. Abre el panel (http://127.0.0.1:$(env_valor PANEL_PORT) o por un túnel SSH) con la contraseña.
Guía completa: docs/instalar-docker.md
EOF
