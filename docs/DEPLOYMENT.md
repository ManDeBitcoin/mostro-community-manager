# Guía de Despliegue y Manual de Operaciones: Mostro Community Manager for Umbrel

> **Versión:** 1.0.0 (Producción)  
> **Compatibilidad:** Umbrel OS v0.5+ / UmbrelOS 1.x  
> **Protocolo:** Mostro v0.20.0, protocolo 2 (mensajes kind 14 con cifrado NIP-44) / Lightning Network (LND)

---

## 1. Visión General del Sistema

**Mostro Community Manager** permite a cualquier operador con un nodo Umbrel desplegar y administrar una comunidad P2P de intercambio soberano de Bitcoin sin necesidad de programar, bifurcar software ni mantener clientes móviles propios.

- **Compradores y Vendedores:** Utilizan la **Mostro App oficial** (disponible para Android, iOS y Web) conectándose directamente a tu nodo mediante la identidad pública (`npub` / `nprofile`) y los relays Nostr que configures.
- **Motor de Mercado:** Ejecuta el daemon oficial de **Mostro** (v0.20.0) orquestado de forma aislada.
- **Canal Financiero:** Se conecta al nodo **LND** de tu Umbrel mediante permisos mínimos de solo lectura (`readonly.macaroon`) para la interfaz administrativa, y credenciales seguras para el daemon de pagos.
- **Panel de Control:** Servido mediante una API en Rust de alto rendimiento y una interfaz web en React con soporte para monitor de órdenes, consola de mediación cifrada, gestión de liquidez Lightning, alertas en vivo y respaldos cifrados.

```text
USUARIOS (Mostro App)
       │ (Nostr kind 14, NIP-44)
       ▼
   Relays Nostr
       │
       ▼
┌────────────────────────────────────────────────────────┐
│ UMBREL DEL OPERADOR                                    │
│  ├── Mostro Community Manager (Web + API Axum)         │
│  │    ├── Monitor de órdenes (Kind 38383)              │
│  │    ├── Tarjeta de la comunidad en los relays (opc.) │
│  │    ├── Consola de mediación de disputas             │
│  │    ├── Operaciones y gestión de liquidez LND        │
│  │    ├── Transmisión de alertas en vivo (SSE)         │
│  │    └── Respaldos cifrados (age)                     │
│  │                                                     │
│  ├── Daemon Mostro (v0.20.0) [Orquestación Atómica]    │
│  │                                                     │
│  ├── LND (Lightning Node)                              │
│  └── Bitcoin Core Node                                 │
└────────────────────────────────────────────────────────┘
```

---

## 2. Requisitos Previos

Antes de instalar y poner en marcha tu comunidad, asegúrate de que tu nodo Umbrel cumpla con las siguientes condiciones:

1. **Hardware y Sistema Operativo:**
   - Servidor Umbrel operativo (Raspberry Pi 4/5, mini PC x86_64, etc.) con al menos 4 GB de memoria RAM.
   - Disco SSD con espacio disponible (mínimo 10 GB libres para datos de la app y backups).
2. **Aplicaciones Base Instaladas en Umbrel:**
   - **Bitcoin Node (Bitcoin Core):** Completamente sincronizado al 100% de la cadena de bloques.
   - **Lightning Node (LND):** Sincronizado a la cadena y con grafo Lightning actualizado.
3. **Conectividad:**
   - Conexión a internet estable 24/7.
   - Puertos salientes habilitados para conectarse a relays Nostr (`wss://`) y peers Lightning.

---

## 3. Instalación en Umbrel

### Opción A: A través de la Umbrel App Store (Recomendada)
1. Abre el panel de tu Umbrel (`http://umbrel.local`).
2. Dirígete a la **App Store** de Umbrel.
3. Si el repositorio comunitario de Mostro ya está indexado, busca **"Mostro Community Manager"** y haz clic en **Instalar**.
4. Si estás usando una tienda comunitaria personalizada:
   - Ve a los ajustes de la App Store de Umbrel.
   - Añade el repositorio: `https://github.com/ManDeBitcoin/mostro-community-umbrel`.
   - Selecciona **Mostro Community Manager** y confirma la instalación.

