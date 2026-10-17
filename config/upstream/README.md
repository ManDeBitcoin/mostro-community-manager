# Archivos de Mostro fijados

Copias literales de [MostroP2P/mostro](https://github.com/MostroP2P/mostro). No se editan a mano: se sustituyen al fijar otra versión.

| Archivo | Origen upstream | Uso en el Manager | Licencia |
| --- | --- | --- | --- |
| `settings.v0.19.0.toml` | `settings.tpl.toml` en `v0.19.0` | Plantilla que el renderer compila dentro de la API | MIT |
| `admin.v0.19.0.proto` | `proto/admin.proto` en `v0.19.0` | Contrato gRPC que compila `api/build.rs` (solo `GetVersion`) | MIT |
| `settings.v0.20.0.toml` | `settings.tpl.toml` en `v0.20.0` | Referencia de la versión fijada; no se compila | GPL-3.0-or-later |
| `admin.v0.20.0.proto` | `proto/admin.proto` en `v0.20.0` | Referencia de la versión fijada; no se compila | GPL-3.0-or-later |
| `settings.v0.18.8.toml`, `admin.v0.18.8.proto` | `v0.18.8` | Histórico, sin uso | MIT |

## Por qué se compila la plantilla de v0.19.0 con el daemon v0.20.0

Mostro pasó de MIT a GPL-3.0-or-later en v0.19.2. Entre v0.19.0 y v0.20.0 la plantilla ganó bloques comentados (`serbero_pubkey`, `[payer_history]`, `[reputation_import]`, cambios en `[cashu]`) y una sola sección activa, `[health]`. mostrod lee una sección `[health]` ausente como sus valores por defecto (`#[serde(default)]` en `Settings::health`, `src/config/settings.rs` de v0.20.0), que son los mismos que lista la plantilla. Así que la plantilla MIT, que no la tiene, produce un `settings.toml` con el mismo efecto. El proto de v0.20.0 solo añade `RevokeReputationImports`; la API solo llama a `GetVersion`.

La prueba `embedded_template_is_equivalent_to_the_pinned_upstream_release` en `api/tests/configuration.rs` compara ambas plantillas ya analizadas, sin `[health]`, comprueba que los valores de `[health]` sean esos valores por defecto y que el proto solo difiera en `RevokeReputationImports`. Falla si upstream cambia cualquier otra cosa.

Desde la v1.0.16 el Manager es GPL-3.0-or-later (archivo `LICENSE` en la raíz del repositorio), igual que mostrod, así que compilar la plantilla GPL de la versión fijada ya no condiciona su licencia. La plantilla MIT de v0.19.0 se mantiene solo hasta que el renderer pase a usar la de la versión fijada.

## El binario `mostrod`

La imagen Docker descarga el binario oficial sin modificar desde la release de GitHub y comprueba su SHA-256 (ver `config/versions.json` y `docker/Dockerfile.umbrel`). Desde v0.19.2 ese binario es GPL-3.0-or-later. La imagen incluye el texto de la licencia y este aviso en `/usr/share/licenses/mostrod/`. El código fuente correspondiente es el tag de la versión fijada:

<https://github.com/MostroP2P/mostro/tree/v0.20.0>

`LICENSE-MIT` conserva el aviso de copyright del código anterior al cambio de licencia. `LICENSE-GPL-3.0-or-later` es el archivo `LICENSE` de v0.19.2, idéntico en v0.20.0.
