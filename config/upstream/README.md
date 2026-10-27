# Archivos de Mostro fijados

Copias literales de [MostroP2P/mostro](https://github.com/MostroP2P/mostro). No se editan a mano: se sustituyen al fijar otra versión.

| Archivo | Origen upstream | Uso en el Manager | Licencia |
| --- | --- | --- | --- |
| `settings.tpl.toml` | `settings.tpl.toml` de la versión fijada | Plantilla de la que parte el renderer (`api/src/config.rs`) | GPL-3.0-or-later |
| `admin.proto` | `proto/admin.proto` de la versión fijada | Contrato gRPC que compila `api/build.rs` (solo `GetVersion`) | GPL-3.0-or-later |

Los dos llevan nombre fijo: al fijar otra versión, `scripts/bump-mostrod.py` los reemplaza junto con el resto de pines, y el diff del PR muestra exactamente qué cambió upstream. Las copias de versiones anteriores están en el historial de git.

## Cómo se usa la plantilla

El renderer parte de la plantilla de la versión fijada y sobrescribe solo lo que gestiona el panel (LND, relays, perfil, comisiones, límites, monedas, prueba de trabajo, garantía, RPC desactivado). Todo lo demás conserva el valor de upstream, así que una sección nueva (como `[health]` en v0.20.0) llega con sus valores por defecto sin tocar código.

Las pruebas de `api/tests/configuration.rs` fijan el contrato con upstream:

- cada clave que escribe el Manager existe en la plantilla, activa o como ejemplo comentado (`# clave = valor`), con el mismo tipo; mostrod ignora claves desconocidas, así que un cambio de nombre upstream pasaría desapercibido sin esta prueba;
- ninguna clave activa de la plantilla se pierde al generar la configuración;
- los valores fijos de `[anti_abuse_bond]` que el panel no muestra son los que documenta upstream;
- la configuración generada pasa las validaciones de arranque conocidas (margen de escrow, aporte al desarrollo).

Al arrancar, la API vuelve a aplicar las reglas guardadas (`daemon::sync_with_recorded_origin`): si la plantilla de una versión nueva cambia el resultado, el daemon se reinicia una sola vez durante la actualización y no en un guardado posterior.

El Manager es GPL-3.0-or-later (archivo `LICENSE` en la raíz del repositorio), igual que mostrod, así que compila la plantilla GPL de la versión fijada sin condicionar su licencia.

## Claves de los firmantes

`keys/` guarda las claves públicas de los mantenedores que firman los manifiestos de las releases (copiadas de `keys/` de MostroP2P/mostro en v0.20.0), y `keys/FINGERPRINTS` sus huellas principales. `scripts/verify-mostrod-release.sh` solo confía en esas huellas. Si upstream agrega o retira un firmante, se cambia aquí a mano, comprobando la huella con las notas de la release.

## El binario `mostrod`

La imagen Docker descarga el binario oficial sin modificar desde la release de GitHub y comprueba su SHA-256 (ver `config/versions.json` y `docker/Dockerfile`). Desde v0.19.2 ese binario es GPL-3.0-or-later. La imagen incluye el texto de la licencia y este aviso en `/usr/share/licenses/mostrod/`. El código fuente correspondiente es el tag de la versión fijada:

<https://github.com/MostroP2P/mostro/tree/v0.20.1>

`LICENSE-MIT` conserva el aviso de copyright del código anterior al cambio de licencia. `LICENSE-GPL-3.0-or-later` es el archivo `LICENSE` de v0.19.2, idéntico en v0.20.0.
