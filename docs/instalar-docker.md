# Instalar con Docker junto a un LND que ya tienes

Para un servidor (VPS o equipo propio) que ya corre Bitcoin Core y LND, como sea que los instalaste. El Manager no habla con Bitcoin Core: solo con LND. Si aún no tienes nodo, mira [nodo completo](instalar-nodo-completo.md); si usas Umbrel, [Umbrel](instalar-umbrel.md).

Se instalan tres contenedores del Manager, todos con la misma imagen:

| Servicio | Qué hace |
|---|---|
| `init` | Deja los permisos de las carpetas como los esperan los otros dos y comprueba que estén las credenciales. Termina enseguida. |
| `web` | El panel y su API. Lee LND con permiso de solo lectura. |
| `mostro` | El daemon `mostrod`. Espera sin hacer nada hasta que lo actives en el panel. |

LND y Bitcoin Core no se tocan.

## Requisitos

- Docker con el plugin compose (`docker compose version`), `git` y `python3`.
- LND sincronizado, con su puerto gRPC (10009) y REST (8080) alcanzables desde los contenedores.
- El certificado de LND tiene que incluir el nombre o la IP con que lo alcanzan los contenedores. Si no, añade a `lnd.conf` `tlsextradomain=<nombre>` o `tlsextraip=<ip>`, borra `tls.cert` y `tls.key` y reinicia LND para que los regenere.

## 1. Descargar

La instalación es un clon del repositorio en una versión publicada. Así `actualizar.sh` puede cambiarla por la siguiente.

```bash
git clone --branch v1.0.16 https://github.com/ManDeBitcoin/mostro-community-manager.git mostro-manager
```

```bash
cd mostro-manager/deploy/compose
```

Todo lo de tu instalación queda en esta carpeta, fuera de Git: `.env`, `data/`, `lnd/`, `secrets/` y `respaldos/`. No edites los archivos del repositorio. Lo que necesites cambiar va en `.env` o en un `docker-compose.override.yml`.

## 2. Preparar

```bash
./preparar.sh --lnd ~/.lnd
```

Con `--lnd`, la carpeta de datos de tu LND (la que tiene `tls.cert` y `data/chain/bitcoin/…`). Si no es mainnet, añade `--red testnet`, `--red signet` o `--red regtest`. Si el nodo lo instalaste con [`deploy/nodo`](instalar-nodo-completo.md), usa en cambio `--nodo <su carpeta>`: además deja `.env` unido a su red. El script:

- crea `.env` a partir de `.env.example`;
- genera la contraseña del panel en `secrets/panel_password` y la muestra una vez: guárdala;
- copia de LND `tls.cert` y `readonly.macaroon` (para el panel, en `lnd/ro/`) y `admin.macaroon` (para mostrod, como `lnd/mostro.macaroon`);
- fija la imagen por su digest en `.env`.

Se puede repetir sin pisar nada, salvo las credenciales de LND, que vuelve a copiar. Hazlo si LND renueva su certificado, y después `docker compose restart web mostro`. Si prefieres copiar las credenciales a mano, deja esos tres archivos con esos nombres.

## 3. Ajustar `.env`

Lo que hay que mirar:

- `LND_GRPC_HOST` y `LND_REST_URL`: cómo llegan los contenedores a tu LND.
  - **LND en otro contenedor:** une los contenedores a su red. En `.env`, quita el `#` de `COMPOSE_FILE=docker-compose.yml:docker-compose.red-lnd.yml` y pon en `LND_DOCKER_NETWORK` la red de LND (`docker inspect <contenedor de lnd>`). Las direcciones quedan como `https://lnd:10009` y `https://lnd:8080`, con el nombre del servicio de LND.
  - **LND en el mismo equipo, fuera de Docker:** `https://host.docker.internal:10009` y `https://host.docker.internal:8080`. LND tiene que escuchar en una dirección alcanzable desde Docker (`rpclisten=0.0.0.0:10009`, `restlisten=0.0.0.0:8080`, con un cortafuegos que no los publique), y su certificado tiene que incluir `host.docker.internal`.
  - **LND en otro equipo** (por ejemplo por Tailscale): su dirección, con el certificado que la incluya.
- `PANEL_BIND` y `PANEL_PORT`: el panel escucha por defecto solo en `127.0.0.1:5173`.
- `BACKUP_PASSPHRASE`: si la defines (mínimo 16 caracteres), el panel hace un respaldo cifrado cada 24 horas en `data/backup`.

## 4. Arrancar

```bash
docker compose up -d
```

```bash
docker compose ps
```

`init` sale con código 0 y `web` y `mostro` quedan en marcha. Si `init` falla, `docker compose logs init` dice qué archivo falta.

## 5. Entrar al panel

Por defecto solo responde en el propio servidor. Desde tu equipo, abre un túnel SSH:

```bash
ssh -L 5173:127.0.0.1:5173 usuario@servidor
```

y abre `http://127.0.0.1:5173` con la contraseña de `secrets/panel_password`.

Para entrar sin túnel, pon delante un proxy con HTTPS (Caddy, nginx, Traefik) que apunte a `127.0.0.1:5173`, y su dirección en `TRUSTED_PROXIES`. No publiques el puerto del panel en `0.0.0.0` sin HTTPS: la contraseña viajaría en claro.

Desde ahí sigue el [manual de operación](DEPLOYMENT.md#3-configuración-inicial-paso-a-paso): identidad, reglas, respaldo, canales y activación.

## Contraseña del panel

Está en `secrets/panel_password`. Tras el primer arranque la carpeta es del usuario 1000, así que si tu usuario es otro, léela con `sudo cat secrets/panel_password`. Para cambiarla, escribe la nueva (al menos 12 caracteres) en ese archivo y ejecuta `docker compose restart web`. Las sesiones abiertas se cierran.

## Comandos útiles

```bash
docker compose logs -f mostro
```

```bash
docker compose exec web mostro-community-api daemon-status
```

```bash
docker compose exec web mostro-community-api open-orders
```

Los comandos de respaldo y restauración están en la página Respaldos del panel.

## Actualizar

Ver [actualizar](actualizar.md).

## Desinstalar

```bash
docker compose down
```

Los datos quedan en `data/`, con la identidad del nodo sin cifrar. Bórralos solo si tienes un respaldo y no vas a volver a usar esa identidad.
