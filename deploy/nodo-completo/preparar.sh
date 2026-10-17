#!/usr/bin/env bash
# Prepara esta carpeta para el nodo completo: crea .env, las carpetas,
# bitcoin.conf y lnd.conf (con una contraseña RPC nueva), la contraseña de la
# billetera de LND (te la pide) y la del panel, y fija la imagen del Manager por
# digest. No arranca nada. Se puede repetir: no pisa lo que ya existe.
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
for d in data secrets respaldos; do [ -e "$d" ] || mkdir -m 700 "$d"; done
for d in data/bitcoind data/lnd data/lnd-export data/lnd-export/ro data/mostro secrets/lnd secrets/panel; do
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

paso "Contraseña del panel"
if [ -e secrets/panel/panel_password ] || ! [ -w secrets/panel ]; then
  echo "secrets/panel/panel_password ya existe"
else
  contrasena="$(contrasena_nueva)"
  (umask 077 && printf '%s\n' "$contrasena" >secrets/panel/panel_password)
  echo "Contraseña del panel: $contrasena"
  echo "Guárdala en tu gestor de contraseñas. Queda en secrets/panel/panel_password."
fi

paso "Imagen del Manager"
imagen="$(env_valor MCM_IMAGE)"
fijada="$(imagen_fijada "$imagen")" || falla "no se pudo descargar $imagen"
env_poner MCM_IMAGE "$fijada"
echo "$fijada"

paso "Permisos"
# bitcoind y LND corren como el usuario 1000: sus carpetas y su configuración
# tienen que ser suyas. Solo se tocan las carpetas de primer nivel y los
# archivos de configuración, nunca la cadena de bloques.
if [ "$(id -u)" != 1000 ]; then
  como_root "$fijada" '
    chown 1000:1000 data/bitcoind data/lnd data/lnd-export data/lnd-export/ro secrets/lnd
    for f in data/bitcoind/bitcoin.conf data/lnd/lnd.conf secrets/lnd/wallet-password; do
      [ -e "$f" ] && chown 1000:1000 "$f"
    done
    true'
fi
echo "listos"

paso "Siguiente"
cat <<EOF
1. Revisa .env (PANEL_BIND, LND_P2P_PORT) y, si quieres, data/lnd/lnd.conf (alias, externalip).
2. docker compose up -d
3. Crea la billetera enseguida, con la misma contraseña que acabas de escribir:
     ./lncli create
   Responde «n» (semilla nueva) y apunta las 24 palabras en papel.
4. Espera a que bitcoind sincronice (días en mainnet): ./bitcoin-cli getblockchaininfo
5. Abre el panel (http://127.0.0.1:$(env_valor PANEL_PORT) o por un túnel SSH) con su contraseña.
Guía completa: docs/instalar-nodo-completo.md
EOF