### Opción B: Despliegue Manual para Pruebas / Desarrollo
En una terminal SSH en tu servidor Umbrel:
```bash
cd ~/umbrel/app-stores/
git clone https://github.com/ManDeBitcoin/mostro-community-umbrel.git getumbrel__mostro-community
# La aplicación aparecerá inmediatamente disponible en la tienda local de tu Umbrel
```

---

## 4. Guía de Configuración Inicial (Paso a Paso)

Una vez completada la instalación, abre la aplicación **Mostro Community Manager** desde el escritorio de tu Umbrel.

El panel se organiza en cinco grupos, en el menú de la izquierda:

| Grupo | Páginas | Para qué |
| --- | --- | --- |
| **Inicio** | Resumen | El estado del mercado y lo que requiere tu atención |
| **Mercado** | Órdenes, Disputas | Lo que tu nodo anuncia: ofertas, operaciones y disputas |
| **Nodo** | Nodo Mostro, Lightning, Conexión de apps | Encender y apagar el daemon, su identidad, la liquidez y los datos que una app necesita |
| **Ajustes** | Configuración, Respaldos, Alertas | Las reglas de la comunidad, las copias cifradas y el registro de avisos |
| **Herramientas** | Simulador | Ensayar una operación sin mover fondos |

En una instalación nueva, la página **Resumen** muestra la lista **Puesta en marcha**, con seis pasos en el mismo orden que esta guía. El botón principal lleva siempre al siguiente paso pendiente. La guía añade dos que no están en la lista: ensayar una operación y conectar las apps.

### Paso 1: Crear la Identidad del Nodo
El nodo Mostro firma sus órdenes y mensajes con un par de claves Nostr: la clave privada (`nsec`) y la pública (`npub`). La pública es lo que las apps reconocen como tu comunidad.

1. Abre **Nodo Mostro**.
2. Elige una de las dos opciones del panel **Identidad del nodo**:
   - **Crear una identidad nueva.** El panel muestra la clave privada **una sola vez**. Cópiala en un gestor de contraseñas antes de continuar: no vuelve a mostrarse.
   - **Tengo una clave nsec.** Pega una clave que ya uses para tu comunidad.
3. La clave se guarda en `/data/config/identity/mostro.nsec` con permisos `0600`, dentro de un directorio `0700`.

La importación también puede hacerse por terminal, sin pasar por el navegador. Está descrita en [`identity.md`](identity.md).

### Paso 2: Definir las Reglas de la Comunidad
Abre **Configuración**. La página tiene cinco secciones y un índice para saltar de una a otra:

1. **Identidad:** nombre de la comunidad, idioma, sitio web y contacto.
2. **Mercado:**
   - Las monedas fiat en las que opera tu comunidad.
   - El importe mínimo y máximo por operación, en sats.
   - La comisión total por operación. Mostro cobra la mitad a cada parte.
   - El aporte al desarrollo de Mostro y la comisión máxima de enrutamiento. Con 0 % de enrutamiento solo salen pagos por rutas gratuitas.
3. **Seguridad:**
   - La garantía: los sats que cada parte retiene mientras dura la operación. Define el porcentaje, el mínimo y quién la paga.
   - La prueba de trabajo. Si la de la primera conversación supera a la general, las apps que no la calculan no reciben respuesta: el panel lo avisa.
4. **Nostr:** entre 2 y 4 relays rápidos, por ejemplo `wss://relay.mostro.network`, `wss://relay.damus.io` y `wss://nos.lol`. Una app solo ve tu nodo si comparte al menos un relay con él.
5. **Métodos de pago:** los que acepta tu comunidad.

Las **plantillas de comisiones y garantía**, al principio de la página, fijan de una vez varias reglas. Cada tarjeta dice todo lo que cambia: la comisión, el aporte al desarrollo, la comisión máxima de enrutamiento, la garantía con su mínimo y quién la paga, y la penalización automática por vencimiento. Nada se guarda hasta pulsar **Guardar configuración**.

Con Mostro aún sin activar, guardar deja las reglas listas. Con Mostro activado, guardar aplica las reglas al nodo y el daemon se reinicia solo si cambia lo que lee.

