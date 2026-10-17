# Funciones de preparar.sh y actualizar.sh. Se cargan con «.» desde la
# carpeta de la instalación (deploy/compose o deploy/nodo).

paso() { printf '\n== %s\n' "$*"; }
falla() { printf '\nERROR: %s\n' "$*" >&2; exit 1; }

# env_valor CLAVE: su valor en .env (la última línea que la define).
env_valor() {
  sed -n "s/^$1=//p" .env | tail -n 1
}

# env_poner CLAVE VALOR: cambia esa línea de .env, o la añade al final.
env_poner() {
  local tmp
  if grep -q "^$1=" .env; then
    tmp="$(mktemp)"
    awk -v k="$1" -v v="$2" 'index($0, k "=") == 1 { print k "=" v; next } { print }' .env >"$tmp"
    cat "$tmp" >.env
    rm -f "$tmp"
  else
    printf '%s=%s\n' "$1" "$2" >>.env
  fi
}

# imagen_fijada REPO:TAG[@DIGEST]: descarga la imagen e imprime REPO:TAG@DIGEST,
# el digest que el registro da hoy a ese tag.
imagen_fijada() {
  local ref="${1%@*}" repo digest
  repo="${ref%:*}"
  docker pull -q "$ref" >/dev/null 2>&1 || return 1
  digest="$(docker image inspect --format '{{range .RepoDigests}}{{println .}}{{end}}' "$ref" |
    sed -n "s|^$repo@||p" | head -n 1)"
  [ -n "$digest" ] || return 1
  printf '%s@%s\n' "$ref" "$digest"
}

# como_root IMAGEN COMANDO...: ejecuta un comando de shell como root en un
# contenedor de usar y tirar, con la carpeta de la instalación en /i. Sirve
# para leer y copiar los datos, que son del usuario 1000 y modo 700.
como_root() {
  local imagen="$1"
  shift
  docker run --rm --network none --user 0:0 --entrypoint sh -v "$PWD:/i" -w /i "$imagen" -c "$*"
}

# Una contraseña aleatoria de 24 caracteres, sin símbolos que haya que escapar.
contrasena_nueva() {
  head -c 18 /dev/urandom | base64 | tr '+/' '-_'
}
