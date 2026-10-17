# Publicación de versiones

El código y la imagen viven en este repositorio (`ManDeBitcoin/mostro-community-manager`). La tienda comunitaria de Umbrel vive aparte, en [`ManDeBitcoin/mostro-community-umbrel`](https://github.com/ManDeBitcoin/mostro-community-umbrel): el manifiesto `mandebitcoin-mostro-manager/umbrel-app.yml`, su `docker-compose.yml`, `store-guard.yml` y las herramientas `scripts/release_metadata.py` y `scripts/verify-public-image.py`. Umbrel clona ese repositorio entero en cada equipo, por eso solo tiene lo que la tienda necesita.

El manifiesto de la tienda solo debe anunciar una versión cuya imagen multi-arquitectura ya esté publicada y sea descargable sin credenciales. La `1.0.4` mostró por qué: Umbrel sincronizó el manifiesto antes de que terminara la compilación y respondió `manifest unknown` al intentar actualizar.

## Regla de publicación

1. Preparar los cambios y escribir `docs/release-notes.md` con el encabezado `# Mostro Community Manager vX.Y.Z`. No se toca el repositorio de la tienda.
2. Ejecutar `./scripts/check.sh` y `python3 -m unittest discover -s scripts/tests`. Abrir el PR hacia `main`: `image-check.yml` construye la imagen de amd64 y arm64 y ejecuta las pruebas de humo sin publicar nada. El mismo workflow corre en `main` para que los PR partan de su caché de construcción. Con el PR fusionado, crear y publicar el tag anotado `vX.Y.Z` en el commit probado.
3. `publish.yml` clona la tienda y rechaza el tag si su manifiesto ya anuncia la nueva versión o si las notas no empiezan por su encabezado. Construye y prueba amd64 y arm64, publica la imagen como `ghcr.io/mandebitcoin/mostro-community-manager:X.Y.Z` y, con el mismo contenido, como `ghcr.io/mandebitcoin/mostro-community-umbrel:X.Y.Z` (el nombre que usa la tienda), verifica acceso anónimo a ambos nombres y crea la release en este repositorio.
4. Solo después de esas comprobaciones, el mismo workflow abre en la tienda un PR que actualiza el manifiesto, las cuatro referencias de Compose, las notas de Umbrel y el README, espera sus checks obligatorios (`store-guard`) y lo fusiona. Si la publicación o la promoción fallan, la tienda conserva la versión anterior.
5. En la tienda, `store-guard.yml` comprueba en cada push que la versión anunciada tenga una imagen pública para ambas arquitecturas.

Requisitos de configuración:

- Secreto `PROMOTION_TOKEN` en este repositorio: un token con permiso de escritura de contenido y de pull requests sobre `ManDeBitcoin/mostro-community-umbrel`.
- En GHCR, el paquete `mostro-community-umbrel` debe dar acceso de escritura a este repositorio (*Package settings → Manage Actions access*), y los dos paquetes deben ser públicos.

No crear ni subir manualmente un commit que cambie la versión del manifiesto antes de que GHCR tenga la imagen. No mover un tag existente para apuntarlo a otro commit; ante un fallo transitorio se puede reintentar el workflow del mismo tag. Para comprobar una versión desde fuera de GitHub, ejecutar en la tienda `python3 scripts/verify-public-image.py --repository mandebitcoin/mostro-community-manager X.Y.Z`.

Umbrel sincroniza la tienda y ofrece la actualización, pero no la aplica automáticamente a las instalaciones. El operador debe elegir **Actualizar** en la interfaz de Umbrel cuando la publicación haya terminado.
