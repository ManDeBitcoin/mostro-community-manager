# Instalar un nodo completo: Bitcoin Core, LND y el Manager

Para un servidor sin nodo Lightning. Si ya tienes Bitcoin Core y LND, sáltate la primera parte y sigue [Docker junto a tu LND](instalar-docker.md).

Son **dos proyectos de Docker Compose separados**, como el nodo de BitMaxis:

| Proyecto | Servicios | Quién lo cambia |
|---|---|---|
| [`deploy/nodo`](../deploy/nodo) | `bitcoind` (`ghcr.io/getumbrel/docker-bitcoind:v31.1`) y `lnd` (`lightninglabs/lnd:v0.21.4-beta`) | Tú, cuando decidas actualizar LND o Bitcoin Core |
| [`deploy/compose`](../deploy/compose) | `init`, `web` y `mostro` (el Manager) | `actualizar.sh`, con cada versión del Manager |

El Manager se une a una sola red del nodo, la de LND (`nodo-lnd`), donde solo está LND. Por eso:

- actualizar, reiniciar o apagar el Manager **nunca para LND ni Bitcoin Core**: los canales siguen en línea;
- los contenedores del Manager no alcanzan a `bitcoind`;
- puedes conectar otros clientes al mismo LND (por ejemplo Alby Hub) uniéndolos a esa red.

Las imágenes van fijadas por digest. Todos los contenedores corren como el usuario 1000, sin privilegios y con el sistema de archivos de solo lectura.

## Requisitos

- Un servidor con 4 GB de RAM (los límites de memoria suman unos 3,3 GB), 2 núcleos y unos 40 GB libres de disco SSD. Como referencia, el nodo de BitMaxis ocupa 23 GB podado y usa alrededor de 1 GB de RAM una vez sincronizado. Con la cadena entera en vez de podada, más de 700 GB.
- Docker con el plugin compose (`docker compose version`), `git` y `python3`.
- El puerto 9735 abierto en el cortafuegos, si quieres que otros nodos te abran canales.

## Parte 1: el nodo (Bitcoin Core y LND)

### 1. Descargar

El nodo va en su propio clon del repositorio, aparte del del Manager. Así, cuando el Manager cambia de versión, los archivos del nodo no cambian.

```bash
git clone --branch v1.0.16 https://github.com/ManDeBitcoin/mostro-community-manager.git nodo-bitcoin
```

```bash
cd nodo-bitcoin/deploy/nodo
```

### 2. Preparar

```bash
./preparar.sh --alias "Mi comunidad"
```

El script:

- crea `.env` y las carpetas;
- genera `data/bitcoind/bitcoin.conf` y `data/lnd/lnd.conf` con una contraseña RPC nueva que nunca se muestra;
- te pide la contraseña de la billetera de LND y la guarda en `secrets/lnd/wallet-password`, para que LND se desbloquee solo al arrancar;
- deja las carpetas a nombre del usuario 1000.

Se puede repetir: no pisa nada que ya exista. `--red regtest` es solo para pruebas.

Antes de arrancar, revisa `.env` (el puerto P2P) y, si quieres que te encuentren otros nodos, añade `externalip=<tu IP pública>` a `data/lnd/lnd.conf`.

### 3. Arrancar y crear la billetera

```bash
docker compose up -d
```

Crea la billetera enseguida, con la misma contraseña que escribiste en el paso 2:

```bash
./lncli create
```

Responde `n` (semilla nueva), deja vacía la frase opcional y **apunta las 24 palabras en papel**. Son la única forma de recuperar los fondos de la billetera; no las guardes en ningún archivo ni chat.

### 4. Esperar la sincronización

Bitcoin Core tarda de uno a varios días en descargar y verificar la cadena. LND no opera hasta que termina.

```bash
./bitcoin-cli getblockchaininfo
```

```bash
./lncli getinfo
```

LND está listo cuando `synced_to_chain` es `true`. El Manager se puede instalar antes; simplemente no podrá activar el mercado hasta entonces.

