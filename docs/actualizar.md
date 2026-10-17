# Actualizar el Manager y volver atrás

Cada versión trae sus notas en [release-notes.md](release-notes.md). Léelas antes: dicen si cambia la versión de `mostrod`, si hay migraciones de la base y si hace falta algo nuevo en `.env`.

## Lo que vale para todas las plataformas

- **Actualiza sin operaciones en curso.** `mostrod` solo atiende mensajes nuevos y descarta los de más de 10 segundos: lo que un comprador o un vendedor le envíe mientras reinicia (un «fiat enviado», un «liberar») se pierde. La página Nodo del panel avisa si hay operaciones en curso. Las ofertas publicadas no estorban: siguen en la base y en los relays, y vuelven a poder tomarse en cuanto el daemon arranca (uno o dos minutos).
- **Las migraciones de la base no tienen vuelta atrás.** Una versión de `mostrod` más vieja no arranca sobre una base que ya migró una más nueva. Por eso, antes del primer arranque de cada versión nueva de `mostrod`, el contenedor `mostro` copia la base en `config/db-snapshots/` (dentro de la carpeta de datos) y guarda las 5 últimas copias. Si no puede copiarla, no arranca.
- **Respaldos.** Además de esas copias, un respaldo cifrado desde la página Respaldos guarda identidad, reglas y base. Cópialo fuera del equipo.

## Docker (deploy/compose y deploy/nodo-completo)

Desde la carpeta de la instalación:

```bash
../actualizar.sh --prueba 1.0.17
```

```bash
../actualizar.sh 1.0.17
```

`--prueba` hace las comprobaciones sin tocar nada. La actualización real sigue estos pasos y se detiene en el primero que falla:

1. **Versión.** Comprueba que el tag `v1.0.17` exista en el repositorio y que su imagen esté publicada, y la fija por digest.
2. **Operaciones en curso.** Si hay un trade tomado y sin terminar, se detiene. Las ofertas publicadas no la detienen.
3. **Prueba en seco.** Arranca el `mostrod` nuevo sin red y con una clave de prueba sobre una copia de tu base y con tus reglas: tiene que cargar la configuración y migrar la base sin errores. Así un fallo de migración se ve antes de parar nada.
4. **Respaldo.** Para `web` y `mostro`, vuelve a revisar que no haya empezado una operación y copia toda la carpeta de datos (identidad, reglas y base) en `respaldos/mcm-antes-1.0.17-…`. Esa copia lleva la identidad sin cifrar: no la saques del equipo así.
5. **Archivos.** Pasa el clon del repositorio al tag `v1.0.17` y cambia `MCM_IMAGE` en `.env`. Si la versión nueva necesita algo en `.env` que no tienes, vuelve a la anterior sin tocar los datos y te lo dice.
6. **Arranque** de `init`, `web` y `mostro`. LND y Bitcoin Core no se tocan.
7. **Verificación.** El panel tiene que responder con la versión nueva y, si el daemon estaba activado, `mostrod` tiene que cargar su configuración.

El script se niega a seguir si modificaste archivos del repositorio: lo propio de tu instalación va en `.env` o en `docker-compose.override.yml`.

### Volver atrás

```bash
../actualizar.sh --volver
```

Vuelve a la imagen y a los archivos de antes de la última actualización y repone la carpeta de datos desde el respaldo del paso 4. **Se pierde lo que el nodo hizo después de actualizar** (órdenes nuevas, cambios de reglas). Por eso conviene volver enseguida, sin dejar que se acumulen operaciones. Los datos de la versión fallida quedan en `respaldos/fallida-…`.

Si en lugar de eso solo quieres el `mostrod` anterior sobre la base anterior, las copias de `config/db-snapshots/` sirven para hacerlo a mano: para `mostro`, pon la copia como `config/active/mostro.db` y arranca la versión anterior.

## Umbrel

Umbrel avisa de la versión nueva en su tienda, pero no la instala sola. Con el nodo sin operaciones en curso:

1. Haz un respaldo desde la página Respaldos y cópialo fuera del equipo.
2. En Umbrel, abre la app en la tienda y pulsa **Actualizar**.
3. Abre el panel y comprueba en la página Nodo la versión y que el daemon esté en marcha.

Umbrel no tiene vuelta atrás desde su interfaz. Si una versión nueva falla, la copia de `config/db-snapshots/` y el respaldo permiten volver a mano. Escribe en las incidencias del repositorio antes de intentarlo.

## Coolify

Coolify guarda su propia copia del compose, así que `actualizar.sh` no sirve. Cambia la línea `image:` de `init`, `web` y `mostro` a `ghcr.io/mandebitcoin/mostro-community-manager:X.Y.Z@sha256:<digest>` y despliega. El digest se obtiene con:

```bash
docker buildx imagetools inspect ghcr.io/mandebitcoin/mostro-community-manager:1.0.17
```
