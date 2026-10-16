# Decisiones y compatibilidad — primera iteración

> Documento de la primera iteración, conservado como registro de decisiones. La versión de Mostro fijada hoy es la v0.20.0: ver `config/versions.json` y `config/upstream/README.md`. El contrato con las apps cliente está en [INTEGRACION-APPS.md](INTEGRACION-APPS.md).

El blueprint adjunto es referencia de producto. Esta entrega implementa una parte de las fases 0–1; no constituye el MVP completo ni una instalación validada en Umbrel.

## Fronteras

Navegador → Vite (desarrollo) o nginx (contenedor) → API Axum → adaptadores. UI React + TypeScript; backend Rust. Los contratos Mostro se fijan a v0.18.8. El backend contiene un renderer que parte de la plantilla completa upstream, preserva sus parámetros de seguridad y convierte puntos básicos a fracciones (60 bps = 0.006). El catálogo de pagos, idioma y contacto son metadatos del Manager; no inventamos claves TOML para publicarlos.

Las consultas de salud son reales cuando se suministran endpoints. Un `GetVersion` exitoso solo verifica RPC, no que relays, mercado o escrow funcionen. Bitcoin se informa como desconocido hasta implementar su adaptador. El mercado permanece sin iniciar en esta versión.

## Persistencia y acceso

El único cambio expuesto por HTTP guarda un borrador sin secretos en `CONFIG_DIR/community.json`. Tiene validación tipada, revisión optimista, archivo temporal, fsync y reemplazo atómico; conserva una revisión anterior. Un proceso API por directorio. Un archivo corrupto impide iniciar en vez de reiniciar silenciosamente la configuración.

API local en `127.0.0.1:3001` por defecto. En contenedor, sin puerto publicado, detrás de web. Se exige JSON y cabecera personalizada para PUT; no se habilita CORS y se compara Origin/Host. En el paquete Umbrel la autenticación depende de `app_proxy`; la instalación aún debe verificarse en un Umbrel de prueba. La API/UI queda en una red interna propia, sin puertos publicados. El proxy de Umbrel entra desde su red compartida. Un puente TCP de destino fijo conecta exclusivamente con LND; el panel conserva su aislamiento de la red compartida. Solo la API monta `tls.cert` y `readonly.macaroon` de LND en modo lectura. El puente no monta credenciales y transmite TLS sin modificarlo. No hay autenticación autónoma ni soporte para exponer este prototipo a Internet. Para la distribución final se debe cerrar también el acceso entre aplicaciones en la red Docker compartida y probar sesiones/CSRF detrás del proxy.

El renderer es una función interna, no un endpoint. Produce un candidato TOML con `nsec_privkey` vacío y RPC deshabilitado; no duplica el `nsec` importado ni un token RPC. Mostro v0.18.8 puede recibir el `nsec` mediante `MOSTRO_NSEC_PRIVKEY`. La imagen opcional dispone de un lanzador que lo lee de un archivo montado privadamente antes de ejecutar el daemon; rechaza archivos ausentes, enlaces simbólicos y permisos amplios. El montaje de esa imagen en Umbrel y la restauración todavía no están implementados. La identidad y el borrador ya se pueden exportar en un archivo `age` cifrado antes de iniciar el mercado; el backup de la futura DB sigue pendiente. El candidato TOML no se aplica al daemon. La validación de monedas comprueba formato y duplicados; la disponibilidad real de cada moneda según proveedores está pendiente. La importación local de identidad comprueba criptográficamente que `nsec` corresponde al `npub` indicado.

## Fuentes comprobadas

- [Release Mostro v0.18.8](https://github.com/MostroP2P/mostro/releases/tag/v0.18.8).
- [Plantilla exacta](https://github.com/MostroP2P/mostro/blob/v0.18.8/settings.tpl.toml), copiada en `config/upstream`.
- [Proto exacto](https://github.com/MostroP2P/mostro/blob/v0.18.8/proto/admin.proto), copiado en `config/upstream`.
- [Arranque oficial](https://github.com/MostroP2P/mostro/blob/v0.18.8/docker/start.sh): `mostrod -d /config`; nuestro contenedor utiliza `/data`.
- [Checksums de release](https://github.com/MostroP2P/mostro/releases/download/v0.18.8/manifest.txt), fijados para amd64/arm64. Las firmas de los mantenedores no se han verificado en esta entrega.
- [Exports Lightning de Umbrel](https://github.com/getumbrel/umbrel-apps/blob/master/lightning/exports.sh).
- [Integración LND en LNbits](https://github.com/getumbrel/umbrel-apps/blob/master/lnbits/docker-compose.yml).

`GetVersion` y `GetMaintenanceStatus` están presentes. Las mutaciones llevan `authorization: Bearer <token>`; `SetMaintenanceMode` además indica restricción loopback. La topología futura debe respetarla, por ejemplo compartiendo namespace de red API/Mostro. El candidato conserva la dirección RPC en loopback, pero deja RPC apagado; su activación exigirá un token privado y una topología verificada.

Chat y resolución explícita de bonds no aparecen en el proto. Deben estudiarse en el protocolo administrativo Nostr, con compatibilidad de cliente comprobada antes de exponerlos. No hay matriz de compatibilidad probada de Mostro App; se guarda `null` en vez de inventar una versión mínima.

## Requisitos pendientes antes del primer mercado

1. Identidad Nostr generada/guardada solo en servidor y exportación de backup cifrado. Resolver la tensión del blueprint entre “nunca entregar nsec al navegador” y “mostrarlo en onboarding”: preferir exportación cifrada, no texto plano.
2. Detección directa de Bitcoin y evaluación de capacidad de pago/recepción de LND; la consulta de saldos agregados no garantiza rutas utilizables.
3. Validación de configuración contra binario upstream; distinguir borrador, configuración aplicada y cambios pendientes.
4. Control de proceso sin montar Docker socket privilegiado; arranque y recuperación probados en regtest.
5. Probar Mostro App compatible, creación/toma/cancelación/settlement y bonds en regtest.
6. Empaquetar imágenes por digest y probar acceso de Umbrel; completar metadatos reales del repositorio, icono y galería.

No se ha conectado ni migrado ninguna instalación existente.
