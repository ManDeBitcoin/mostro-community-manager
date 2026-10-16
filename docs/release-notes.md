# Mostro Community Manager v1.0.14

- Incluye el daemon oficial Mostro v0.20.0 (antes v0.19.2), comprobado por SHA-256 para amd64 y arm64. Las firmas del manifiesto de la release, de grunch y arkanoider, se verificaron.
- **Haz un respaldo antes de actualizar.** Al primer arranque el daemon aplica nueve migraciones a su base de datos. El daemon v0.19.2 de la 1.0.13 no arranca sobre una base ya migrada: volver a la 1.0.13 exige restaurar el respaldo hecho antes de actualizar.
- Actualiza sin órdenes abiertas.
- Nuevo en el daemon: publica cada minuto un evento de salud (kind 38389) que dice si el nodo funciona (LND sincronizado, pagos a compradores, relays, base de datos). El Manager no configura un vigilante (`watchdog_pubkey`), así que solo salen las etiquetas públicas generales, sin métricas detalladas.
- Nuevo en el daemon: si LND deja de estar sincronizado durante diez minutos seguidos, el daemon entra solo en modo mantenimiento con el motivo `lnd_unsynced`: no acepta órdenes nuevas ni tomas, y las operaciones abiertas pueden terminar. Sale solo tras cinco minutos con LND sincronizado. Un modo mantenimiento puesto a mano nunca lo levanta el daemon.
- Nuevo en el daemon: los pagos salientes (al comprador, el aporte al desarrollo y las devoluciones de garantía) esperan mientras LND no está sincronizado, sin gastar los intentos del comprador. Un pago del aporte al desarrollo que falla se reintenta cada vez más espaciado, por orden.
- El daemon comprueba al arrancar que `escrow_deadline_margin_blocks` sea al menos 18 y menor que `hold_invoice_cltv_delta`. Los valores que genera el Manager (24 y 144) cumplen.
- La importación de reputación de v0.20.0 queda desactivada: el Manager no la configura.
- El panel todavía no muestra el evento de salud.
- La configuración que genera el Manager no cambia.

# Mostro Community Manager v1.0.13

- Nuevo en **Conexión de apps**: el panel puede publicar la tarjeta firmada de la comunidad en los relays del nodo. Una app preparada para ello encuentra sola los métodos de pago, la moneda, la web y el contacto, sin código ni enlace.
- La opción nace apagada y pide confirmación al encenderla: deja esos datos a la vista de cualquiera que consulte los relays, ligados a la clave del nodo. El panel lo explica antes de publicar nada.
- Actualizar no publica nada. Mientras no enciendas el interruptor, el panel sigue sin enviar ningún evento a los relays.
- La tarjeta se envía al encender el interruptor, al guardar un cambio que la afecta, al arrancar y cada 6 horas. Mientras no cambia se reenvía el mismo evento, con la misma fecha, para que las apps la lean como «última modificación».
- El panel muestra la respuesta de cada relay y solo da la tarjeta por publicada donde el relay confirma que la aceptó. Si un relay la rechaza o no responde, lo reintenta solo, lo indica en el resumen y registra una alerta.
- Si el nodo no tiene identidad, no hay reglas guardadas o la configuración contiene caracteres que la tarjeta no admite, no se publica nada y el panel dice el motivo.
- Al apagar el interruptor, el panel deja de enviar la tarjeta y pide a los relays que la borren. Es una petición: un relay puede no atenderla y una app que ya la leyó la conserva.
- La tarjeta publicada es un evento kind 30078 con `d = mostro-community-card`, distinto del evento de cotizaciones del daemon. No indica si el nodo está en marcha, y las apps no deben tomar de ella la comisión ni la garantía. El formato, las comprobaciones que debe hacer una app y un vector de prueba están en `docs/INTEGRACION-APPS.md`, sección 3.3.
- El interruptor se guarda con la configuración y entra en los respaldos. No cambia la revisión de las reglas ni reinicia el daemon.
- Después de actualizar o de reiniciar, el panel ya no da el mercado por abierto ni avisa de que «el daemon anuncia otra versión» por el anuncio que dejó en los relays el daemon anterior. Muestra «Arrancando» hasta que el daemon nuevo se anuncia. El campo `market_started` de `/api/dashboard` sigue la misma regla.
- El primer guardado después de actualizar ya no reinicia el daemon cuando la descripción o la web de la comunidad están vacías. El panel compara lo que el daemon leería de su configuración, no el texto del archivo.
- Regla de la tarjeta firmada de la comunidad: el nombre, la web y el contacto no pueden contener «&», y los relays y los métodos de pago activos no pueden contener «&» ni «,». El panel rechaza el guardado y dice qué campo hay que cambiar. Si tus reglas, guardadas con una versión anterior, ya los contienen, la tarjeta no se genera hasta que los cambies y guardes. La página «Conexión de apps» y `GET /api/community/card` dan ahora ese motivo; antes respondían que faltaba la identidad o la configuración.
- Los métodos de pago y los importes de las órdenes que llegan de los relays se muestran acotados: como mucho diez métodos de sesenta caracteres, en una línea y sin caracteres invisibles, y el importe solo cuando es un número entero, o los dos de un rango. Una orden sin importe legible lo dice; antes mostraba 0.
- Si el daemon termina de forma inesperada, el resumen lo avisa durante una hora aunque el supervisor ya lo haya vuelto a arrancar, y dice si lo terminó una señal del sistema. Antes solo avisaba cuando caía en su primer minuto.
- Con el mercado abierto, el estado deja de mostrarse en verde cuando Lightning no tiene canales activos, cuando LND no está sincronizado o cuando el panel no ha podido leerlo. Pasa a «Abierto con avisos» y dice cuál de los tres es.
- Las alertas se vuelven a leer cada 20 segundos y su conexión en vivo se reabre sola. Antes, tras un reinicio de la aplicación, una pestaña abierta dejaba de recibirlas hasta recargarla.
- El panel ya no dice que no hay reglas guardadas mientras todavía las está leyendo o si no ha podido leerlas. Tampoco cuenta órdenes como cero, ni dice «Todo en orden» o «Ninguna abierta», mientras lee los relays o cuando ninguno responde.
- Fuera de Umbrel, cuando el panel y el daemon comparten espacio de procesos, una parada pedida desde el panel ya no queda registrada como una caída del daemon.
- Sigue incluyendo el daemon Mostro v0.19.2. La actualización no requiere migración de base de datos ni cambios de configuración.

