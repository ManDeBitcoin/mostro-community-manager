# Convivencia con una instancia Mostro existente

## Qué instala el Manager

- Su propio daemon `mostrod` (el binario oficial), que espera sin hacer nada hasta que lo actives en el panel. No reutiliza la identidad, la configuración ni la base de datos de otra instancia.
- En Umbrel: app `mandebitcoin-mostro-manager`, con contenedores de nombres distintos de cualquier otra instalación de Mostro; panel en el puerto `5173` a través de `app_proxy`; datos solo en `${APP_DATA_DIR}/data`. La API vive en la red interna `manager_private`; `app_proxy` y un puente TCP de destino fijo hacia el REST de LND la conectan con la red de Umbrel, y el puente no recibe credenciales.
- Con Docker ([`deploy/compose`](../deploy/compose)): un proyecto de Compose propio, con su nombre (`COMPOSE_PROJECT_NAME`) y sus carpetas, conectado a un LND que ya tienes. Si Bitcoin Core y LND los instala [`deploy/nodo`](../deploy/nodo), son otro proyecto: el Manager solo se une a la red de LND y nunca los para ni los reinicia.

No incluye Watchdog, relay ni Push. No monta el socket de Docker ni la configuración o la base de datos de otro Mostro. El panel lee de LND solo `tls.cert` y `readonly.macaroon`; el daemon usa el macaroon que le des (en Umbrel, `admin.macaroon`). El `nsec` importado pertenece al almacenamiento del Manager y no se expone por HTTP. Instalar o desinstalar el Manager no reinicia ni cambia los servicios que ya tenías.

Comprueba antes de instalar que el puerto del panel (`5173` por defecto) esté libre en el equipo. En Umbrel, si hay un conflicto real, cambia `port` en el manifiesto; con Docker, `PANEL_PORT` en `.env`. Los puertos internos 3001 de distintos contenedores no son por sí mismos un conflicto.

## Cómo conectar el Mostro actual en una etapa posterior

1. Identificar contenedor/servicio, versión exacta, red y rutas (sin publicar secretos).
2. Confirmar si hay trades, disputes, bonds o pagos pendientes mediante interfaces de lectura compatibles.
3. Verificar backup consistente y vinculación de identidad de Mostro con su LND.
4. Añadir primero observación de solo lectura por una ruta privada; comprobar compatibilidad del RPC y de las credenciales.
5. Diseñar y probar en regtest las mutaciones administrativas antes de habilitarlas en el panel.

No copiar SQLite/nsec a otro daemon y arrancarlo en paralelo. Cambiar identidad, usar otra base de datos con el mismo estado Lightning o mezclar credenciales puede dejar operaciones pendientes fuera del control de la instancia correcta. La migración es un proyecto aparte.

Una comprobación de archivos no prueba que un servicio esté vivo: para cerrar el inventario hace falta acceso de lectura a Docker/systemd o que el operador proporcione el nombre y la ruta de la instancia.

## Docker anidado en Dockge (Umbrel)

Una lista del Docker del host no incluye necesariamente los contenedores de Dockge. En la instalación inspeccionada, su compose configura el daemon con `--host unix:///data/docker.sock`. Consultar dentro del contenedor sin especificar esa ruta produce un error de conexión y no demuestra que esté vacío:

```sh
sudo docker exec dockge_docker_1 docker -H unix:///data/docker.sock ps -a --format={{.Names}}
```

Esta consulta muestra únicamente nombres; no muestra claves ni modifica contenedores. Si aparece un prompt `>` al pegar un comando con comillas, cancelar con Ctrl+C y pegar la línea completa anterior, que no necesita comillas.
