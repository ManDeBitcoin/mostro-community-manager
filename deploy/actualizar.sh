#!/usr/bin/env bash
# Actualiza el Mostro Community Manager de una instalación de deploy/ a la
# versión X.Y.Z, o la devuelve a como estaba antes de la última actualización.
# Se ejecuta desde la carpeta deploy/compose de la instalación. Solo toca init,
# web y mostro: nunca LND ni bitcoind, que viven en otro proyecto.
#
# Uso: ../actualizar.sh [--prueba] X.Y.Z
#      ../actualizar.sh --volver [--si]
#   --prueba   revisa todo y dice qué cambiaría, sin tocar nada
#   --volver   vuelve a la versión anterior con los datos de antes de actualizar
#   --si       no pregunta antes de volver atrás
#
# Pasos: versión publicada, operaciones en curso (un trade tomado detiene la
# actualización; las ofertas publicadas no), prueba en seco del mostrod nuevo
# sobre una copia de la base, respaldo de los datos con el daemon parado,
# archivos de la versión (git), arranque y verificación.
set -euo pipefail

main() {
  local aqui
  aqui="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  . "$aqui/comun.sh"

  local prueba=0 volver=0 si=0 version=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --prueba) prueba=1 ;;
      --volver) volver=1 ;;
      --si) si=1 ;;
      -h|--help) sed -n '2,17p' "$0"; exit 0 ;;
      -*) falla "opción desconocida: $1" ;;
      *) version="${1#v}" ;;
    esac
    shift
  done
  [ -f docker-compose.yml ] && [ -f .env ] ||
    falla "ejecútalo desde la carpeta de la instalación (la que tiene docker-compose.yml y .env)"
  docker compose version >/dev/null 2>&1 || falla "hace falta Docker con el plugin compose"
  command -v python3 >/dev/null || falla "hace falta python3"

  actual="$(env_valor MCM_IMAGE)"
  [ -n "$actual" ] || falla "falta MCM_IMAGE en .env"
  repo="${actual%@*}"
  repo="${repo%:*}"
  # La carpeta de datos del Manager: la que el servicio web monta en /data.
  datos="$(docker compose config --format json | python3 -I -c '
import json, sys
web = json.load(sys.stdin)["services"]["web"]
print(next(v["source"] for v in web.get("volumes", []) if v.get("target") == "/data"))')" ||
    falla "no encontré la carpeta de datos del servicio web en docker-compose.yml"
  raiz="$(git rev-parse --show-toplevel 2>/dev/null || true)"
  [ -e respaldos ] || mkdir -m 700 respaldos
  registro=respaldos/ultima-actualizacion

  if [ "$volver" = 1 ]; then
    volver_atras
    return
  fi
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || falla "uso: ../actualizar.sh [--prueba] X.Y.Z  o  ../actualizar.sh --volver"
  actualizar
}