# Mostro Community Manager v1.0.12

- El panel se reorganiza en cinco grupos: Inicio, Mercado, Nodo, Ajustes y Herramientas. Cada página tiene su propia dirección, así que recargar no pierde el sitio y el botón de retroceso funciona.
- Nueva página de resumen: dice si el mercado está abierto, lista lo que requiere atención con un enlace a la página donde se resuelve y, en una instalación nueva, guía la puesta en marcha paso a paso.
- Nuevas páginas para el nodo, la conexión de apps, los respaldos y las alertas, que antes compartían una sola pantalla. La identidad del nodo se crea o se importa desde la página del nodo.
- El panel solo dice que el mercado está abierto cuando Mostro está activado en este equipo con su daemon en ejecución y el nodo se ha anunciado en los relays en los últimos minutos. Tras desactivarlo avisa de que el anuncio sigue vigente unos minutos.
- Un dato que el panel no ha podido leer se muestra como tal, no como vacío ni como cero. La página de Lightning no enseña saldos si no puede leer LND, y estima cuántas operaciones del importe máximo caben a la vez con la liquidez entrante.
- Las plantillas de comisiones dicen en su tarjeta todo lo que cambian, también el aporte al desarrollo, la comisión máxima de enrutamiento y la penalización automática.
- La página de respaldos avisa de que cada respaldo nuevo borra los que sobran, y sus instrucciones de restauración usan un destino que el paquete de Umbrel acepta.
- Un solo término para cada cosa en el panel y en sus avisos: «garantía» para el bono antiabuso y «depósito» para los sats que retiene el vendedor. Las fechas y los importes se escriben igual en todas las páginas.
- El panel se puede usar desde un teléfono y con teclado: el menú se abre como un cajón, ninguna página se desborda a lo ancho y los diálogos retienen el foco.
- Incluye el daemon oficial Mostro v0.19.2, comprobado por SHA-256 para amd64 y arm64. La actualización no requiere migración de base de datos ni cambios de configuración.
- Mostro v0.19.2 envía el aporte al desarrollo a la nueva dirección del proyecto, porque la anterior fue suspendida, y solo cuando LND opera en mainnet.
- El monitor de órdenes reconoce el estado «En curso» que el daemon publica al tomarse una orden. Antes descartaba esa revisión y la orden seguía figurando como oferta abierta.
- Las disputas se leen de los eventos propios del nodo (kind 38386) y generan una alerta al abrirse.
- La consola de mediación muestra las disputas y los mensajes de protocolo de cada orden, y explica cómo resolverlas con un cliente de mediación. Se retiraron los botones de resolución, que no ejecutaban ninguna acción.
- La página del nodo muestra la versión que el nodo anuncia en los relays y hace cuánto. Si el daemon se reinicia en bucle, el panel lo indica.
- Guardar un cambio que no altera la configuración del daemon ya no lo reinicia.
- La tarjeta de la comunidad se firma con el formato que verifican las apps y se ofrece como enlace `mostro://community/…` con todos los datos.
- El panel ya no publica por su cuenta el evento de información del nodo: lo publica solo el daemon.
- El simulador sigue la secuencia y los importes reales de una operación en Mostro v0.19.2.
- El panel avisa cuando la prueba de trabajo de la primera conversación supera a la general: las apps que no la calculan dejan de recibir respuesta del daemon.
- Nueva guía técnica para desarrolladores de apps: `docs/INTEGRACION-APPS.md`.

