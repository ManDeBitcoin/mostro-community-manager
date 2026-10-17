# Instalar en Umbrel

La app se instala desde la tienda comunitaria [ManDeBitcoin/mostro-community-umbrel](https://github.com/ManDeBitcoin/mostro-community-umbrel). Usa el Bitcoin Node y el Lightning Node (LND) de tu Umbrel.

## Requisitos

- Umbrel con las apps **Bitcoin Node** y **Lightning Node** instaladas y sincronizadas.
- Al menos 4 GB de RAM y 10 GB libres para la app y sus respaldos.

## Pasos

1. En Umbrel, abre **App Store**, menú de los tres puntos, **Community App Stores**.
2. Añade `https://github.com/ManDeBitcoin/mostro-community-umbrel`.
3. Abre la tienda nueva e instala **Mostro Community Manager**.
4. Ábrela desde el escritorio de Umbrel. El login es el de Umbrel: el panel no pide otra contraseña.

ID de la app: `mandebitcoin-mostro-manager`. Puerto del panel: `5173`, a través del `app_proxy` de Umbrel. No publiques el contenedor con un proxy propio.

Desde ahí sigue el [manual de operación](DEPLOYMENT.md#3-configuración-inicial-paso-a-paso): identidad, reglas, respaldo, canales y activación.

## Qué instala

| Servicio | Qué hace |
|---|---|
| `init` | Deja los permisos de la carpeta de datos. Termina enseguida. |
| `lnd_bridge` | Un puente de red de destino fijo hacia el REST de LND, sin credenciales. |
| `web` | El panel y su API, en una red privada. Lee LND con `readonly.macaroon`. |
| `mostro` | El daemon `mostrod`, con el `admin.macaroon` de LND. Espera sin hacer nada hasta que lo actives. |

Los datos (identidad, reglas, base de `mostrod` y respaldos) viven en `~/umbrel/app-data/mandebitcoin-mostro-manager/data`.

## Comandos por SSH

Los comandos de la API se ejecutan en el contenedor `web`:

```bash
sudo docker exec -it --user 1000:1000 mandebitcoin-mostro-manager_web_1 mostro-community-api daemon-status
```

La página Respaldos del panel muestra los de respaldar, comprobar y restaurar.

## Actualizar

Ver [actualizar](actualizar.md#umbrel).
