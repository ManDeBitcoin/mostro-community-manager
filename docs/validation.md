# Validación de esta entrega

Comprobaciones ejecutadas el 2026-09-27:

- `cargo test --workspace --locked`: 8 tests pasan. Incluyen configuración inválida, campos desconocidos/secretos, unidades y escape TOML, persistencia y copia previa, archivo corrupto, CSRF, conflicto de revisiones y estados de infraestructura sin configurar.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: sin advertencias.
- `cargo fmt --all -- --check`: correcto.
- `npm --prefix web run build`: TypeScript y Vite compilan.
- `docker compose config --quiet`: composición de desarrollo válida.
- Chromium headless sobre Vite + API reales, con `CONFIG_DIR` aislado en `/tmp`: carga dashboard, crea comunidad EUR y método de pago, guarda y verifica 0.6% → 60 bps y 30% → 3000 bps, recarga, comprueba persistencia, indicador de cambios y cancelación del descarte, y verifica anchura móvil de 390px sin desbordamiento. Sin errores JavaScript.

El test de navegador detectó y permitió corregir una incompatibilidad de Origin/Host del proxy de Vite. Se conserva Host con `changeOrigin: false`; la API sigue rechazando orígenes ajenos. La captura `screenshots/dashboard.png` muestra el estado inicial sin servicios conectados.

Pendiente: instalación final desde la UI de Umbrel, llamadas a LND/Mostro reales, regtest de trades/bonds, Mostro App, backup/restauración o upgrades. Esas integraciones siguen pendientes; no se ha tocado ningún nodo existente.

## Publicación del paquete Umbrel

La primera compilación de `v0.1.0-preview.1` se detuvo porque la imagen oficial de Rust no incluye rustfmt. La corrección instala explícitamente rustfmt y Clippy antes de ejecutar los checks. La publicación ahora exige verificar anónimamente el manifiesto multi-arquitectura, los manifiestos amd64/arm64, las configuraciones y el acceso HEAD a todas las capas. Una imagen privada o inexistente impide anunciar la release.

### Resultado de publicación — 0.1.0-preview.4