# Mostro Community Manager v1.0.11

- Actualización integral al Protocolo Mostro v0.19.0 (Protocolo de Transporte v2 sobre Nostr Kind 14 con cifrado NIP-44 v2).
- Detección e introspección activa de la versión del daemon Mostro en ejecución con reporte en la API (`/api/daemon/status`) y visualización en tiempo real en la tarjeta de servicio y cuadrícula del panel de administración.
- Adaptación del canal de mensajería y mediación para decodificar eventos Kind 14 y procesar estructuras de órdenes en tupla de 3 elementos (`[order, signature, proof]`).
- Actualización de plantillas de configuración upstream v0.19.0 incorporando el nuevo parámetro `maker_bond_payment_timeout_seconds` y retirando directivas de transporte obsoletas.
- Verificación criptográfica SHA-256 de los binarios oficiales upstream de Mostro v0.19.0 para arquitecturas amd64 y arm64.

# Mostro Community Manager v1.0.10

- Actualización del estándar de JSON comunitario para conexiones con Mostro App (esquema estándar v1 con version, name, pubkey, relays, currency, payment_methods, fee_bps, bond_percent, website, contact, signature).
- Firma criptográfica Schnorr (BIP-340) generada automáticamente sobre el payload canónico con la identidad Nostr del nodo.
- Nuevo endpoint público `GET /api/community/card` para consultar la tarjeta de la comunidad directamente.
- Integración en la interfaz de conexión para copiar el JSON estándar y generar el código QR interactivo correspondiente.

# Mostro Community Manager v1.0.9

- Rediseño estético y armonioso de la tarjeta de Conexión con Mostro App y Mostrix.
- Distribución en 2 columnas: panel de código QR interactivo con selector de pestañas (URI Mostro / JSON) a la izquierda y detalles de conexión con botones de copiado a la derecha.
- Visualización dinámica del payload activo (URI o JSON) según la pestaña seleccionada.

# Mostro Community Manager v1.0.8

- Agregado switch para alternar entre formato URI y JSON en el código QR comunitario.
- Corregido el copiado al portapapeles en entornos HTTP.

# Mostro Community Manager v1.0.7

Actualización del formato de conexión de la URI QR (nostr: -> mostro://community/)

# Mostro Community Manager v1.0.6

Onboarding soberano con generación de identidad Nostr, plantillas de comisiones y catálogo bilingüe de divisas ISO 4217.

- Generación soberana de clave privada Nostr (nsec/npub) en 1 clic e importación directa desde el panel web con permisos restringidos (0600) y modal de custodia.
- Prellenado rápido (presets) exclusivo para comisiones de mercado, desarrollo y fianza anti-abuso (sin sobreescribir identidad, relays ni métodos de pago).
- Selector interactivo de divisas fiat con el catálogo oficial ISO 4217 (174 monedas) y búsqueda bilingüe (español e inglés) insensible a mayúsculas y acentos.
- Píldoras de acceso rápido para monedas P2P frecuentes (USD, EUR, ARS, VES, COP, BRL, MXN, CLP, PEN, GBP, CHF) y validación de hasta 30 divisas por el protocolo.