actualizar() {
  paso "1. Versión $version"
  if [ -n "$raiz" ]; then
    if [ -n "$(git -C "$raiz" status --porcelain --untracked-files=no)" ]; then
      [ "$prueba" = 1 ] || falla "hay archivos del repositorio modificados ($(git -C "$raiz" status --porcelain --untracked-files=no | head -n 3 | tr '\n' ' ')). Lo propio de la instalación va en .env o en docker-compose.override.yml; deshaz esos cambios (git -C $raiz stash) y vuelve a ejecutar."
      echo "AVISO: hay archivos del repositorio modificados; la actualización real se detendría"
    fi
    if ! git -C "$raiz" rev-parse -q --verify "refs/tags/v$version^{commit}" >/dev/null; then
      local profundidad=()
      [ "$(git -C "$raiz" rev-parse --is-shallow-repository)" = true ] && profundidad=(--depth 1)
      git -C "$raiz" fetch -q "${profundidad[@]}" origin "refs/tags/v$version:refs/tags/v$version" ||
        falla "el repositorio no tiene el tag v$version"
    fi
    carpeta="$(git rev-parse --show-prefix)"
    git -C "$raiz" cat-file -e "v$version:${carpeta}docker-compose.yml" 2>/dev/null ||
      falla "la versión $version no trae ${carpeta}docker-compose.yml"
    git_anterior="$(git -C "$raiz" rev-parse HEAD)"
    echo "archivos: v$version (ahora $(git -C "$raiz" describe --tags --always HEAD))"
  else
    git_anterior=""
    echo "AVISO: esta carpeta no es un clon del repositorio; solo se cambia la imagen. Si la versión nueva cambia docker-compose.yml, lo dicen sus notas."
  fi
  nueva="$(imagen_fijada "$repo:$version")" ||
    falla "$repo:$version no está publicada: falta el tag o el workflow de publicación no terminó"
  echo "imagen: $nueva"
  if [ "$nueva" = "$actual" ] && { [ -z "$raiz" ] || [ "$git_anterior" = "$(git -C "$raiz" rev-parse "v$version^{commit}")" ]; }; then
    echo "Ya está en $version."
    return
  fi

  paso "2. Operaciones en curso"
  # mostrod solo atiende mensajes nuevos y descarta los de más de 10 s: lo que
  # le envíen mientras reinicia se pierde. Un trade en curso no puede esperar
  # eso (un «fiat enviado» o un «liberar» perdido); una oferta publicada sí,
  # porque sigue en la base y en los relays.
  hay_base=1
  revisar_ordenes || falla "$problema"

  paso "3. Prueba en seco del mostrod nuevo"
  prueba_en_seco

  if [ "$prueba" = 1 ]; then
    paso "Prueba terminada"
    echo "Pasaría a: $nueva"
    echo "Nada cambió."
    return
  fi

  paso "4. Respaldo"
  docker compose stop web mostro
  # Una operación pudo empezar entre la revisión y la parada.
  if [ "$hay_base" = 1 ] && ! revisar_ordenes; then
    docker compose start web mostro
    falla "$problema No se cambió nada; el daemon vuelve a arrancar."
  fi
  respaldo="respaldos/mcm-antes-$version-$(date +%Y%m%d-%H%M%S)"
  docker run --rm --network none --user 0:0 --entrypoint sh -v "$datos:/d:ro" -v "$PWD/respaldos:/r" "$nueva" \
    -c "cp -a /d /r/$(basename "$respaldo")" || { docker compose start web mostro; falla "no se pudo copiar $datos"; }
  if [ "$hay_base" = 1 ]; then
    # Como root: respaldos/ es del usuario que ejecuta el script, que puede no ser el 1000.
    docker run --rm --network none --user 0:0 -v "$PWD/$respaldo:/d:ro" "$nueva" db-check /d/config/active/mostro.db >/dev/null ||
      { docker compose start web mostro; falla "la copia de la base no está íntegra: no se cambió nada"; }
  fi
  printf 'imagen_anterior=%s\ngit_anterior=%s\nrespaldo=%s\nversion=%s\n' \
    "$actual" "$git_anterior" "$respaldo" "$version" >"$registro"
  echo "$respaldo (identidad, reglas y base: no lo saques del equipo sin cifrar)"

  paso "5. Archivos de la versión"
  [ -z "$raiz" ] || git -C "$raiz" checkout -q "v$version"
  env_poner MCM_IMAGE "$nueva"
  if ! docker compose config --quiet; then
    deshacer_archivos
    docker compose up -d web mostro
    falla "la versión $version necesita algo nuevo en .env (ver sus notas). Se volvió a la anterior, con los mismos datos."
  fi
  echo "docker-compose.yml y .env de $version"

  paso "6. Arranque"
  inicio="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  docker compose up -d web mostro

  paso "7. Verificación"
  verificar "$version" || {
    cat >&2 <<EOF

ERROR: la versión $version no quedó funcionando.
Logs:          docker compose logs --tail 50 web mostro
Volver atrás:  ../actualizar.sh --volver
EOF
    exit 1
  }
  echo
  echo "Actualizado a $version. Si algo va mal: ../actualizar.sh --volver"
}