- [Workflow completo aprobado](https://github.com/ManDeBitcoin/mostro-community-umbrel/actions/runs/36302699252): builds nativas Linux amd64 y arm64, ocho tests Rust por arquitectura, formato, Clippy, build web y smoke test de contenedor.
- El smoke test verifica archivos estáticos, CSP, guardado por mismo origen, estado aislado y persistencia tras reiniciar Docker. Incluye inicialización del directorio raíz de datos y consulta del nuevo puerto temporal tras el reinicio.
- Chromium sobre la distribución estática real: guardado/recarga, fees, catálogo, cambios sin guardar y vista móvil sin errores.
- Verificación anónima repetida desde el servidor: HTTP HEAD/GET del índice, ambos manifiestos, configs y todas las capas. No requiere credenciales GHCR.
- Digest multi-arquitectura: `sha256:bbf12029ceb7f3e6f41ab26a676d1579609dd98176d1db99526b834aaf176a20`. Ambos servicios del paquete utilizan ese mismo digest.

## Conexión de lectura LND — preview.6

- Pruebas Rust: 16 pasan (siete unitarias, cuatro de configuración y cinco HTTP). Incluyen validación de URL TLS, preservación de satoshis como cadenas, ausencia de datos ficticios y transmisión bidireccional del puente TCP.
- Prueba HTTPS sintética: nueve escenarios, incluidos certificado no confiable, nombre TLS incorrecto, macaroon en cabecera, error de permiso, redirección, respuesta excesiva, saldo no disponible y resolución privada del puente.
- Compilación web y Clippy: correctos.
- Comprobación contra el LND real del operador: `online`, mainnet, cadena y grafo sincronizados y saldo de canales disponible. Se consultó con el macaroon readonly y TLS; no se mostraron ni guardaron credenciales o saldos en el registro de prueba.
- Puente TCP conectado al LND real desde un puerto local de prueba: mismo resultado `online`. El proceso de prueba se detuvo tras la comprobación.
- El workflow de publicación ejecuta la prueba HTTPS sintética dentro de cada imagen nativa, además del smoke test de persistencia y la verificación de descarga anónima.

## Preflight Mostro — preview.7

- 18 pruebas Rust pasan. El preflight lee el borrador y deriva únicamente el npub de la identidad importada; la prueba confirma que nunca incluye el nsec en la respuesta. No cambia datos.
- Clippy, formato Rust y compilación web: correctos.
- El binario oficial Mostro v0.18.8 para x86_64 coincidió con el SHA-256 fijado (`9fa0516a79270dec2dbe074f14cf988c0311a9d56c80f0a00a98e8d579b3cf53`) y respondió `mostro p2p 0.18.8` al ejecutar `--version` con `TERM=xterm`.
- Preflight local de la instalación real: borrador válido en revisión 4, identidad pública coincidente con el npub que proporcionó el operador, LND mainnet sincronizado y **cero canales activos**. El comando no imprimió secretos ni montó credenciales financieras.
- El workflow de publicación verifica el binario upstream en cada arquitectura. No ejecuta Mostro conectado a LND ni realiza pagos.

## Candidato TOML sin secretos — desarrollo posterior a preview.7

- El renderer fija `nsec_privkey` a una cadena vacía, deja RPC deshabilitado y no agrega un token RPC. Solo acepta un origen LND HTTPS sin credenciales y rutas absolutas para certificado y macaroon.
- Las pruebas comprueban el escape de texto, los porcentajes y que sobrevivan los límites upstream `allow_node_change=false`, `escrow_deadline_margin_blocks=24` y transporte `nip44`.
- `./scripts/check.sh` pasó: 18 pruebas Rust, Clippy sin advertencias, formato, build web y validación de Compose. La consulta al LND real y el arranque de Mostro quedan fuera de esta prueba.

## Lanzador privado de la imagen opcional — desarrollo posterior a preview.7

- `scripts/mostro-entrypoint-smoke.sh` pasa con un binario sintético: acepta identidad privada y settings presentes; rechaza permisos amplios, enlaces simbólicos, settings ausentes y el secreto pasado en el entorno de Docker.
- La CI ahora compila la imagen final de Mostro, comprueba `--version` y exige que rechace un arranque sin archivos. La imagen no se publica ni se incorpora a la aplicación Umbrel. La construcción Docker local no se pudo ejecutar desde este entorno sin la contraseña de `sudo`; la prueba de imagen real se comprobará en CI cuando se publique un tag.
- `scripts/verify-mostro-image.sh` permite al operador ejecutar esa misma prueba con `sudo` en el host. Se comprobó su sintaxis, el smoke del lanzador y el flujo del script con un Docker simulado; la construcción real queda pendiente de la ejecución con Docker privilegiado.

El operador ejecutó el script con `sudo` y obtuvo: `Verificación aprobada: imagen oficial v0.18.8 y arranque protegido.` La imagen de prueba se eliminó al finalizar; no se arrancó un servicio Mostro.

## Backup cifrado previo al mercado — desarrollo posterior a preview.7

- La API local puede exportar la identidad y el borrador en un archivo `age` con frase interactiva y permisos privados, y verifica el descifrado antes de publicar el archivo. `verify-backup` valida versión, `npub` derivado del `nsec` y configuración sin restaurar archivos.
- La prueba usa una identidad sintética, comprueba el viaje cifrado y descifrado, permisos de archivo/directorio y rechazo de una frase incorrecta. El mercado y LND no intervienen. La restauración de la configuración y el respaldo de la futura base de datos siguen pendientes.

## Publicación de preview.8

- [GitHub Actions aprobó](https://github.com/ManDeBitcoin/mostro-community-umbrel/actions/runs/36331949528) las imágenes nativas amd64 y arm64, las pruebas Rust, smoke de persistencia, conexión LND sintética y arranque protegido de la imagen opcional de Mostro.
- El manifiesto multi-arquitectura de Manager tiene digest `sha256:09834e540a209147e23040acdcbf1752f74b690cbce7420e004095813bab6aca`; la CI verificó acceso anónimo a los manifiestos y capas antes de crear la release.

## Restauración aislada — desarrollo posterior a preview.8

- La prueba con identidad sintética exporta el backup, lo restaura en un directorio nuevo, comprueba `npub`, revisión original y permisos 0700, y confirma que un destino existente permanece intacto.
- `./scripts/check.sh` pasa con 19 pruebas Rust, Clippy, build web, validación Compose y smoke del lanzador. No se restauró el backup real ni se cambió la instalación Umbrel.

## Preparación aislada de configuración Mostro — desarrollo posterior a preview.8

- `mostro-community-api stage-mostro-settings <origen>` genera un `settings.toml` inerte por revisión en `CONFIG_DIR/staging/revision-N/` con permisos `0700` de directorio y `0600` de archivo.
- Valida que el origen gRPC use HTTPS y puerto explícito, y rechaza rutas inseguras o preparar dos veces la misma revisión.
- No monta el macaroon financiero ni lee la identidad ni toca el daemon Mostro.
- `./scripts/check.sh` pasa con 21 pruebas Rust (incluyendo 2 de staging), Clippy sin advertencias, build web, validación Compose y smoke del lanzador.

## Publicación de preview.9

- [GitHub Actions aprobó](https://github.com/ManDeBitcoin/mostro-community-umbrel/actions/runs/36340721380) las imágenes nativas amd64 y arm64, 23 pruebas Rust por arquitectura, Clippy, build web, smoke de contenedor/persistencia, conexión LND sintética y arranque protegido de la imagen de Mostro.
- El manifiesto multi-arquitectura tiene digest `sha256:c0c14d416ecea7940f1ac11e5bdcc8bf6b2111bef87c6e5eb01b69f96107f2d0`; la CI y el host verificaron acceso anónimo a los manifiestos y capas antes de fijarlo en Umbrel.
- La versión incluye endpoint `GET /api/connection`, CLI `connection-info`, tarjeta web con QR en SVG, `nprofile` y `nostr:` URI para conexión directa con Mostro App.

## Orquestación del Demonio Mostro (Módulo 1) — preview.10

- 27 pruebas Rust pasan (`cargo test --workspace --locked`). Se agregaron pruebas para la máquina de estados de `daemon.rs` (`unconfigured`, `configured_standby`, `active_ready`, `active_running`), ciclo de vida completo de activación y desactivación atómica, y endpoints HTTP `/api/daemon/*` con protección contra CSRF.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: 0 advertencias.
- `cargo fmt --all -- --check`: correcto.
- `npm --prefix web run build`: TypeScript y Vite compilan correctamente con los nuevos tipos y tarjeta de control de orquestación.
- `scripts/mostro-entrypoint-smoke.sh`: pasa exitosamente validando el modo de espera `STANDBY_IF_UNCONFIGURED=true`, el rechazo de enlaces simbólicos, permisos laxos y variables de entorno con secretos.
- `Dockerfile.umbrel` unificado: integra la descarga y comprobación de checksums SHA-256 oficiales del binario `mostrod` v0.18.8 para amd64 y arm64 (`9fa0516a...` y `034af649...`), instalando tanto el daemon Mostro como la API y la UI en la misma imagen para evitar duplicidad de repositorios en GHCR.
- Tarjeta de control web: muestra estado en vivo, revisión activa, advertencia de falta de canales LND y acciones de activación/desactivación sin comprometer la seguridad ni revelar el `nsec`.

## Simulación de Ciclo Completo P2P y Fixture Regtest (Módulo 2) — preview.11

- 31 pruebas Rust pasan (`cargo test --workspace --locked`): 10 unitarias en lib, 4 de configuración, 2 de conexión, 3 de daemon/orquestación, 6 de HTTP, 2 de staging y 4 nuevas pruebas de integración de simulación (`api/tests/simulation.rs`).
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: 0 advertencias.
- `cargo fmt --all -- --check`: correcto.
- `npm --prefix web run build`: TypeScript y Vite compilan correctamente con la interfaz del Simulador P2P, desglose financiero satoshi/fiat, actor badges y trazabilidad cronológica.
- Motor de simulación (`api/src/simulation.rs`): soporta 4 escenarios clave (`HappyPath`, `DisputeSettledForBuyer`, `DisputeRefundedToSeller`, `SellerCancellation`), cálculo matemático exacto de fees (`fee_bps`) y bonds (`bond_bps` + `base_bond_sats`, según `BondApply`), eventos Nostr cifrados (Kind 4 / Kind 38383) y ciclo de facturas Lightning Hold Invoices (`OPEN` -> `ACCEPTED` -> `SETTLED` / `CANCELED`).
- Endpoints HTTP seguros: `GET /api/simulation/scenarios` y `POST /api/simulation/run` protegidos con validación de cabecera `X-Requested-With: mostro-community`.
- CLI `simulate-trade`: permite ejecutar simulaciones locales parametrizadas desde terminal (`happy-path`, `dispute-buyer`, `dispute-seller`, `cancel`) con desglose satoshi completo.
- Fixture Docker Regtest (`docker/docker-compose.regtest.yml`): composición aislada para pruebas locales que integra Bitcoin Core 26 (regtest), Nostr RS Relay (puerto 7777), LND Alice (Mostro) y LND Bob (contraparte) en red bridge privada sin colisionar con Umbrel.
- Smoke test automatizado (`scripts/regtest-smoke.sh`): ejecuta validación de Compose, las 4 simulaciones CLI, las 4 suites de tests Rust y el build de frontend en un único script reproducible.

## Monitor de Órdenes Nostr de Solo Lectura y Auditoría de Comisiones (Módulo 3A) — Desarrollo Actual

- **49 pruebas Rust automáticas aprobadas** (`cargo test --workspace --locked`):
  - 11 unitarias en `src/lib.rs` (permisos y rechazo de symlinks en `mostro.pub`, validación de pares de claves, balances LND sin inventar ceros, túnel TCP bidireccional y backups cifrados).
  - 4 de configuración (`tests/configuration.rs`).
  - 2 de conexión e identidad (`tests/connection.rs`).
  - 3 de orquestación del daemon (`tests/daemon.rs`).
  - 13 de contratos HTTP (`tests/http.rs`), incluyendo no fuga de secretos en `GET /api/orders` y headers de protección.
  - 7 de integración real WebSocket del monitor (`tests/orders.rs`) con relay local `tokio-tungstenite`: suscripción con autor HEX, filtros EOSE, pre-EOSE stale, desempate y reemplazo de eventos, tombstones de órdenes cerradas para evitar replay de eventos pendientes viejos, preservación de snapshot tras desconexión, failover multi-relay a estado degradado y canal `watch` de reconfiguración con limpieza selectiva.
  - 7 de simulación matemática de comisiones y Hold Invoices (`tests/simulation.rs`): validación de 10,000 sats / 25 bps => 13 por parte y 26 total, casos cero/medio satélite/100M sats, y resolución explícita de facturas en disputas.
  - 2 de staging (`tests/staging.rs`).
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: 0 errores, 0 advertencias.
- `cargo fmt --all -- --check`: formato verificado y limpio (código de salida 0).
- `npm --prefix web run build`: compilación de TypeScript y Vite exitosa con 0 errores (código de salida 0).
- Smoke test sintético (`scripts/simulation-smoke.sh`): validación sintáctica de Compose regtest, 4 simulaciones CLI con verificaciones estrictas de error ante montos negativos/cero/argumentos sobrantes, tests de integración y build web (código de salida 0).
- **Prueba End-to-End con Playwright** (`scripts/playwright-smoke.cjs`):
  - Servidor API Axum real sirviendo la distribución estática de Vite en puerto dinámico efímero.
  - Relay WebSocket local simulado con `mostro-community-api mock-relay`.
  - Navegación al Dashboard y ejecución del simulador rápido: comprobación de ausencia de `TypeError` en `total_mostro_fee_sats`.
  - Navegación al Simulador P2P completo: ejecución y verificación de desglose financiero y trazabilidad por pasos.
  - Navegación al Monitor de Órdenes: verificación del estado inicial vacío/desconectado.
  - Reconfiguración dinámica en caliente vía API PUT con el relay local: verificación de suscripción en tiempo real y renderizado exacto de orden (250,000 sats, 100 EUR, estado `pending`).
  - Verificación de filtros por tipo (`sell` vs `buy`) y estado de lista vacía filtrada.
  - Comprobación de viewport móvil responsivo a 390px x 844px sin desbordamiento horizontal (`scrollWidth <= innerWidth`).
  - 0 errores JavaScript de consola en toda la sesión del navegador (código de salida 0).
- Registros reproducibles de salida y códigos de salida guardados en:
  - `/tmp/mostro-final-fmt.log` (salida 0)
  - `/tmp/mostro-final-clippy.log` (salida 0)
  - `/tmp/mostro-final-tests.log` (salida 0)
  - `/tmp/mostro-final-web.log` (salida 0)
  - `/tmp/mostro-final-smoke.log` (salida 0)
  - `/tmp/mostro-final-playwright.log` (salida 0)

## Chat Cifrado y Consola de Arbitraje Asistido (Módulo 3B) — Desarrollo Actual

- **57 pruebas Rust automáticas aprobadas** (`cargo test --workspace --locked`):
  - 11 unitarias en `src/lib.rs` (permisos y symlinks en `mostro.pub`, validación de pares de claves, balances LND sin inventar ceros, túnel TCP bidireccional, backups cifrados).
  - 8 de integración hermética de chat cifrado (`tests/chat.rs`):
    - `test_kind4_nip04_valid_decryption_and_cache`: descifrado hermético de eventos NIP-04 Kind 4 con tag `["e", order_id]` y verificación en caché en memoria.
    - `test_kind4_nip44_decryption`: soporte y descifrado de payloads NIP-44.
    - `test_kind1059_gift_wrap_decryption`: desempaquetado hermético de NIP-59 GiftWrap (Kind 1059), verificación de seal y rumor interno.
    - `test_kind4_nip04_sent_by_community`: manejo correcto de mensajes originados por la identidad de la comunidad (`is_from_me = true`).
    - `test_invalid_events_are_rejected`: rechazo de eventos con firmas inválidas, autores no coincidentes o payloads corruptos.
    - `test_chat_cache_ordering_and_limits`: ordenación cronológica por `created_at`, deduplicación por ID de evento y límites acotados de capacidad en memoria.
    - `test_http_chat_endpoint_contract`: endpoint protegido `GET /api/chat/:order_id` con validación UUID estricta y cabecera CSRF `X-Requested-With: mostro-community`.
    - `test_chat_worker_with_mock_relay`: suscripción WebSocket en background con relay mock local enviando eventos Kind 4 y validando recepción sin redes externas.
  - 4 de configuración (`tests/configuration.rs`).
  - 2 de conexión e identidad (`tests/connection.rs`).
  - 3 de orquestación del daemon (`tests/daemon.rs`).
  - 13 de contratos HTTP (`tests/http.rs`).
  - 7 de integración real WebSocket del monitor (`tests/orders.rs`).
  - 7 de simulación matemática de comisiones y Hold Invoices (`tests/simulation.rs`).
  - 2 de staging (`tests/staging.rs`).
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: 0 errores, 0 advertencias.
- `cargo fmt --all -- --check`: formato verificado y limpio (código de salida 0).
- `npm --prefix web run build`: compilación de TypeScript y Vite exitosa con 0 errores (código de salida 0).
- Consola de Mediación en UI React (`web/src/App.tsx`):
  - Nueva vista accesible desde navegación lateral (`Mediación`), topbar y enlace directo desde el Monitor de Órdenes (`Ir a Consola de Mediación` o botón `Mediar` en órdenes en estado `dispute`).
  - Visualización del historial de chat descifrado con identificación de remitente (`Comprador`, `Vendedor`, `Mediador`, `Mostro`, o `Comunidad`), marcas de tiempo y etiquetas de cifrado (`NIP-04`, `NIP-59 / NIP-44`).
  - Selector y buscador rápido por UUID de disputa y chips de disputas activas.
  - Bosquejo interactivo de acciones de arbitraje asistido (`adm-settle` para liquidar a favor del comprador y `adm-refund` para devolver al vendedor).
- Reporte detallado generado en `/tmp/mostro-gemini-module3b-report.md`.

## Notificaciones de Eventos, Backups Automáticos Offsite y Endurecimiento de Red (Módulo 4) — Desarrollo Actual

- **63 pruebas Rust automáticas aprobadas** (`cargo test --workspace --locked`):
  - 16 unitarias en `src/lib.rs`:
    - Permisos y rechazo de symlinks en `mostro.pub` y claves privadas (`identity.rs`).
    - Balances LND de precisión satoshi y orígenes HTTPS estrictos (`lnd.rs`).
    - Túnel TCP bidireccional LND (`tunnel.rs`).
    - Detección de liquidez y preflight sin fugas de secretos (`preflight.rs`).
    - Cifrado simétrico `age` y rechazo de passphrases cortas (`backup.rs`).
    - Exportación a carpeta secundaria personalizada y retención automática (`backup.rs`).
    - Inicialización de constructores y helpers de alertas (`notifications.rs`).
    - Capacidad y comportamiento de búfer circular acotado a 50 eventos (`notifications.rs`).
    - Publicación y recuperación de eventos recientes en `NotificationHub` (`notifications.rs`).
  - 8 de integración hermética de chat cifrado (`tests/chat.rs`).
  - 4 de configuración (`tests/configuration.rs`).
  - 2 de conexión e identidad (`tests/connection.rs`).
  - 3 de orquestación del daemon (`tests/daemon.rs`).
  - 16 de contratos HTTP (`tests/http.rs`):
    - Incluye `notifications_endpoint_returns_recent_and_sse_headers` (verificación de array JSON y headers de streaming `text/event-stream`).
    - Incluye `backup_status_and_trigger_contracts` (consulta de estado y disparo manual seguro con cabecera CSRF).
    - Incluye `rate_limiting_enforces_limits_and_returns_429` (bloqueo determinista al superar cuota y cabecera `Retry-After`).
  - 5 de integración y ciclo de vida de Módulo 4 (`tests/module4.rs`):
    - `test_notifications_broadcast_and_sse_stream`: difusión multicanal y consumo continuo vía stream SSE.
    - `test_relay_down_notification_emitted_on_connection_failure`: emisión automática de alerta `relay_alert` ante fallo de conexión con relay.
    - `test_dispute_notification_emitted_by_order_monitor`: emisión de alerta `dispute_alert` en tiempo real cuando una orden pasa a estado `dispute`.
    - `test_rate_limiter_token_replenishment`: reabastecimiento continuo del algoritmo Token Bucket.
    - `test_auto_backup_cycle_and_retention_policy`: ciclo completo de respaldo periódico cifrado con age y poda determinista conservando exactamente la cuota de retención configurada.
  - 7 de integración real WebSocket del monitor (`tests/orders.rs`).
  - 7 de simulación matemática de comisiones y Hold Invoices (`tests/simulation.rs`).
  - 2 de staging (`tests/staging.rs`).
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: 0 errores, 0 advertencias.
- `cargo fmt --all -- --check`: formato verificado y limpio (código de salida 0).
- `npm --prefix web run build`: compilación de TypeScript y Vite exitosa con 0 errores (código de salida 0).
- Endurecimiento de red y contenedores (`mandebitcoin-mostro-manager/docker-compose.yml`):
  - Servicio `mostro`: `read_only: true`, `tmpfs: [/tmp]`, `cap_drop: [ALL]`.
  - Servicio `init`: inicialización y permisos `0700` para `/data/backup`, `cap_drop: [ALL]`, `cap_add: [CHOWN, FOWNER, DAC_OVERRIDE]`.
  - Red privada segregada `manager_private` para aislamiento estricto de los contenedores de la aplicación.
  - Rate limiting en API Axum: algoritmo Token Bucket (60 tokens/minuto por cliente IP) con respuesta `429 Too Many Requests` y cabecera `Retry-After`.
  - Timeout middleware global a 30s con respuesta `504 Gateway Timeout`, con exclusión selectiva de `/api/notifications/sse` (que cuenta con `KeepAlive` a 15s) para evitar desconexiones prematuras de streaming.
- Interfaz de Usuario React (`web/src/App.tsx`):
  - Tarjeta en Dashboard **Alertas y Notificaciones de Eventos (SSE en vivo)**: suscripción automática a `/api/notifications/sse`, indicador visual en tiempo real de estado SSE (`En vivo (SSE)` vs `Desconectado`), badges por severidad (`info`, `warning`, `critical`), filtrado y marcas de tiempo relativas.
  - Tarjeta en Dashboard **Backups Automáticos Offsite y Persistencia**: telemetría en vivo del worker (`Activo / Periódico`), conteo de backups retenidos, último backup generado, botón para forzar backup inmediato (`POST /api/backup/trigger`) y listado histórico con fechas y tamaños.
- Reporte detallado generado en `/tmp/mostro-gemini-module4-report.md`.

## Fase Final de Integración y Operador (Módulo 5) — Validación Completa

- **73 pruebas Rust automáticas aprobadas** (`cargo test --workspace --locked`):
  - 18 unitarias en `src/lib.rs`:
    - `lnd::tests::mock_channels_report_structure_and_balances`: estructura, presencia de campos obligatorios y coherencia matemática de balances simulados.
    - `lnd::tests::parse_channels_json_calculates_totals_and_handles_flags`: cálculo exacto de capacidad total, saldo local, saldo remoto (inbound) y estado de canales activos/inactivos a partir de JSON upstream de LND REST.
    - Permisos y rechazo de symlinks en `mostro.pub` y claves privadas (`identity.rs`).
    - Balances LND de precisión satoshi y orígenes HTTPS estrictos (`lnd.rs`).
    - Túnel TCP bidireccional LND (`tunnel.rs`).
    - Detección de liquidez y preflight sin fugas de secretos (`preflight.rs`).
    - Cifrado simétrico `age` y rechazo de passphrases cortas (`backup.rs`).
    - Exportación a carpeta secundaria personalizada y retención automática (`backup.rs`).
    - Inicialización de constructores y helpers de alertas (`notifications.rs`).
    - Capacidad y comportamiento de búfer circular acotado a 50 eventos (`notifications.rs`).
    - Publicación y recuperación de eventos recientes en `NotificationHub` (`notifications.rs`).
  - 17 de contratos HTTP (`tests/http.rs`):
    - `lnd_channels_endpoint_contract`: verificación del endpoint `GET /api/lnd/channels` respondiendo 200 OK con datos estructurados de prueba (`?mock=true`) y comportamiento seguro de sólo lectura (`unconfigured` sin credenciales).
    - `health_never_fabricates_connected_services`.
    - `backup_status_and_trigger_contracts`.
    - `daemon_endpoints_enforce_protection_and_report_state`.
    - `invalid_configuration_is_not_persisted`.
    - `notifications_endpoint_returns_recent_and_sse_headers`.
    - `orders_endpoint_reports_snapshot_without_secrets`.
    - `mutation_requires_same_origin_custom_header_and_fresh_revision`.
    - `rate_limiting_enforces_limits_and_returns_429`.
    - `secrets_and_unknown_fields_are_rejected`.
    - `simulation_contract_enforces_protection_headers`.
    - `simulation_contract_accepts_defaults_with_valid_config`.
    - `simulation_contract_rejects_invalid_trade_amounts`.
    - `imported_identity_is_not_exposed_over_http`.
    - `simulation_scenarios_contract_returns_metadata`.
    - `simulation_contract_rejects_unknown_scenarios_and_fields`.
    - `simulation_contract_requires_configured_community`.
  - 8 de integración hermética de chat cifrado (`tests/chat.rs`).
  - 4 de configuración (`tests/configuration.rs`).
  - 2 de conexión e identidad (`tests/connection.rs`).
  - 3 de orquestación del daemon (`tests/daemon.rs`).
  - 5 de integración y ciclo de vida de Módulo 4 (`tests/module4.rs`).
  - 7 de integración real WebSocket del monitor (`tests/orders.rs`).
  - 7 de simulación matemática de comisiones y Hold Invoices (`tests/simulation.rs`).
  - 2 de staging (`tests/staging.rs`).
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: 0 errores, 0 advertencias.
- `cargo fmt --all -- --check`: formato verificado y limpio (código de salida 0).
- `npm --prefix web run build`: compilación de TypeScript y Vite exitosa en < 1.1s con 0 errores (código de salida 0).
- **Verificación de Ciclo Completo (E2E) Automatizada (`scripts/verify-e2e.sh`)**:
  - Ejecución de 73 tests de Rust y build del frontend.
  - Arranque automático del servidor en entorno mock aislado temporal con permisos 0700/0600.
  - Validación con `curl` de respuestas 200 OK y esquemas JSON esperados en:
    - `GET /api/health` -> `{"status":"ok",...}`
    - `GET /api/orders` -> `{"state":..., "orders":[...]}`
    - `GET /api/notifications/sse` -> `Content-Type: text/event-stream` con `KeepAlive`
    - `GET /api/chat/:id` -> `403 Forbidden` sin cabecera CSRF, `200 OK` con `X-Requested-With: mostro-community`
    - `GET /api/backup/status` -> `{"interval_secs":..., "backups":[...]}`
    - `GET /api/lnd/channels` -> `{"status":"online", "channels":[...]}`
    - `GET /api/simulation/scenarios` -> `[{"id":"happy_path",...}]`
    - `GET /` -> Frontend estático con `<div id="root">`
  - Ejecución exitosa de simulación sintética CLI `simulate-trade happy-path 50000`.
  - Parada limpia de procesos y eliminación de archivos temporales mediante `trap`.
- **Preparación de Nodos Lightning (Operador)**:
  - Nueva pestaña en frontend React: `Operaciones LND` (`LiquidityOperationsPage`) con tarjetas métricas de capacidad total, saldo local vs remoto (inbound), porcentaje de distribución, alerta de suficiencia de liquidez, tabla interactiva de canales abiertos con copia de puntos de canal, y selector de modo en vivo vs demostración.
  - Manual operativo paso a paso integrado en la UI explicando la regla de oro de la liquidez en Mostro (por qué se necesita Inbound para depósitos de vendedores), procedimiento de apertura con Alby Hub / LSPs, canales salientes hacia routing nodes (ACINQ, Kraken) y mantenimiento.
  - Endpoint `GET /api/lnd/channels` implementado en Axum (`lnd.rs` y `http.rs`) con cálculo exacto de balances satoshi en uint128 sin overflow y soporte para modo demostración (`?mock=true`).
- **Despliegue Final y Producción**:
  - Manifiesto `mandebitcoin-mostro-manager/umbrel-app.yml` actualizado a versión de Producción `1.0.0`, con descripciones completas, release notes, dependencia única `lightning` y categoría `bitcoin`.
  - Guía completa de despliegue y manual de operaciones redactado en `docs/DEPLOYMENT.md`.
- Reporte detallado generado en `/tmp/mostro-gemini-module5-report.md`.

## Mostro v0.19.2 e interoperabilidad con apps (2026-10-03 y 2026-10-04)

Validación previa a la versión 1.0.12. El nodo en producción no se tocó: todo se ejecutó en un entorno regtest aislado en el mismo equipo, con puertos locales y una identidad desechable.

### Binario y configuración

- Binarios oficiales `mostrod` v0.19.2 descargados de la release de GitHub. SHA-256 calculados sobre los artefactos y coincidentes con `manifest.txt`: amd64 `4d9aa45bbbca12a16024d72d2bf4f5fcb243196226750c50b741d035ffc2c071`, arm64 `73c69e18f9d417e2e23347770b68be942eb1e28fd69d7180a54d18e9409d6c31`.
- Firma GPG de `manifest.txt` válida con la clave `1E41631D137BA2ADE55344F73852B843679AD6F0` (negrunch). La clave no está certificada por una cadena de confianza propia.
- El binario amd64 responde `mostro p2p 0.19.2`. El de arm64 no se ejecutó.
- `settings.toml` generado por el Manager y cargado por el binario real: `Settings correctly loaded!` y `Transport: nip44 (protocol v2, event kind 14)`.
- Entre v0.19.0 y v0.19.2 no cambian las migraciones de base de datos ni `proto/admin.proto`, y la plantilla de configuración solo gana un bloque comentado.

### Ciclo completo en regtest

Bitcoin Core 31.1, tres nodos LND 0.21.3-beta (nodo de Mostro, vendedor y comprador) con canales abiertos, un relay Nostr local y un cliente de prueba de protocolo v2 en modo de privacidad total.

- Trece variantes de `new-order`, con los resultados de la tabla de `docs/INTEGRACION-APPS.md`, sección 6.2. La combinación de sats fijos y prima distinta de cero devuelve `cant-do: invalid_parameters`; con `amount = 0` la orden se acepta.
- Operación de venta de 50 USD con prima +5 % y comisión 0,6 %: orden de 56 076 sats, factura retenida de 56 244 sats pagada por el vendedor y 55 908 sats recibidos por el comprador (factura `SETTLED`).
- Disputa abierta por el comprador, tomada con `admin-take-dispute` y resuelta con `admin-settle` usando la clave del nodo. Eventos kind 38386 `initiated`, `in-progress` y `settled`.
- El Manager, conectado al mismo relay, mostró las órdenes con sus estados (`pending`, `in-progress`, `success`, `canceled`), la disputa enlazada con su orden, los 26 mensajes de protocolo de la orden disputada y la versión 0.19.2 anunciada por el daemon.
- Los eventos y mensajes capturados se guardan como fixtures en `api/tests/fixtures/mostrod-v0.19.2/` y los usan las pruebas de `api/tests/orders.rs` y `api/tests/chat.rs`.
- Supervisión real: con `docker/mostro-entrypoint.sh` en modo de espera y el binario v0.19.2, la activación desde la API del Manager arrancó el daemon; al faltar el certificado de LND el daemon salió con código 1, el supervisor registró la caída y reintentó con pausa, y el panel informó «terminó con código 1 a los 1 s de arrancar y se está reiniciando». La desactivación desde la API devolvió el supervisor a la espera.

### Ampliación del 4 de octubre

Mismo entorno, con dos nodos `mostrod` v0.19.2: el anterior y otro con garantía del 3 % para ambas partes, mínimo de 1 000 sats y `pow_first_contact = 8`. Los dos `settings.toml` los generó el Manager. Todos los escenarios terminaron con el resultado esperado.

Nodo sin garantía, cliente en privacidad total:

- Venta tomada con la factura sin importe adjunta en `take-sell`: un solo `pay-invoice` al vendedor, estados públicos `pending` y `success`, y 35 078 sats cobrados sobre 35 184.
- Orden de rango de 20 a 60 USD: tomarla sin importe devuelve `cant-do: out_of_range_sats_amount`; con `{"amount": 30}` se completa.
- Orden de compra tomada con `take-buy` hasta `success`.
- Disputa abierta por el vendedor y resuelta con `admin-cancel`: factura retenida `CANCELED`, disputa `seller-refunded`, orden `canceled`.
- Cancelación de mutuo acuerdo con la operación activa: el depósito sigue retenido tras el primer `cancel` y se cancela con el segundo.
- Tomar la propia orden (`invalid_pubkey`), tomar una orden ya tomada (`invalid_order_status`) y actuar sobre una orden inexistente (`not_found`).
- Quien toma se retira antes de enviar la factura: la orden vuelve a publicarse `pending` y quien publicó recibe de nuevo `new-order`.
- Sin respuesta: `take-sell` sin `id`, `new-order` sin payload y un evento con `created_at` 60 s atrás. Con 8 s de antigüedad se aceptó.

Nodo con garantía y prueba de trabajo de primer contacto:

- El evento de información anuncia los tags `bond_*`, `pow = 0` y `pow_first_contact = 8`, y el Manager los lee.
- `new-order`, `take-sell` y `admin-take-dispute` sin prueba de trabajo no reciben respuesta. Con ella se aceptan, y los mensajes siguientes de esa clave ya no la necesitan.
- Garantía de quien publica: la respuesta a `new-order` es `pay-bond-invoice` (1 056 sats, 900 s de vigencia) y la orden no se publica hasta pagarla. Garantía de quien toma: 300 s de vigencia, y la orden sigue `pending` hasta pagarla.
- Venta completa con ambas garantías: depósito `SETTLED` y las dos garantías `CANCELED`, es decir, devueltas. Lo mismo tras una disputa resuelta con `admin-settle` sin penalización.
- Dos tomas simultáneas de una misma orden: ambas reciben `pay-bond-invoice`, gana la primera garantía pagada y la otra parte recibe `canceled` con su factura de garantía `CANCELED`. Una tercera toma devuelve `invalid_order_status`.
- Penalización: disputa resuelta con `admin-cancel` y `bond_resolution` contra el comprador. Garantía del comprador `SETTLED` (1 054 sats) y `bond-slashed`; el vendedor recibe `add-bond-invoice` por 527 sats, envía su factura y recibe `bond-invoice-accepted` y `bond-payout-completed` con la factura `SETTLED`. Su propia garantía queda `CANCELED`.
- Orden de compra creada con la factura del comprador incluida: no hay petición `add-invoice` y el comprador cobra al liberarse la operación.
- Vencimiento: quien toma no envía su factura. A los 915 s, con `expiration_seconds = 900`, recibe `canceled`, quien publicó recibe de nuevo `new-order`, la orden vuelve a `pending` y la garantía de quien tomó queda `CANCELED`.

Modo de reputación, con un cliente propio que no usa mostro-core:

- Venta completa con clave de identidad, una clave por operación, `trade_index`, firma interna y prueba de identidad. Confirma la serialización canónica y el texto de la prueba de identidad descritos en la guía, que incluye un vector verificable.
- `trade_index` repetido: `cant-do: invalid_trade_index`. Firma interna o prueba de identidad alteradas: sin respuesta.
- Valoraciones: `rate-user` devuelve `rate-received`. No hay respuesta si la otra parte opera en privacidad total ni en una segunda valoración. La siguiente orden de la identidad valorada anuncia el tag `rating` actualizado. Los eventos kind 38384 salieron en el lote horario del daemon, con `d` igual a la clave de operación de quien valoró.
- Una valoración enviada 135 s después del cierre sin prueba de trabajo no recibe respuesta. Con la prueba se acepta. El panel avisa ahora cuando `pow_first_contact` supera a `pow`.

El Manager, conectado a ese relay, mostró las cinco órdenes del nodo con garantía, su disputa enlazada con la orden y los 26 mensajes de la operación en modo de reputación. Las capturas nuevas se añadieron a los fixtures.

A petición de la sesión que adapta la app, el tráfico cifrado de esas pruebas se descifró después con las claves desechables de los dos nodos de regtest y se guardó por flujos en `protocol-flows.json`: compra, compra con factura incluida, venta con factura adjunta, rango, cancelación de mutuo acuerdo, retirada de quien tomó, vencimiento, tomas simultáneas y disputa con `admin-cancel`. Conserva el orden de llegada al relay y el segundo de cada mensaje. La consola del Manager reproduce los nueve en las pruebas, con el mismo orden sea cual sea el orden de entrega. De ese tráfico sale la tabla de qué mensajes repiten el `request_id`, sección 5.5 de la guía.

### Tarjeta de la comunidad

- La tarjeta emitida por el Manager, en JSON y como enlace `mostro://community/<base64url>`, verifica con una copia del verificador de la app BitMaxis (`k256`, `verify_raw` sobre el digest de la cadena canónica). Antes del cambio no verificaba.

### Comprobaciones del repositorio

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` y `cargo test --workspace --locked` (124 pruebas) con Rust 1.97 y con Rust 1.94.0, la versión de la imagen de compilación. Las suites asíncronas se repitieron 20 veces sin fallos.
- Revisión independiente de los cambios en seis frentes (backend, seguridad, script supervisor y empaquetado, interfaz, guía de integración y pruebas), con verificación de cada hallazgo. Los hallazgos se corrigieron antes de cerrar esta validación.
- Segunda revisión independiente de los cambios del 4 de octubre: sin hallazgos bloqueantes ni mayores. De los menores se corrigieron los de la consola (texto de un remitente en una sola línea y sin caracteres de control, etiquetas del daemon solo para mensajes del daemon, cupo de rechazos, orden de las respuestas), el script de humo, el workflow y la prueba de versiones fijadas, que ahora también corre fuera de la imagen.
- `npm --prefix web run build` y `python3 -m unittest discover -s scripts/tests`, que incluye la coherencia de la versión y los checksums de mostrod entre `config/versions.json`, los Dockerfile y los scripts de humo.
- `sh scripts/mostro-entrypoint-smoke.sh`, ampliado con el modo de espera: salida con error ante un arranque rechazado, registro de caídas con pausa entre reintentos, reinicio por petición del panel y parada al retirar la configuración.
- Interfaz comprobada con Chromium sin errores de consola contra el daemon de regtest.
- Imagen Docker: este equipo no da acceso al socket de Docker a la sesión de desarrollo, así que se construyó en GitHub Actions con el workflow `image-check.yml` del PR de esta versión. Pasó en amd64 y arm64: construcción con las comprobaciones de Rust dentro de la imagen, `scripts/container-smoke.sh`, `scripts/lnd-smoke.py` y `scripts/verify-mostro-image.sh`.
- Contexto de compilación de la imagen simulado sin Docker: con solo `Cargo.toml`, `Cargo.lock`, `api/` y `config/`, y Rust 1.94.0, pasan el formato, las pruebas, clippy y la compilación de release. `npm ci` y la compilación del frontend pasan desde los archivos versionados. El binario de release responde `{"mode":"release","version":"1.0.12"}` en `/api/health` cuando se compila con `MANAGER_VERSION=v1.0.12`.

### No verificado

- Penalización automática por vencimiento, vencimiento cuando falla quien publicó, modo mantenimiento, restauración de sesión, Cashu y Serbero. Su descripción procede del código de v0.19.2.
- La app BitMaxis en ejecución. Sus hallazgos proceden de leer su código.
- La actualización del nodo en producción de v0.19.0 a v0.19.2.

## Reorganización del panel web (2026-10-05)

El panel pasa de una sola pantalla larga a diez páginas en cinco grupos, con una dirección por página. No se añade ni se quita ningún endpoint. En el servidor cambian tres cosas: `/api/daemon/status` da a cada aviso un código estable (`notices`), informa del tiempo que lleva el daemon en ejecución (`running_for_secs`) y devuelve `null`, no cero, en los datos de LND que no ha podido leer; los avisos y las alertas hablan con el vocabulario del panel; y un respaldo manual conserva los mismos archivos que uno automático.

### Cómo se probó

Copias aisladas del panel en puertos locales, cada una con su propio directorio de configuración y de respaldos temporal, y sin las variables que las conectarían a LND, al daemon, a un webhook o a una identidad existente:

- **Instalación vacía:** sin identidad ni reglas.
- **Lista para activar:** identidad y reglas guardadas, con los eventos reales capturados del daemon de regtest (9 órdenes y 2 disputas resueltas) servidos por un relay local.
- **Mercado abierto:** configuración activada, un anuncio reciente del nodo, ofertas publicadas, operaciones en curso y dos disputas abiertas, firmados con la clave desechable del nodo de regtest en un relay local. El proceso del daemon se simuló con su archivo de latido. LND se sustituyó por un servidor HTTPS local que responde a las tres consultas de solo lectura del panel con datos inventados: 3 canales activos y 1 inactivo.
- **Respaldo automático activo:** la misma configuración con `BACKUP_PASSPHRASE` definida.

### Qué se comprobó

- Las diez páginas en las tres primeras copias, a 1440 px y a 390 px de ancho, sin errores en la consola del navegador. Ninguna página se desborda a lo ancho a 320, 360, 390, 640, 860, 861, 1050, 1051, 1280, 1440 ni 1920 px.
- `scripts/playwright-smoke.cjs`, reescrito para la nueva estructura: 28 pasos sin errores de consola. Arranca su propio relay simulado y dos copias del panel en puertos efímeros, y recorre:
  - La puesta en marcha completa desde una instalación vacía: clave inválida rechazada, identidad nueva que se muestra una sola vez y que el servidor no repite, reglas, plantilla, guardado, respaldo cifrado, conexión de apps, y activación y desactivación con sus confirmaciones.
  - El libro de órdenes con sus filtros en la dirección, los enlaces desde cada cifra del resumen, el simulador con sus importes y la página de Lightning sin acceso a LND.
  - Todas las páginas desde el menú, el enlace de salto, una dirección desconocida y una malformada.
  - Con respuestas simuladas del servidor: los cinco estados del mercado, el destino de cada aviso según su código, una lectura fallida y la página de disputas al usar Atrás y Adelante.
  - El uso desde un teléfono: recargar no cambia de página, nada se desborda y el menú se maneja con teclado.
- La prueba de disputas falla si se retira la corrección que impide emparejar una disputa con la orden de otra, y vuelve a pasar al restaurarla.
- Una configuración activada sin daemon que se anuncie no se presenta como mercado abierto: el panel dice «Sin confirmar». Un anuncio reciente sin Mostro activado aquí se presenta como «Anuncio vigente».
- Con LND ilegible el panel no muestra saldos ni dice «0 canales». Con LND legible muestra canales, liquidez y la estimación de operaciones simultáneas, calculada con los canales activos.
- `verify-backup` y `restore-backup`, ejecutados tal como los muestra la página de respaldos sobre un respaldo de prueba: la restauración dentro de la carpeta de respaldos crea `community.json` e `identity/` con permisos privados, y bajo una carpeta que no es privada el comando la rechaza con «El directorio padre debe ser privado (0700)».
- Las hojas de estilo perdieron 199 selectores que ningún componente usaba. Se compararon 60 capturas de antes y después, las diez páginas en tres copias y a dos anchos: sin diferencias.
- `./scripts/check.sh` completo: formato, `cargo test` (127 pruebas), clippy, las pruebas de Python (10, con la nueva que exige que el panel conozca todos los códigos de aviso del servidor), la compilación del frontend y el smoke del supervisor. Formato, clippy y pruebas se repitieron con Rust 1.94.0, la versión de la imagen de compilación.
- Dos revisiones independientes antes de confirmar los cambios. Una leyó el código de la interfaz en busca de regresiones respecto al panel anterior. La otra contrastó cada afirmación de la interfaz y de la documentación con el código del servidor. Cada una encontró un defecto bloqueante: la guía de disputas podía mostrar, tras usar Atrás, el comando de resolución con la orden de otra disputa; y el comando de restauración documentado usaba un destino que el paquete de Umbrel rechaza. Los dos, los once hallazgos mayores y los menores se corrigieron, y los que se podían probar quedaron cubiertos por la prueba de navegador.

### No verificado

- El panel reorganizado contra el nodo en producción ni contra un daemon real en ejecución: el estado «mercado abierto» se reprodujo con eventos firmados y un latido de proceso simulado.
- La imagen Docker con estos cambios. Este equipo no da acceso al socket de Docker a la sesión de desarrollo; la construye el workflow `image-check.yml` cuando la rama se sube y se abre su PR.
- El modo de permisos de `/data` en una instalación real de Umbrel. Las instrucciones de restauración no dependen de él: usan la carpeta de respaldos, que el paquete crea privada.
- Lectores de pantalla. Se comprobó el manejo con teclado del menú y de los diálogos, no una tecnología de apoyo real.
- Navegadores distintos de Chromium.
- Lo que el panel afirma sobre el comportamiento de `mostrod` y no se ejecutó aquí: el modo mantenimiento, la penalización automática por vencimiento y los comandos de `mostro-cli` de la guía de disputas.

## Publicación de la tarjeta de la comunidad en los relays (2026-10-05)

El panel puede publicar la tarjeta firmada de la comunidad como evento Nostr kind 30078 con `d = mostro-community-card`. Lo decide el operador con un interruptor de **Conexión de apps** que nace apagado. El formato y lo que debe comprobar una app están en `INTEGRACION-APPS.md`, sección 3.3. Es el único evento que el panel firma con la clave del nodo.

### Cómo se probó

Sin ningún relay real. Todos los relays fueron simulados en `127.0.0.1`, en puertos del rango 39xxx, y las copias del servidor usaron directorios de configuración y de respaldos temporales, sin las variables que las conectarían a LND, al daemon, a un webhook o a una identidad existente. Las claves fueron sintéticas o generadas para la prueba.

### Qué se comprobó

- `api/tests/card_publication.rs`, 22 pruebas contra relays simulados:
  - La forma del evento: kind 30078, autor la clave del nodo, un solo tag `d`, sin `expiration`, y como contenido la tarjeta en JSON compacto con su firma. Verifican la firma del evento y la de la tarjeta.
  - Con el interruptor apagado no se abre ninguna conexión ni se firma nada. Con el interruptor encendido y sin identidad, sin reglas o con una configuración ambigua, tampoco, y el estado dice el motivo.
  - Una tarjeta sin cambios conserva el mismo evento, byte a byte, entre reenvíos, tras un guardado que no la altera y tras reiniciar. Una tarjeta cambiada recibe un evento nuevo con fecha estrictamente posterior, también dentro del mismo segundo.
  - El informe por relay: aceptado, rechazado con el motivo del relay, sin respuesta, sin conexión, `OK` de otro evento, `NOTICE`, petición de autenticación y cierre sin responder. Solo un `OK` afirmativo para ese evento cuenta como publicado. Con un relay que rechaza, el estado es «no publicada» o «parcial», nunca «publicada».
  - La alerta por fallo se registra una vez, al segundo intento fallido, y no en cada reintento.
  - La retirada: petición de borrado kind 5 con los tags `a`, `e` y `k` de la tarjeta y ningún otro; reintento a un relay que la rechaza, también tras reiniciar; abandono al vencer el plazo; envío a un relay que ya salió de la configuración; nueva publicación con fecha posterior a la petición.
  - Casos límite: un evento que no se puede guardar en disco no se envía; una clave que no se puede leer al apagar retrasa la retirada en lugar de perderla; al cambiar la identidad el panel avisa de que la tarjeta anterior sigue en los relays; un evento del nodo que no es una tarjeta, colocado en el archivo de estado, no recibe petición de borrado.
  - Por HTTP: el interruptor exige la misma protección que guardar las reglas, rechaza campos desconocidos, no funciona sin reglas guardadas, no cambia la revisión ni la copia de la revisión anterior, y sobrevive a un guardado. Con el trabajador en marcha, encender el interruptor y guardar un cambio publican sin más intervención.
- Once alteraciones deliberadas del código. Diez hicieron fallar al menos una prueba: no reutilizar el evento guardado, fechar siempre con la hora actual, ignorar el interruptor, contar un rechazo como aceptación, añadir un tag de expiración, dar por perdida la retirada si la clave no se lee, enviar antes de guardar, pedir el borrado de cualquier evento, olvidar un relay retirado y reintentar sin fin. La undécima, aceptar un `OK` de otro evento, no la detectaba ninguna prueba: se añadió el caso y, repetida, ya falla.
- El vector de la guía (`api/tests/fixtures/community-card-event.json`): el `id` y la firma del evento con `nostr-tools`, y la firma de la tarjeta con `@noble/curves` siguiendo a mano la sección 3.1. Son implementaciones independientes del Manager.
- El binario real contra un relay simulado, por HTTP: nada publicado antes de encender; un evento al encender; el mismo evento, con el mismo `id` y la misma fecha, reenviado al reiniciar el servidor sin pedirlo; un evento nuevo un segundo posterior al guardar un método de pago; nada enviado al guardar un cambio que no afecta a la tarjeta; la petición de borrado al apagar. Los archivos de estado quedaron con permisos `0600` y la clave privada no apareció en los registros ni en esos archivos.
- `scripts/playwright-smoke.cjs`, ampliado a 33 pasos sin errores de consola. Antes de encender el interruptor comprueba que el único relay de la configuración es el simulado. Recorre: el interruptor apagado con su explicación; la confirmación al encender y lo que ocurre al decir que no; la publicación, con el evento que recibió el relay; la retirada; el motivo cuando no hay clave para firmar, con el aviso en el resumen y el enlace a donde se corrige; y, con respuestas simuladas, un relay que rechaza, la publicación parcial, un relay que salió de la configuración y una retirada que aún no se pudo firmar. Incluye la página a 390 px.
- `./scripts/check.sh` completo: formato, `cargo test` (153 pruebas), clippy, las 10 pruebas de Python, la compilación del frontend, la validación del Compose y el smoke del supervisor. Formato, clippy y las 153 pruebas también con Rust 1.94.0, la versión de la imagen.
- Una revisión independiente del cambio antes de confirmarlo. Encontró, entre otras cosas, que un error transitorio al leer la clave en el momento de apagar hacía perder la retirada, y que un fallo de escritura permitía enviar un evento sin guardar. Están corregidos y cada corrección tiene su prueba.

### No verificado

- Ningún envío a un relay real. No se sabe cómo responde cada implementación de relay al reenvío de un evento que ya tiene, a un evento con fecha un segundo por delante, ni a la petición de borrado NIP-09 por dirección. El panel muestra lo que responda cada relay.
- La lectura del evento por la app BitMaxis en ejecución. Su código se leyó en el commit `1167506f`: busca este kind y este `d`, y comprueba autor, firma del evento y firma de la tarjeta.
- El cambio en el nodo en producción. Llega con la siguiente versión publicada, y actualizar no publica nada mientras el interruptor siga apagado.
- La imagen Docker con estos cambios: la construye el workflow del PR.
- Relays que exigen autenticación NIP-42, más allá de que el panel muestre su petición como motivo.
- Navegadores distintos de Chromium.

## Correcciones tras la revisión de v1.0.12 (2026-10-05)

La revisión previa a v1.0.12 dejó nueve hallazgos abiertos que nadie había comprobado en el código. Los nueve se confirmaron leyéndolo antes de cambiar nada, y los nueve se sostienen. No se cambia la versión del daemon ni ningún endpoint; `/api/daemon/status` gana el campo `announced_before_start` y el código de aviso `daemon_unexpected_exit`, y `/api/connection` el campo `card_unavailable`.

### Qué se confirmó y qué cambia

- **Anuncio del proceso anterior.** `report_with_node_info` daba por reciente cualquier anuncio de menos de 660 s sin compararlo con el arranque del proceso. El supervisor escribe `mostro.pid` una vez por arranque y solo vuelve a tocar `mostro.heartbeat`, así que la edad de ese archivo es el tiempo en marcha. Ahora un anuncio más viejo que el proceso, con 5 s de margen, no cuenta como reciente ni da su versión por la del daemon. Sin archivo de pid no se descarta nada. `/api/dashboard` aplica el mismo criterio a `market_started` y al estado de Mostro.
- **Reinicio en el primer guardado.** Hasta v1.0.11 `name`, `about` y `website` se escribían siempre, también vacíos; desde v1.0.12 se recortan y se omiten si están vacíos, y el guardado comparaba el archivo byte a byte. Ahora compara el TOML leído, con esos tres campos recortados y omitidos si quedan vacíos. Un archivo equivalente no se toca; una activación explícita lo reescribe, porque reinicia de todos modos.
- **Tarjeta sin generar.** En v1.0.12 `GET /api/community/card` respondía que faltaban la identidad o la configuración también cuando existían y lo que impedía firmar era un «&» o una «,». La publicación de la tarjeta, que entró antes en esta misma versión, ya distingue los motivos. Sobre eso, esta entrega nombra el campo que hay que cambiar, en esa respuesta y en el guardado rechazado, y lleva el motivo a la página «Conexión de apps», que seguía diciendo «guarda la configuración».
- **Texto de las órdenes.** `mostrod` copia en la etiqueta `pm` el texto que escribió quien publicó la orden, partido por comas (`src/nip33.rs` de v0.19.2), y el panel lo mostraba entero. Ahora pasa por la misma limpieza que el texto del chat, con un máximo de diez métodos de sesenta caracteres; `fa` se acepta como una o dos cifras de hasta quince dígitos, que es lo que el daemon publica.
- **Salida inesperada.** El resumen solo la veía dentro del aviso de reinicio en bucle. Ahora hay un aviso propio durante una hora. Al comprobar que el supervisor solo registra salidas no pedidas apareció un caso en que no era así: si el panel detiene el daemon por su cuenta, lo que solo puede hacer cuando comparten espacio de procesos, la parada quedaba registrada como caída. El supervisor ya no la registra si hay un reinicio pedido o `settings.toml` ya no existe. A cambio, una caída real que coincida en el mismo segundo con un reinicio pedido tampoco queda registrada: el daemon se reinicia igualmente. Un código de salida mayor que 128 se presenta como la señal que es, porque en ese caso el daemon no terminó por su cuenta.
- **Mercado abierto en verde.** `marketStatus` no miraba los avisos del mismo informe.
- **Alertas.** Se leían una vez y después solo llegaban por un `EventSource` que el navegador abandona si el servidor le responde con un error.
- **Datos sin leer.** El resumen contaba ceros con el monitor en `connecting` o `syncing`, y las páginas decían «no hay reglas guardadas» antes de leerlas. Lo mismo hacían los filtros de la página de órdenes, que contaban ceros, y la de disputas, que decía «Ninguna abierta» con el monitor sin leer. Mientras las reglas no se han leído, el panel distingue entre estar leyéndolas y no haber podido.
- **Prueba de contrato.** El patrón `[a-z_]+` habría dejado pasar sin comparar un código con un dígito.

### Cómo se probó

- Pruebas de Rust nuevas en `api/tests/daemon.rs`, `orders.rs`, `http.rs` y `configuration.rs`. El proceso del daemon se simula con sus archivos de pid y de latido, con la fecha del pid retrasada para darle edad.
- El primer hallazgo se repitió por el camino real del servidor: una copia aislada de la API, un relay local, una identidad desechable creada por la propia API y un daemon que solo existe como sus archivos de pid y de latido. Con un anuncio de hace 200 s y el proceso recién arrancado, `/api/daemon/status` responde `announced_fresh: false` y `announced_before_start: true`, sin `version_mismatch` y con la versión del paquete. Al publicar el anuncio nuevo pasa a `announced_fresh: true`. Con el proceso en marcha desde hace 15 min, un anuncio con otra versión sigue dando `version_mismatch`.
- Cada corrección del servidor se deshizo por separado y su prueba falló: el anuncio anterior al arranque, la comparación de `settings.toml`, el motivo de la tarjeta, el campo nombrado al guardar, los métodos de pago, los importes y el aviso de salida.
- `scripts/mostro-entrypoint-smoke.sh` comprueba ahora que el archivo de pid no se toca mientras el latido avanza, y que una parada hecha por el panel no se registra, ni con un reinicio pendiente ni con la configuración retirada. Cada caso falla tres veces de tres sin su mitad de la corrección.
- `scripts/playwright-smoke.cjs`: 42 pasos sin errores de consola, contando los de la publicación de la tarjeta. Añade una tercera copia del panel con reglas que la tarjeta no admite, y pasos con respuestas simuladas para el reinicio, el mercado abierto con avisos de Lightning, la salida inesperada, las reglas sin leer, el monitor que aún lee los relays, las alertas sin conexión en vivo y una orden sin importe legible. Los ocho pasos que dependen del panel fallan contra el panel anterior servido por el servidor nuevo.
- `scripts/tests/test_panel_contract.py` falla con el patrón anterior.
- `./scripts/check.sh` completo sobre el árbol ya fusionado con la publicación de la tarjeta: formato, `cargo test` (160 pruebas), clippy, las pruebas de Python (14), la compilación del frontend, `docker compose config` y el smoke del supervisor. El formato, clippy y las pruebas se repitieron con Rust 1.94.0.
- La fusión con la publicación de la tarjeta chocaba en ocho archivos. El motivo de la tarjeta usa el `CardUnavailable` de esa entrega; las dos secciones de notas de v1.0.13 quedan en una sola. En el smoke, los pasos de esta entrega van antes de los que encienden la publicación en la copia de prueba, porque ese interruptor añade un aviso al resumen.
- Una revisión independiente del diff, solo de lectura, antes de confirmar los cambios. No encontró defectos bloqueantes. Señaló que la nota de versión prometía para todo el panel lo que solo cumplía el resumen, que `/api/dashboard` seguía con el criterio anterior, que la página del nodo mostraba una salida registrada con Mostro desactivado, que un código de señal se presentaba como una salida por cuenta propia y varios textos. Todo ello se corrigió y tiene prueba.

### No verificado

- Un `mostrod` real que se reinicia y se vuelve a anunciar. El caso se reprodujo con archivos de pid y de latido y con anuncios firmados por una clave desechable, y en el navegador con informes simulados.
- La imagen Docker con estos cambios, `scripts/container-smoke.sh` y el supervisor dentro del contenedor. Este equipo no da acceso al socket de Docker a la sesión de desarrollo; los ejecuta `image-check.yml` cuando la rama se sube y se abre su PR.
- El cierre de la conexión de alertas durante una actualización real detrás del proxy de Umbrel. Se reprodujo respondiendo a la conexión con un error.
- El nodo en producción. No se leyó su directorio de datos ni su API; si su `settings.toml` es todavía el que escribió v1.0.11 no se sabe.
- Navegadores distintos de Chromium.

### Queda abierto

- En la puesta en marcha, el paso del respaldo figura como pendiente mientras el estado de los respaldos aún no se ha leído. Es anterior a estos cambios y no se ha tocado.
- El monitor no comprueba la etiqueta `d` del evento de información del nodo, y la tarjeta puede firmarse con un relay cuyo esquema está en mayúsculas, que la app descarta antes de verificar. Los dos proceden de la verificación de la guía de integración y no formaban parte de esta lista.

## Mostro v0.20.0 (2026-10-16)

Validación previa a la versión 1.0.14. El nodo en producción no se tocó. No se repitió el ciclo completo en regtest de la v0.19.2.

### Binario

- Binarios oficiales `mostrod` v0.20.0 descargados de la release de GitHub. SHA-256 coincidentes con `manifest.txt`: amd64 `078c2f1a2f0d9a3e438e5d363480c79a19931664ceb36be0661155f73171bc86`, arm64 `f82fe9e7f09e727851a8fcbf107d4c266abdb1ee47f7092dba116f891ccce1f6`.
- Firmas GPG de `manifest.txt` válidas con las claves `1E41631D137BA2ADE55344F73852B843679AD6F0` (negrunch) y `2E986CA1C5E7EA1635CD059C4989CC7415A43AEC` (arkanoider), descargadas del tag v0.20.0. La release no trae firma de catrya. Las claves no están certificadas por una cadena de confianza propia.
- El binario amd64 responde `mostro p2p 0.20.0`. El de arm64 no se ejecutó.

### Configuración y base de datos

- La plantilla de v0.20.0 frente a la de v0.19.0 (la que compila el Manager): solo cambia en bloques comentados y en la sección nueva `[health]`. En el código de v0.20.0, `Settings::health` lleva `#[serde(default)]` y sus valores por defecto son los de la plantilla, así que omitirla es equivalente. `admin.proto` solo añade `RevokeReputationImports`.
- El `settings.toml` de producción, con una clave desechable, LND y relays inalcanzables y sin red, cargado por el binario v0.20.0: `Settings correctly loaded!`, base de datos creada con todas las migraciones y `Transport: nip44 (protocol v2, event kind 14)`. Se detiene, como se espera, al no poder conectar con LND. La validación nueva de `escrow_deadline_margin_blocks` (24, con `hold_invoice_cltv_delta` 144) pasa.
- Copia de la base de datos de producción (6 órdenes, 15 migraciones) abierta con v0.20.0 sin red: se aplican las 9 migraciones nuevas (24 en total, ninguna fallida) y las 6 órdenes siguen.
- La misma copia, ya migrada, abierta con v0.19.2: el daemon no arranca (`migration 20261002120000 was previously applied but is missing in the resolved migrations`). Volver a la 1.0.13 exige restaurar un respaldo previo a la actualización.

### Protocolo

- mostro-core 0.17.3 (el que usa v0.20.0) define las mismas seis variantes de mensaje que la 0.16.0. La consola de mediación acepta cualquier nombre de acción, así que las acciones nuevas se muestran sin cambios en el Manager.