### Paso 3: Guardar un Respaldo Cifrado
Abre **Respaldos**, escribe una frase de cifrado de al menos 16 caracteres y pulsa **Crear respaldo**. El archivo contiene la identidad del nodo y las reglas guardadas, cifradas con `age`. Sin la frase no se puede abrir: guárdala aparte, y copia el archivo fuera del equipo.

Hazlo antes de abrir el mercado. Sin una copia de la identidad, perder el equipo es perder la comunidad.

### Paso 4: Preparar los Canales Lightning
Abre **Lightning**. La página muestra el estado de tu nodo LND, la liquidez y cada canal.

> **Las dos direcciones de la liquidez.** En cada operación el vendedor paga un depósito a tu nodo: esos sats **entran**, y para recibirlos hace falta **liquidez entrante**. Cuando la operación termina, tu nodo paga al comprador: esos sats **salen**, y para enviarlos hace falta **liquidez saliente** y una ruta hasta su monedero. Con solo saldo de tu lado, los depósitos de los vendedores no llegan.

1. **Mira lo que hay.** La página indica la capacidad total, la liquidez saliente y entrante, y cuántas operaciones del importe máximo caben a la vez según la liquidez entrante de los canales activos. Es una estimación: no tiene en cuenta cómo se reparte el saldo entre canales, ni las garantías, las comisiones o las reservas de cada canal.
2. **Consigue liquidez entrante.** Es la que suele faltar en un nodo nuevo. Puedes comprar un canal entrante a un proveedor de liquidez o en un mercado como Amboss Magma, o pedir a otro nodo que abra un canal hacia el tuyo. El canal tiene que abrirse en el nodo LND de tu Umbrel, que es el que usa Mostro: una app con su propio nodo Lightning no sirve para esto. Como referencia, ten varias veces el importe de tu operación máxima: cada operación en curso ocupa su importe hasta que se cierra, y las garantías ocupan el suyo.
3. **Abre canales salientes** hacia dos o tres nodos grandes y estables, para que el pago al comprador encuentre ruta.
4. Un canal nuevo tarda unas confirmaciones en quedar activo. Espera a que la página muestre canales activos y la cadena sincronizada.

Si el panel no puede leer LND, lo dice y no muestra saldos: un cero en pantalla siempre es un cero leído del nodo. El botón **Ejemplo** enseña la página con datos de demostración.

### Paso 5: Ensayar una Operación (Opcional)
Abre **Simulador**, elige un caso y un importe, y pulsa **Simular**:

- *Operación completada*: una venta de principio a fin, con sus garantías, su depósito y sus comisiones.
- *Disputa resuelta a favor del comprador* y *Disputa resuelta con devolución al vendedor*.
- *Cancelación por el vendedor antes de la toma*.

El simulador usa las reglas guardadas y la secuencia de mensajes de Mostro v0.19.2. No mueve fondos ni publica en los relays.

### Paso 6: Activar Mostro
1. Abre **Nodo Mostro**. El panel **Avisos del nodo** lista lo que falte por resolver.
2. Pulsa **Activar Mostro** y confirma.
3. El sistema escribe `active/settings.toml` (`0600`) con las reglas guardadas y el daemon arranca.
4. El daemon publica su información en los relays al arrancar y cada cinco minutos. Cuando el panel la recibe, **Resumen** pasa a decir **Tu mercado está abierto**. Ese anuncio es lo mismo que comprueban las apps.

El panel solo dice que el mercado está abierto cuando se cumplen las dos cosas: Mostro está activado aquí con su daemon en ejecución, y el nodo se ha anunciado en los relays en los últimos once minutos. En los demás casos lo dice así:

| Lo que ve el panel | Lo que muestra |
| --- | --- |
| El daemon acaba de arrancar y aún no se ha anunciado | **Arrancando** |
| El daemon lleva más de once minutos en marcha sin anunciarse, o la configuración está activa y el daemon no se ve en ejecución | **Sin confirmar** |
| El nodo se anuncia, pero Mostro no está activado en este equipo | **Anuncio vigente** |
| El nodo anuncia el modo mantenimiento | **En mantenimiento** |