## Parte 2: el Manager

Sigue [Docker junto a tu LND](instalar-docker.md) en otro clon. En el paso de preparar, en lugar de `--lnd`, dale la carpeta del nodo:

```bash
git clone --branch v1.0.16 https://github.com/ManDeBitcoin/mostro-community-manager.git mostro-manager
```

```bash
cd mostro-manager/deploy/compose
```

```bash
./preparar.sh --nodo ~/nodo-bitcoin/deploy/nodo
```

Copia del nodo `tls.cert`, `readonly.macaroon` y `admin.macaroon`, y deja `.env` unido a la red `nodo-lnd`, con LND en `https://lnd:10009` y `https://lnd:8080`. Después:

```bash
docker compose up -d
```

y entra al panel como dice la [guía de Docker](instalar-docker.md#5-entrar-al-panel). Los canales se abren desde LND (`./lncli openchannel …` en la carpeta del nodo) o con una herramienta como Alby Hub conectada a ese LND.

## Respaldo de los canales

LND guarda en `data/lnd/data/chain/bitcoin/mainnet/channel.backup` lo necesario para recuperar los fondos de los canales con las 24 palabras. Cambia cada vez que se abre o se cierra un canal: cópialo fuera del servidor después de cada cambio.

**Nunca restaures una copia vieja de `data/lnd`** (ni un snapshot del servidor) sobre un LND que ya tuvo canales: un estado de canal antiguo puede costar todos los fondos del canal.

## Parar y reiniciar

- Solo el Manager: `docker compose stop` en `deploy/compose`. LND sigue en marcha.
- El nodo: primero el Manager y luego `docker compose stop` en `deploy/nodo`, que respeta el tiempo de cierre de cada servicio (3 minutos bitcoind, 2 LND). No uses `docker kill` ni reinicies el servidor sin pararlo antes.
- Con el nodo parado, el Manager sigue arrancando, pero sin LND no puede cobrar ni pagar. Si bajas el nodo con `docker compose down`, su red desaparece y el Manager no arranca hasta que lo vuelvas a levantar.

## Actualizar

- **El Manager:** [actualizar](actualizar.md). Solo toca `init`, `web` y `mostro`.
- **LND o Bitcoin Core:** cuando tú lo decidas, en el clon del nodo. Lee antes sus notas: sus migraciones no tienen vuelta atrás. Hazlo sin operaciones en curso en Mostro, porque durante el reinicio de LND el daemon no puede cobrar ni pagar.

```bash
git fetch --tags && git checkout vX.Y.Z
```

```bash
docker compose up -d
```

`git checkout` de un tag más nuevo trae las versiones de LND y Bitcoin Core que esa versión del repositorio recomienda; `docker compose up -d` solo recrea los servicios cuya imagen o configuración cambió.

## En Coolify

Los dos proyectos sirven como recursos «Docker Compose» separados de Coolify, como en BitMaxis (el nodo en uno; el Manager, como Alby Hub, en otro unido a la red del nodo):

- Coolify solo debe montar **carpetas con ruta absoluta**: cambia cada `./` de los volúmenes por la ruta de la carpeta. Si montas archivos sueltos, Coolify copia su contenido a su base de datos.
- Coolify da a cada recurso su propia red. En el recurso del Manager, la red externa es la del recurso del nodo (su uuid), en lugar de `nodo-lnd`.
- Las variables de `.env` van en la pestaña de variables de Coolify, salvo las contraseñas, que siguen en archivos.
- No pongas dominio a ningún servicio. El panel no debe quedar público sin HTTPS.
- Los botones Restart y Stop de Coolify cortan a los 30 segundos: para los contenedores con `docker stop` antes de usarlos.
- `actualizar.sh` no sirve en Coolify, porque Coolify guarda su propia copia del compose. Cambia la línea `image:` de los tres servicios del Manager en Coolify y despliega solo ese recurso.