# Devuelve 1, con el motivo en $problema, si hay trades en curso o no se
# pudo leer la base. Sin base de datos, apunta hay_base=0.
revisar_ordenes() {
  local salida codigo
  set +e
  salida="$(docker run --rm --network none -v "$datos:/data:ro" "$nueva" open-orders /data/config/active/mostro.db 2>&1)"
  codigo=$?
  set -e
  case "$codigo" in
    0) echo "ninguna" ;;
    3)
      echo "$salida"
      problema="hay operaciones en curso: espera a que terminen (página Órdenes del panel) y vuelve a ejecutar."
      return 1
      ;;
    4)
      python3 -I -c 'import json, sys; print(json.loads(sys.argv[1])["pending"], "oferta(s) publicada(s): siguen disponibles después de actualizar; mientras el daemon reinicia (1 o 2 minutos) nadie podrá tomarlas.")' "$salida"
      ;;
    *)
      if grep -q "No hay base de datos" <<<"$salida"; then
        echo "el daemon aún no tiene base de datos"
        hay_base=0
      else
        problema="no se pudo leer la base del daemon: $salida."
        return 1
      fi
      ;;
  esac
}

# El mostrod nuevo sobre una copia de la base, sin red y con una clave de
# prueba: aplica sus migraciones y carga la configuración que le daría el panel.
prueba_en_seco() {
  local clave="$aqui/../scripts/fixtures/throwaway.nsec" tmp log
  if [ "$hay_base" = 0 ]; then
    echo "sin base de datos: nada que migrar"
    return
  fi
  if [ ! -s "$clave" ]; then
    echo "AVISO: falta $clave (no es un clon del repositorio): se omite la prueba en seco"
    return
  fi
  if ! docker run --rm --network none -v "$datos:/data:ro" --entrypoint test "$nueva" -s /data/config/community.json; then
    echo "sin reglas guardadas: se omite la prueba en seco"
    return
  fi
  tmp="$(mktemp -d)"
  mkdir "$tmp/s" "$tmp/lnd"
  printf 'no es un certificado\n' >"$tmp/lnd/tls.cert"
  printf 'no es un macaroon' >"$tmp/lnd/mostro.macaroon"
  chmod -R a+rX "$tmp"
  docker run --rm --network none --user 0:0 --entrypoint sh -v "$datos:/data:ro" -v "$tmp/s:/s" "$nueva" -c '
    set -e
    mostro-community-api snapshot-db /data/config/active/mostro.db /s/mostro.db
    mostro-community-api render-settings /data/config/community.json >/s/settings.toml
    chown -R 1000:1000 /s' || { borrar_tmp "$tmp"; falla "no se pudo preparar la copia para la prueba en seco"; }
  docker run --rm --network none --read-only --tmpfs /tmp --cap-drop ALL --security-opt no-new-privileges \
    -e TERM=xterm -e MOSTRO_NSEC_PRIVKEY="$(cat "$clave")" -v "$tmp/s:/s" -v "$tmp/lnd:/lnd:ro" \
    --entrypoint timeout "$nueva" 90 mostrod -d /s >"$tmp/log" 2>&1 || true
  log="$(sed 's/\x1b\[[0-9;]*[a-zA-Z]//g' "$tmp/log")"
  if ! grep -q "Settings correctly loaded" <<<"$log" ||
     grep -E -q "Could not load settings|REFUSING TO START|missing field|invalid type|unknown variant|unknown field|Failed to run migrations|DbAccessError" <<<"$log"; then
    echo "$log" | tail -n 30 >&2
    borrar_tmp "$tmp"
    falla "el mostrod de $version no arranca con tus reglas o tu base (arriba, su log). No se cambió nada."
  fi
  if ! docker run --rm --network none -v "$tmp/s:/s:ro" "$nueva" db-check /s/mostro.db >"$tmp/check" 2>&1; then
    cat "$tmp/check" >&2
    borrar_tmp "$tmp"
    falla "la base migrada por el mostrod de $version no está sana. No se cambió nada."
  fi
  borrar_tmp "$tmp"
  echo "configuración cargada y base migrada sin errores"
}

borrar_tmp() {
  docker run --rm --network none --user 0:0 --entrypoint sh -v "$1:/t" "$nueva" -c 'rm -rf /t/s' >/dev/null 2>&1 || true
  rm -rf -- "$1"
}

deshacer_archivos() {
  [ -z "$raiz" ] || [ -z "$git_anterior" ] || git -C "$raiz" checkout -q "$git_anterior"
  env_poner MCM_IMAGE "$actual"
}