**Anuncio vigente** es lo normal durante unos minutos después de desactivar Mostro: las apps pueden seguir viendo el nodo como activo hasta que caduque su último anuncio, aunque nadie les responde. Si el anuncio se sigue renovando, otra instancia de Mostro usa la misma clave: no actives una segunda.

**Desactivar Mostro**, en la misma página, detiene el daemon. El nodo deja de atender a las apps, también en las operaciones en curso, hasta que lo actives de nuevo. La confirmación avisa de las órdenes que figuran «En curso» y de las disputas abiertas según los relays, y de lo que el panel no puede ver: una orden «Publicada» puede estar ya tomada, y el panel puede no estar al día con los relays. Revisa los mensajes de las órdenes recientes antes de desactivar.

### Paso 7: Conectar las Apps
Abre **Conexión de apps**. Ahí están el código QR, el enlace de la tarjeta firmada de la comunidad, la clave pública del nodo y sus relays. Es todo lo que una app necesita. La tarjeta lleva la comisión y el porcentaje de garantía como referencia; los valores vigentes y los límites los lee la app del propio nodo. Si cambias la comisión o la garantía, vuelve a compartir la tarjeta.

#### Publicar la tarjeta en los relays (opcional)

En la misma página, **Publicar la tarjeta en los relays** tiene un interruptor que nace apagado. Apagado, la tarjeta solo sale del panel cuando tú compartes el código o el enlace.

Al encenderlo, el panel publica la tarjeta firmada en los relays de tu nodo. Las apps preparadas para ello la encuentran solas y muestran tus métodos de pago y tu contacto sin que nadie escanee nada.

- **Qué queda público.** El nombre de la comunidad, la moneda principal, los métodos de pago activos, la web y el contacto, con la comisión y el porcentaje de garantía como referencia. Cualquiera que consulte esos relays puede leerlo, copiarlo y saber que es de tu nodo, porque va firmado con su clave. No actives la opción si no quieres eso. La descripción de la comunidad, los límites y los métodos desactivados no forman parte de la tarjeta.
- **Qué no hace.** No anuncia que el nodo esté en marcha ni cambia sus reglas. Eso lo publica el propio daemon, y es lo que las apps aplican.
- **Cuándo se envía.** Al encender el interruptor, cada vez que guardas un cambio que afecta a la tarjeta, al arrancar la aplicación y cada 6 horas. Mientras la tarjeta no cambia se reenvía el mismo evento, con la misma fecha: las apps la leen como «última modificación».
- **Qué te muestra el panel.** La respuesta de cada relay. La tarjeta solo figura como publicada en un relay cuando ese relay confirma que la aceptó. Si alguno la rechaza o no responde, lo ves ahí con el motivo que dio el relay, el panel lo reintenta solo y deja un aviso en **Resumen** y una alerta. Si ninguno la acepta, el panel dice que no está publicada.
- **Cuándo no se publica nada.** Si el nodo no tiene identidad, si aún no hay reglas guardadas o si la configuración contiene caracteres que la tarjeta no admite. El panel dice cuál es el motivo y dónde se corrige.
- **Si lo apagas.** El panel deja de enviarla y pide a los relays que la borren (una petición de borrado de Nostr, firmada por el nodo, que solo nombra la tarjeta). Lo repite durante una semana a los relays que no la acepten. Es una petición: un relay puede no atenderla, y una app que ya leyó la tarjeta la conserva. Lo que se publicó no se puede dar por borrado.

El interruptor se guarda con la configuración y entra en los respaldos. No cambia el número de revisión de las reglas ni reinicia el daemon. Otras cosas que conviene saber:

