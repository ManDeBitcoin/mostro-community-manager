# Mostro Community Manager

<img src="assets/logo.png" alt="Mostro Community Manager" width="720">

Panel de operador para configurar una comunidad P2P sobre Mostro y Lightning, activar el daemon y supervisar órdenes, liquidez y mediación.

Este repositorio es el código: la API, el panel y la imagen `ghcr.io/mandebitcoin/mostro-community-manager`, que incluye el daemon oficial Mostro (v0.20.0 desde la versión 1.0.14 del Manager, v0.19.2 de la 1.0.12 a la 1.0.13, v0.19.0 en la 1.0.11). La imagen también se publica con su nombre anterior, `ghcr.io/mandebitcoin/mostro-community-umbrel`.

## Instalar en Umbrel

La app se instala desde la tienda comunitaria [ManDeBitcoin/mostro-community-umbrel](https://github.com/ManDeBitcoin/mostro-community-umbrel), que tiene sus pasos:

1. En Umbrel, abrir **App Store → Community App Stores / Tiendas comunitarias**.
2. Añadir `https://github.com/ManDeBitcoin/mostro-community-umbrel`.
3. Instalar **Mostro Community Manager**.

ID estable de la aplicación: `mandebitcoin-mostro-manager`. Puerto del panel: `5173`, gestionado por `app_proxy`. No configurar un proxy público directo al contenedor. Datos exclusivos en `${APP_DATA_DIR}/data/config`.

## Si ya tienes Mostro funcionando

Puedes conservar tu instalación. El Manager incluye su propio daemon Mostro, que permanece en espera hasta la activación, y no importa automáticamente la identidad ni la base de datos de una instancia externa. Antes de activar otra comunidad, comprueba identidad, red, LND, estado de operaciones y backups. Ver [convivencia y futura conexión](docs/coexistence.md).

## Incluye

- Panel React en español y API Rust/Axum.
- Configuración persistente de comunidad, mercado, seguridad, relays y métodos de pago, con validación y revisiones.
- Activación y desactivación del daemon Mostro oficial a partir de la configuración guardada.
- Página de resumen con el estado del mercado, lo que requiere atención y la puesta en marcha guiada.
- Consulta de LND y página de liquidez; libro de órdenes públicas Nostr y simulador de operaciones.
- Consola de mediación de solo lectura (disputas del nodo y mensajes de protocolo), notificaciones en vivo y respaldos cifrados.
- Tarjeta firmada de la comunidad para conectar apps, con publicación opcional en los relays del nodo.

El [roadmap](docs/roadmap.md) conserva el historial y los siguientes pasos.

## Desarrollo local

Requisitos: Rust 1.94+, Node 22.13+ y npm. En dos terminales desde la raíz:

```sh
cargo run --locked -p mostro-community-api
```

```sh
npm --prefix web ci
npm --prefix web run dev
```

Abrir `http://127.0.0.1:5173`. Los datos locales se guardan en `var/config/`, ignorado por Git. `.env.example` documenta las variables opcionales; no se carga automáticamente. No se necesitan nodos ni claves para editar el borrador.

## Compilar y verificar

```sh
./scripts/check.sh
docker build -f docker/Dockerfile -t mostro-community:preview .
./scripts/container-smoke.sh mostro-community:preview
./scripts/daemon-contract-smoke.sh mostro-community:preview
```

El script de contenedor utiliza datos temporales; comprueba UI, CSP, guardado y persistencia tras reinicio. El workflow [`publish.yml`](.github/workflows/publish.yml) compila y prueba imágenes nativas amd64 y arm64 al publicar un tag `v*`. Solo después de verificar que GHCR las entrega sin credenciales abre el PR que promueve la nueva versión en el repositorio de la tienda. Ver el [proceso de publicación](docs/release-process.md) antes de crear un tag.

Las imágenes GHCR deben ser públicas para que Umbrel pueda descargarlas sin credenciales. La visibilidad del repositorio no convierte automáticamente sus paquetes en públicos.

[Arquitectura](docs/architecture.md) · [Validación](docs/validation.md) · [Identidad visual y dirección de interfaz](docs/brand-and-ui-direction.md) · [Blueprint original](docs/reference/blueprint-v4.md)

## Integrar una app con tu nodo

[Guía técnica de integración de apps](docs/INTEGRACION-APPS.md): tarjeta de la comunidad, eventos del nodo, protocolo de mensajes, reglas para crear órdenes y diagnóstico de errores como «prima no válida».

## Documentación operativa

[Resolver errores de Mostrix y discrepancias con el daemon](docs/mostrix-troubleshooting.md).

[Identidad y respaldos](docs/identity.md) · [LND de Umbrel](docs/lnd.md) · [Revisión previa de Mostro](docs/mostro-preflight.md) · [Convivencia con una instancia existente](docs/coexistence.md)

## Licencia

Copyright (C) 2026 Man De Bitcoin.

Mostro Community Manager es software libre: puedes redistribuirlo y modificarlo bajo los términos de la [GNU General Public License](LICENSE) publicada por la Free Software Foundation, versión 3 o (a tu elección) cualquier versión posterior (GPL-3.0-or-later). Se distribuye sin ninguna garantía; consulta la licencia para los detalles.

La imagen incluye el daemon oficial `mostrod` del proyecto [MostroP2P](https://github.com/MostroP2P/mostro), sin modificar y también GPL-3.0-or-later. Su licencia y su aviso van en `/usr/share/licenses/mostrod/` dentro de la imagen; ver [`config/upstream/NOTICE-mostrod.md`](config/upstream/NOTICE-mostrod.md).