# verificar X.Y.Z: el panel responde con esa versión y, si el daemon estaba
# activado, mostrod carga la configuración.
verificar() {
  local esperada="$1" salud estado
  for _ in $(seq 1 60); do
    salud="$(docker compose exec -T web curl -fsS http://127.0.0.1:3001/api/health 2>/dev/null || true)"
    if [ -n "$salud" ] && python3 -I -c 'import json, sys; sys.exit(json.loads(sys.argv[1])["version"] != sys.argv[2])' "$salud" "$esperada"; then
      break
    fi
    salud=""
    sleep 5
  done
  [ -n "$salud" ] || { echo "el panel no respondió con la versión $esperada en 5 minutos" >&2; return 1; }
  echo "panel: $(python3 -I -c 'import json, sys; h = json.loads(sys.argv[1]); print("v" + h["version"] + ", mostrod " + h["mostro_pinned_version"] + ", acceso " + h.get("auth", "?"))' "$salud")"
  estado="$(docker compose exec -T web mostro-community-api daemon-status 2>/dev/null |
    python3 -I -c 'import json, sys; print(json.load(sys.stdin)["state"])' || echo desconocido)"
  case "$estado" in
    unconfigured|configured_standby)
      echo "daemon: no está activado, nada más que comprobar"
      return 0
      ;;
  esac
  for _ in $(seq 1 60); do
    if docker compose logs --no-log-prefix --since "$inicio" mostro 2>&1 | grep -q "Settings correctly loaded"; then
      echo "daemon: configuración cargada"
      return 0
    fi
    sleep 10
  done
  echo "mostrod no cargó la configuración en 10 minutos (estado: $estado)" >&2
  return 1
}

volver_atras() {
  [ -f "$registro" ] || falla "no hay ninguna actualización que deshacer ($registro)"
  local imagen_anterior git_anterior respaldo version fallida
  imagen_anterior="$(sed -n 's/^imagen_anterior=//p' "$registro")"
  git_anterior="$(sed -n 's/^git_anterior=//p' "$registro")"
  respaldo="$(sed -n 's/^respaldo=//p' "$registro")"
  version="$(sed -n 's/^version=//p' "$registro")"
  [ -d "$respaldo" ] || falla "no está el respaldo $respaldo"
  paso "Volver a antes de la $version"
  echo "imagen: $imagen_anterior"
  echo "datos:  $respaldo"
  echo "Lo que el nodo hizo después de actualizar (órdenes nuevas, cambios de reglas) se pierde."
  echo "Los datos de ahora quedan guardados en respaldos/."
  if [ "$si" != 1 ]; then
    read -rp "¿Seguir? [s/N] " respuesta
    [ "$respuesta" = s ] || [ "$respuesta" = S ] || { echo "No se cambió nada."; return; }
  fi
  nueva="$imagen_anterior"
  docker image inspect "$imagen_anterior" >/dev/null 2>&1 || docker pull -q "$imagen_anterior" >/dev/null ||
    falla "no se pudo descargar $imagen_anterior"
  docker compose stop web mostro
  fallida="respaldos/fallida-$version-$(date +%Y%m%d-%H%M%S)"
  docker run --rm --network none --user 0:0 --entrypoint sh -v "$datos:/d" -v "$PWD/respaldos:/r" "$imagen_anterior" -c "
    set -e
    cp -a /d /r/$(basename "$fallida")
    find /d -mindepth 1 -maxdepth 1 -exec rm -rf {} +
    cp -a /r/$(basename "$respaldo")/. /d/" || falla "no se pudieron reponer los datos; los de ahora están en $fallida"
  [ -z "$raiz" ] || [ -z "$git_anterior" ] || git -C "$raiz" checkout -q "$git_anterior"
  env_poner MCM_IMAGE "$imagen_anterior"
  inicio="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  docker compose up -d web mostro
  mv "$registro" "$registro.deshecha-$(date +%Y%m%d-%H%M%S)"
  local anterior="${imagen_anterior%@*}"
  verificar "${anterior##*:}" || falla "la versión anterior tampoco quedó funcionando: docker compose logs --tail 50 web mostro"
  echo
  echo "De vuelta en ${anterior##*:}. Los datos de la versión fallida están en $fallida."
}

main "$@"
exit