- Los relays que exigen autenticación para escribir rechazan la tarjeta: el panel no se autentica ante ellos.
- Si quitas un relay de la configuración, la copia que tenía se queda allí. El panel te lo indica en esa misma página y, cuando apagues la publicación, le pide también a ese relay que la borre.
- Si copias a otra instalación un `community.json` guardado con el interruptor encendido, esa instalación publica la tarjeta al arrancar.
- Con el interruptor encendido, una versión del Manager anterior a la 1.0.13 no abre la configuración. Apágalo antes si alguna vez tienes que volver a una versión anterior.
- La tarjeta la firma la clave del nodo. Si sustituyes la identidad del nodo con la tarjeta publicada, la de la clave anterior queda en los relays y este panel ya no puede pedir que se borre: apaga la publicación antes de cambiar de identidad. El panel no ofrece ese cambio; solo es posible desde su API.

Quien desarrolla una app tiene la referencia completa en [`INTEGRACION-APPS.md`](INTEGRACION-APPS.md).

---

## 5. Operaciones y Mantenimiento Diario

### Resumen
La página de inicio responde a tres preguntas:

- **¿Está abierto el mercado?** El estado y las reglas que el nodo anuncia a las apps.
- **¿Qué requiere mi atención?** Disputas abiertas, avisos del nodo, relays que no responden, respaldos que fallan y cambios sin guardar. Cada aviso lleva a la página donde se resuelve.
- **¿Qué actividad hay?** Ofertas publicadas, operaciones en curso, completadas, canceladas y disputas abiertas. Cada cifra de órdenes abre el libro con ese filtro, y la de disputas abre la página **Disputas**.

Debajo, **Servicios** muestra de qué depende el mercado: el daemon Mostro, Lightning y los relays.

### Órdenes
- La página **Órdenes** muestra las ofertas y operaciones que tu nodo anuncia en Nostr (kind 38383). Es de solo lectura: el panel no toma órdenes ni mueve fondos.
- Filtra por estado (publicadas, en curso, completadas, canceladas) y por tipo (compra o venta).
- El estado público es menos detallado que el real. Una orden tomada suele figurar como «En curso» hasta que se cierra, pero una venta tomada con la factura ya adjunta sigue como «Publicada». El daemon no publica las disputas en el evento de la orden: las anuncia aparte, y el panel las muestra en un aviso y en la página **Disputas**.
- El botón **Mensajes** abre los mensajes de protocolo de una orden.

### Disputas
- Cuando dos usuarios no se ponen de acuerdo sobre el pago fiat, una de las partes abre una disputa y el nodo la anuncia en un evento propio (kind 38386).
- La página **Disputas** lista esas disputas y, para cada orden, los mensajes de protocolo entre los usuarios y el daemon (kind 14, NIP-44), descifrados en memoria con la identidad del nodo.
- El panel **no** puede leer el chat entre comprador y vendedor ni el del mediador con las partes, y **no** resuelve disputas: es de solo lectura.
- La disputa la resuelve un mediador registrado en el nodo desde un cliente de mediación (Mostrix o `mostro-cli`). La clave del propio nodo es administradora por defecto:
  - `mostro-cli admtakedispute -d <id-de-la-disputa>` para tomarla.
  - `mostro-cli admsettle -o <id-de-la-orden>` si el comprador demostró el pago: el nodo le paga.
  - `mostro-cli admcancel -o <id-de-la-orden>` si el pago no existió: los sats vuelven al vendedor.
  - `mostro-cli admaddsolver -n <npub>` para registrar una clave de mediador distinta de la del nodo.

### Alertas
- La página **Alertas** guarda las 50 más recientes mientras la aplicación está en marcha. Llegan al momento por Server-Sent Events (`/api/notifications/sse`), sin recargar.
- El panel registra una alerta cuando:
  - Un relay se desconecta o no responde.
  - El nodo anuncia una disputa nueva.
  - Un respaldo automático termina, bien o con error, o uno manual termina bien. Un respaldo manual que falla muestra el error en su propia página.
  - Se crea o se importa la identidad del nodo.
  - Unas reglas guardadas no llegan a aplicarse al nodo.
  - Con la publicación de la tarjeta activada, algún relay no la acepta en dos intentos seguidos.
- Cada alerta enlaza con la página donde se atiende. El menú marca las graves que aún no has visto.

