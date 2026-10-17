#!/usr/bin/env bash
# Prepara esta carpeta para el nodo Bitcoin y Lightning: crea .env, las
# carpetas, bitcoin.conf y lnd.conf (con una contraseña RPC nueva) y la
# contraseña de la billetera de LND (te la pide). No arranca nada. Se puede
# repetir: no pisa lo que ya existe.
#
# Uso: ./preparar.sh [--red mainnet|regtest] [--alias NOMBRE]
#   --red     red de Bitcoin, por defecto mainnet (regtest: solo para pruebas)
#   --alias   nombre público del nodo Lightning
set -euo pipefail
cd "$(dirname "$0")"
. ../comun.sh

red=mainnet
alias=""
while [ $# -gt 0 ]; do
  case "$1" in
    --red) red="${2:?falta la red de --red}"; shift ;;
    --alias) alias="${2:?falta el nombre de --alias}"; shift ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) falla "opción desconocida: $1" ;;
  esac
  shift
done

docker compose version >/dev/null 2>&1 || falla "hace falta Docker con el plugin compose (docker compose version)"
command -v python3 >/dev/null || falla "hace falta python3 para generar la configuración"

paso "Configuración"
if [ -f .env ]; then
  echo ".env ya existe"
else
  cp .env.example .env
  chmod 600 .env
  echo ".env creado a partir de .env.example"
fi
for d in data secrets data/bitcoind data/lnd data/lnd-export data/lnd-export/ro secrets/lnd; do
  [ -e "$d" ] || mkdir -m 700 "$d"
done

paso "bitcoin.conf y lnd.conf ($red)"
python3 -I generar-config.py "$red" data "$alias"

paso "Contraseña de la billetera de LND"
if [ -e secrets/lnd/wallet-password ] || ! [ -w secrets/lnd ]; then
  echo "secrets/lnd/wallet-password ya existe"
else
  [ -t 0 ] || falla "hace falta una terminal para escribir la contraseña de la billetera"
  read -rsp "Contraseña nueva para la billetera de LND (mínimo 8 caracteres): " uno; echo
  read -rsp "Repítela: " dos; echo
  [ "$uno" = "$dos" ] || falla "no coinciden"
  [ "${#uno}" -ge 8 ] || falla "tiene menos de 8 caracteres"
  (umask 077 && printf '%s' "$uno" >secrets/lnd/wallet-password)
  unset uno dos
  echo "guardada en secrets/lnd/wallet-password: LND la usa para desbloquearse al arrancar"
fi

paso "Permisos"
# bitcoind y LND corren como el usuario 1000: sus carpetas y su configuración
# tienen que ser suyas. Solo se tocan las carpetas y los archivos de
# configuración, nunca la cadena de bloques. Como root, con la imagen de LND.
if [ "$(id -u)" != 1000 ]; then
  imagen_lnd="$(docker compose config --images | grep -m1 lightninglabs/lnd)"
  como_root "$imagen_lnd" '
    chown 1000:1000 data/bitcoind data/lnd data/lnd-export data/lnd-export/ro secrets/lnd
    for f in data/bitcoind/bitcoin.conf data/lnd/lnd.conf secrets/lnd/wallet-password; do
      if [ -e "$f" ]; then chown 1000:1000 "$f"; fi
    done'
fi
echo "listos"

paso "Siguiente"
cat <<EOF
1. Revisa .env (LND_P2P_PORT) y, si quieres, data/lnd/lnd.conf (alias, externalip).
2. docker compose up -d
3. Crea la billetera enseguida, con la misma contraseña que acabas de escribir:
     ./lncli create
   Responde «n» (semilla nueva) y apunta las 24 palabras en papel.
4. Espera a que bitcoind sincronice (días en mainnet): ./bitcoin-cli getblockchaininfo
5. Instala el Manager aparte, unido a este LND (docs/instalar-nodo-completo.md):
     ./preparar.sh --nodo $PWD   (en la carpeta deploy/compose de otro clon)
EOF
