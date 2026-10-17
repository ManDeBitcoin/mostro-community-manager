# Instalar un nodo completo: Bitcoin Core, LND y el Manager

Para un servidor sin nodo Lightning: un solo proyecto de Docker Compose con Bitcoin Core, LND y el Manager (panel y daemon `mostrod`). Es la generalización de cómo corre el nodo de BitMaxis. Si ya tienes LND, mira [Docker junto a tu LND](instalar-docker.md).

| Servicio | Imagen | Qué hace |
|---|---|---|
| `bitcoind` | `ghcr.io/getumbrel/docker-bitcoind:v31.1` | Bitcoin Core, podado a 20 GB de bloques |
| `lnd` | `lightninglabs/lnd:v0.21.4-beta` | El nodo Lightning: billetera y canales |
| `init`, `web`, `mostro` | el Manager | Permisos, panel y daemon, como en [Docker](instalar-docker.md) |

Las imágenes van fijadas por digest. Todos corren como el usuario 1000, sin privilegios y con el sistema de archivos de solo lectura.

## Requisitos

- Un servidor con 4 GB de RAM (los límites de memoria suman unos 3,3 GB), 2 núcleos y unos 40 GB libres de disco SSD. Como referencia, el nodo de BitMaxis ocupa 23 GB podado y usa alrededor de 1 GB de RAM una vez sincronizado. Con la cadena entera en vez de podada, más de 700 GB.
- Docker con el plugin compose (`docker compose version`), `git` y `python3`.
- El puerto 9735 abierto en el cortafuegos, si quieres que otros nodos te abran canales.

## 1. Descargar

```bash
git clone --branch v1.0.16 https://github.com/ManDeBitcoin/mostro-community-manager.git mostro-nodo
```

```bash
cd mostro-nodo/deploy/nodo-completo
```

Todo lo de tu instalación queda en esta carpeta, fuera de Git: `.env`, `data/`, `secrets/` y `respaldos/`. No edites los archivos del repositorio.

## 2. Preparar

```bash
./preparar.sh --alias "Mi comunidad"
```

El script:

- crea `.env` y las carpetas;
- genera `data/bitcoind/bitcoin.conf` y `data/lnd/lnd.conf` con una contraseña RPC nueva que nunca se muestra;
- te pide la contraseña de la billetera de LND y la guarda en `secrets/lnd/wallet-password`, para que LND se desbloquee solo al arrancar;
- genera la contraseña del panel en `secrets/panel/panel_password` y la muestra una vez: guárdala;
- fija la imagen del Manager por su digest y deja las carpetas a nombre del usuario 1000.

Se puede repetir: no pisa nada que ya exista. `--red regtest` es solo para pruebas.

Antes de arrancar, revisa `.env` (`PANEL_BIND`, el puerto P2P) y, si quieres que te encuentren otros nodos, añade `externalip=<tu IP pública>` a `data/lnd/lnd.conf`.

## 3. Arrancar y crear la billetera

```bash
docker compose up -d
```

Crea la billetera enseguida, con la misma contraseña que escribiste en el paso 2:

```bash
./lncli create
```

Responde `n` (semilla nueva), deja vacía la frase opcional y **apunta las 24 palabras en papel**. Son la única forma de recuperar los fondos de la billetera; no las guardes en ningún archivo ni chat.

## 4. Esperar la sincronización

Bitcoin Core tarda de uno a varios días en descargar y verificar la cadena. LND no opera hasta que termina.

```bash
./bitcoin-cli getblockchaininfo
```

```bash
./lncli getinfo
```

LND está listo cuando `synced_to_chain` es `true`.

## 5. Entrar al panel

Por defecto el panel solo responde en el servidor, en `127.0.0.1:5173`. Desde tu equipo, abre un túnel SSH:

```bash
ssh -L 5173:127.0.0.1:5173 usuario@servidor
```

y abre `http://127.0.0.1:5173` con la contraseña de `secrets/panel/panel_password`. Con una VPN como Tailscale, puedes poner su IP en `PANEL_BIND`. Para publicarlo en internet, solo detrás de un proxy con HTTPS.

Desde ahí sigue el [manual de operación](DEPLOYMENT.md#3-configuración-inicial-paso-a-paso): identidad, reglas, respaldo, canales y activación. Los canales se abren desde LND (`./lncli openchannel …`) o con una herramienta como Alby Hub conectada a este LND.

## Respaldo de los canales

LND guarda en `data/lnd/data/chain/bitcoin/mainnet/channel.backup` lo necesario para recuperar los fondos de los canales con las 24 palabras. Cambia cada vez que se abre o se cierra un canal: cópialo fuera del servidor después de cada cambio.

**Nunca restaures una copia vieja de `data/lnd`** (ni un snapshot del servidor) sobre un LND que ya tuvo canales: un estado de canal antiguo puede costar todos los fondos del canal.

## Parar el nodo

`docker compose stop` respeta el tiempo de cierre de cada servicio (3 minutos bitcoind, 2 LND). No uses `docker kill` ni reinicies el servidor sin pararlo antes.

## Actualizar

- El Manager: [actualizar](actualizar.md). Solo toca `init`, `web` y `mostro`.
- LND y Bitcoin Core vienen fijados en `docker-compose.yml`. Cuando una versión del Manager trae otros, los aplica `docker compose up -d lnd bitcoind`. Lee antes sus notas: sus migraciones no tienen vuelta atrás. Hazlo sin órdenes abiertas en Mostro.

## En Coolify

El compose sirve como recurso «Docker Compose» de Coolify, como el de BitMaxis, con estos cambios:

- Coolify solo debe montar **carpetas con ruta absoluta**: cambia cada `./` de los volúmenes por la ruta de esta carpeta. Si montas archivos sueltos, Coolify copia su contenido a su base de datos.
- Las variables de `.env` van en la pestaña de variables de Coolify, salvo las contraseñas, que siguen en `secrets/`.
- No pongas dominio a ningún servicio. El panel no debe quedar público sin HTTPS.
- Los botones Restart y Stop de Coolify cortan a los 30 segundos: para el nodo con `docker stop` de cada contenedor (mostro, web, lnd, bitcoind) antes de usarlos.
- `actualizar.sh` no sirve en Coolify, porque Coolify guarda su propia copia del compose. Cambia la línea `image:` de los tres servicios del Manager en Coolify y despliega.