### Respaldos
1. **A mano.** En **Respaldos**, escribe la frase de cifrado y pulsa **Crear respaldo**. El archivo se descifra de inmediato con la misma frase para comprobarlo antes de darlo por bueno.
2. **Automático.** Se activa cuando el servicio arranca con la frase en la variable de entorno `BACKUP_PASSPHRASE`, y repite la copia cada 24 horas. El paquete de Umbrel no define esa variable, así que en Umbrel los respaldos son manuales.
3. **Cuántos se conservan.** Los 7 más recientes. Cada respaldo nuevo, manual o automático, borra de esa carpeta los que sobran: copia fuera los que quieras guardar más tiempo.
4. Los archivos quedan en `/data/backup` dentro del contenedor. Para sacar uno del equipo:
   ```bash
   sudo docker cp mandebitcoin-mostro-manager_web_1:/data/backup/<archivo.age> .
   ```
   Una copia que solo vive en el mismo disco no protege frente a su pérdida.

> **La base del daemon va en un archivo aparte.** Desde la v1.0.16, cada respaldo guarda también la base de datos del daemon (`/data/config/active/mostro.db`: órdenes, disputas, usuarios, garantías y reputación) en `mostro-db-rev<N>-<fecha>.age`, junto al archivo de identidad y reglas. Se copia de forma consistente aunque el daemon esté en marcha, se cifra con la misma frase y se comprueba antes de darlo por bueno. Se restaura con `restore-db <archivo> <destino>`, en un archivo nuevo; ponerlo en lugar de `active/mostro.db` es un paso manual, con el daemon parado. Además, antes de cada versión nueva de mostrod el contenedor `mostro` guarda una copia en `/data/config/db-snapshots/`. Antes de migrar o reinstalar, espera a que no queden operaciones en curso (`open-orders`). El saldo y los canales dependen del respaldo de LND, que gestiona la app Lightning de Umbrel.

### Restauración
La restauración no se hace desde el panel. Se ejecuta por SSH, dentro del contenedor de la aplicación, y siempre sobre una carpeta nueva para no pisar una instalación en uso. Los dos comandos piden la frase en la terminal.

1. Comprueba que el archivo se abre con tu frase:
   ```bash
   sudo docker exec -it --user 1000:1000 mandebitcoin-mostro-manager_web_1 mostro-community-api verify-backup /data/backup/<archivo.age>
   ```
2. Restaura en una carpeta que aún no exista. Tiene que colgar de una carpeta privada (`0700`), como la de respaldos; el comando rechaza cualquier otro destino:
   ```bash
   sudo docker exec -it --user 1000:1000 mandebitcoin-mostro-manager_web_1 mostro-community-api restore-backup /data/backup/<archivo.age> /data/backup/restaurado
   ```
3. La carpeta restaurada contiene `community.json` y `identity/mostro.nsec`, sin cifrar. Úsala y bórrala. Aplicarla a una instalación es un paso manual: el comando no toca la configuración en uso. Para una instalación nueva, la clave restaurada se importa como en el paso 1 y las reglas se vuelven a guardar en **Configuración**.

---

## 6. Seguridad y Buenas Prácticas

- **Principio de Mínimo Privilegio:** El Community Manager opera con macaroon de solo lectura (`readonly.macaroon`) y nunca almacena claves privadas en texto claro en el navegador.
- **Permisos de Archivos:** Todo el almacenamiento sensible bajo `/data/config` se mantiene estrictamente en permisos `0700` para carpetas y `0600` para archivos.
- **Protección Anti-CSRF:** Todos los endpoints de mutación y lectura sensible exigen la cabecera `X-Requested-With: mostro-community` y validación estricta de origen.
- **Seguridad en Contenedores:** Los servicios en Docker Compose operan con `read_only: true`, `cap_drop: [ALL]`, `tmpfs: [/tmp]` y red interna segregada `manager_private`.
- **Cero Gasto Accidental:** Todas las funciones de monitoreo y simulación son de solo lectura o dry-run en memoria.
- **Lo único que el panel publica:** la tarjeta de la comunidad, y solo con su interruptor encendido (paso 7). Es el único evento que el panel firma con la clave del nodo. No es un mensaje del protocolo Mostro y no mueve fondos. El resto de lo que tu nodo anuncia lo publica el daemon.
