# Arquitectura de Micra

Micra es una app de escritorio (Windows y Linux) que convierte un PC en un
servidor de Minecraft sin tocar terminal ni router: elige un modpack, ajusta
RAM y núcleos, acepta el EULA y pulsa *Encender*.

## 1. Pila tecnológica

**Tauri 2 + Rust (backend) + React/TypeScript (interfaz).**

| Criterio | Tauri + Rust | Electron + Node | Python + PyQt/CustomTkinter |
|---|---|---|---|
| RAM de la propia app | ~40–80 MB (usa el WebView del sistema) | ~150–300 MB (incluye Chromium) | ~60–120 MB |
| Tamaño del instalador | ~5–10 MB | ~80–120 MB | ~40–80 MB (con PyInstaller) |
| Gestión de procesos y señales | Nativa (`tokio::process`, `libc`, `windows-sys`) | `child_process`; afinidad de CPU requiere addons nativos | Buena (`subprocess`, `psutil`) |
| Descargas concurrentes y hashing | Async real y muy rápido | Bien | Limitado por el GIL |
| UI moderna con modo oscuro | React/CSS, sin límites | React/CSS, sin límites | Más laboriosa |
| Distribución multiplataforma | `tauri build` genera .msi/.exe, .deb, .rpm, AppImage | Igual de completa | PyInstaller es frágil |

Por qué Tauri gana aquí:

1. **La app compite por recursos con el propio servidor de Minecraft.** Cada
   MB que no gasta la interfaz es RAM para el servidor. Electron cargaría un
   Chromium entero solo para pintar un panel.
2. **El núcleo del producto es gestión de procesos del sistema**: lanzar Java,
   fijar afinidad de CPU (`sched_setaffinity` / `SetProcessAffinityMask`),
   leer la consola en streaming, apagar sin corromper mundos. Rust da acceso
   directo a esas APIs sin addons nativos ni dependencias externas (`taskset`).
3. **Seguridad**: se descargan y descomprimen archivos de terceros (modpacks,
   runtimes). Rust evita clases enteras de errores de memoria, y la extracción
   valida cada ruta contra *zip slip*.
4. **La interfaz sigue siendo web** (React + TypeScript), así que el panel en
   tiempo real, las gráficas y el modo oscuro se hacen con herramientas
   conocidas.

Coste a asumir: Rust tiene más curva que Python/Node. Por eso el núcleo está
aislado en un crate sin Tauri (`micra-core`) que se prueba con `cargo test`.

## 2. Vista general

```
┌──────────────────────────── Ventana (WebView) ───────────────────────────┐
│  React + TS en 5 idiomas: asistente, panel (consola, CPU/RAM/TPS,        │
│  jugadores), jugadores, copias, mods, estadísticas, ajustes              │
└───────────────▲──────────────────────────────────────┬───────────────────┘
                │ eventos: server://event, server://   │ invoke("start_server", …)
                │ stats, server://live, task://progress │
┌───────────────┴──────────────────────────────────────▼───────────────────┐
│ src-tauri: comandos, estado, supervisor de cada servidor en marcha,      │
│ avisos (escritorio y Discord), agente del túnel, actualizador            │
└───────────────▲──────────────────────────────────────┬───────────────────┘
                │                                       │
┌───────────────┴──────────────── micra-core ──────────▼───────────────────┐
│ provision   ─ orquesta el asistente: contenido → Java → loader → EULA    │
│ import      ─ adopta un servidor que ya existe                           │
│ providers   ─ mojang · modrinth (+ .mrpack) · curseforge · loaders       │
│ java        ─ runtimes Temurin aislados (API de Adoptium)                │
│ server      ─ config · launcher · affinity · process · monitor · tps     │
│               backup · schedule · players · properties · mods · pack     │
│               stats · map (BlueMap) · ports · guard · sandbox · scan     │
│ net         ─ UPnP · túnel de playit.gg · comprobación desde internet    │
│ notify · secrets (llavero) · settings · i18n                             │
│ download / archive ─ descargas HTTPS con hash, extracción segura         │
└──────────────────────────────────────────────────────────────────────────┘
                │ proceso hijo: <runtimes>/java-21/bin/java -Xmx… -jar …
                ▼
         Servidor de Minecraft (Vanilla / Fabric / Forge / NeoForge)
```

### Flujo "Crear servidor"

1. La interfaz manda un `NewServerRequest` (nombre, fuente, RAM, núcleos,
   puerto, EULA aceptado). Se valida **antes** de descargar nada.
2. Se obtiene el contenido:
   - Vanilla, Fabric, Forge o NeoForge: versión elegida. El asistente solo
     ofrece las versiones que tienen ese loader (meta.fabricmc.net, las
     promociones de Forge y el Maven de NeoForge) y usa la última versión
     estable del loader.
   - Modrinth: se descarga el `.mrpack`, se bajan solo los archivos con
     `env.server != "unsupported"` (8 en paralelo, verificados con SHA-512 y
     solo desde dominios permitidos), y se aplican `overrides/` y
     `server-overrides/`.
   - CurseForge: se localiza el *server pack* oficial del archivo y se
     extrae.
3. Versión de Java: se lee `javaVersion.majorVersion` del manifiesto de
   Mojang (exacto para cada versión); si falla, heurística
   (≤1.16 → 8, 1.17–1.20.4 → 17, 1.20.5+ → 21, 26.x → 25).
4. `JavaManager::ensure(n)` descarga Temurin JRE `n` a
   `<datos>/runtimes/java-n` verificando su SHA-256. Nunca toca el PATH ni el
   Java del sistema.
5. Instala el loader si hace falta (jar de Fabric, o el instalador oficial de
   Forge/NeoForge con `--installServer`) y detecta cómo arrancar (jar o
   `@libraries/.../unix_args.txt`).
6. Escribe `eula.txt`, `server.properties` (puerto, MOTD y valores seguros:
   `online-mode`, lista blanca obligatoria, RCON y query apagados) y
   `micra.json`.
7. Si en el asistente se eligieron mods sueltos, la interfaz los instala uno
   a uno desde Modrinth (con sus dependencias) cuando el servidor ya existe;
   el que falla se avisa al final y no deshace el servidor.

*Importar* sigue el mismo camino con una carpeta ya existente: la copia a la
carpeta de servidores (la original no se toca), detecta versión, loader, mods
y cómo arrancar, y añade su `micra.json`.

### Flujo "Encender"

`launcher::build_command` monta `java -Xms -Xmx [flags de Aikar] -jar X nogui`,
con la afinidad aplicada en el hijo antes del `exec` (Linux) o justo tras crear
el proceso (Windows). `ServerHandle` lee stdout/stderr línea a línea, detecta
`Done (…)` (en línea), `X joined/left the game` (jugadores), y permite enviar
comandos por stdin. *Apagar* envía `stop` y solo mata el proceso si no termina
a tiempo. Al cerrar la app se apagan todos los servidores de forma ordenada.

Antes de arrancar, el supervisor (`src-tauri/src/supervisor.rs`) comprueba que
el puerto esté libre y haya RAM suficiente, deja `server.properties` al día y
analiza los mods en busca de malware conocido; si encuentra algo, no arranca.
También aplica al mundo lo que `server.properties` ya no controla: el modo
extremo de un mundo ya creado se escribe en su `level.dat` (`nbt.rs`,
`server/world.rs`), y desde 1.21.9 el PvP, los monstruos, el Nether y los
bloques de comandos son reglas del juego que se envían con `gamerule` al
arrancar o al cambiarlas (`server/gamerules.rs`).
El proceso se lanza con permisos mínimos (Landlock en Linux, Job Object en
Windows). Mientras funciona, el supervisor mide CPU, RAM y TPS, guarda un
punto de estadísticas por minuto, bloquea IPs que fallan demasiados intentos,
hace las copias automáticas y los reinicios programados, mantiene abierto el
puerto (UPnP) o el túnel, y avisa en el escritorio o en Discord.

## 3. Estructura de carpetas

```
Micra/
├─ Cargo.toml                    workspace de Rust
├─ package.json, vite.config.ts  frontend (Vite + React + TS)
├─ index.html
├─ app-icon.svg                  logo original (npx tauri icon genera los iconos)
├─ .github/workflows/build.yml   compila, firma y publica los instaladores
├─ scripts/install.sh            instalador de Linux (micra-install.sh)
├─ docs/ARCHITECTURE.md          este documento
├─ docs/DESIGN.md                logo, colores y reglas de la interfaz
├─ docs/DEVELOPMENT.md           compilar, probar, secretos y publicar versiones
├─ crates/micra-core/            lógica de negocio, sin Tauri, con tests
│  ├─ src/
│  │  ├─ lib.rs, error.rs, paths.rs, http.rs, hardware.rs, fsutil.rs
│  │  ├─ i18n.rs                 idioma de errores y avisos
│  │  ├─ settings.rs, secrets.rs ajustes de la app y llavero del sistema
│  │  ├─ notify.rs               textos de los avisos y webhook de Discord
│  │  ├─ download.rs             descargas en streaming + SHA-1/256/512
│  │  ├─ archive.rs              ZIP / tar.gz con protección zip-slip
│  │  ├─ nbt.rs                  leer y escribir level.dat (NBT con gzip)
│  │  ├─ provision.rs            orquestación del asistente
│  │  ├─ import.rs               importar un servidor existente
│  │  ├─ java/                   JavaManager y API de Eclipse Temurin
│  │  ├─ providers/              mojang, modrinth, mrpack, curseforge, loaders
│  │  ├─ net/                    IP local, upnp.rs, playit.rs
│  │  └─ server/
│  │     ├─ config.rs            micra.json + validación
│  │     ├─ launcher.rs          argumentos de la JVM
│  │     ├─ affinity.rs          afinidad de CPU (Linux / Windows)
│  │     ├─ process.rs           proceso vivo: consola, comandos, jugadores
│  │     ├─ monitor.rs, tps.rs   CPU/RAM del proceso y TPS del servidor
│  │     ├─ stats.rs             histórico (CSV por día, 31 días)
│  │     ├─ backup.rs            copias en ZIP y restauración
│  │     ├─ schedule.rs          reinicios programados y apagado sin jugadores
│  │     ├─ players.rs           lista blanca, ops, baneos
│  │     ├─ properties.rs        server.properties: lectura, validación, escritura
│  │     ├─ gamerules.rs         reglas del juego según la versión
│  │     ├─ world.rs             modo extremo y reglas guardadas en el mundo
│  │     ├─ mods.rs, pack.rs     mods de Modrinth y actualización de modpacks
│  │     ├─ map.rs               BlueMap
│  │     ├─ ports.rs, version.rs
│  │     ├─ guard.rs             bloqueo de IPs y límite por IP
│  │     ├─ sandbox/             Landlock (Linux) y Job Object (Windows)
│  │     ├─ scan.rs              malware conocido en los .jar
│  │     ├─ detect.rs            detectar jar / args file
│  │     └─ eula.rs
│  └─ tests/process_lifecycle.rs
├─ src-tauri/                    capa de escritorio
│  ├─ tauri.conf.json, capabilities/, icons/
│  └─ src/
│     ├─ main.rs, state.rs, events.rs
│     ├─ supervisor.rs           arranque y vigilancia de cada servidor
│     ├─ notifier.rs, tunnel.rs  avisos y agente de playit.gg
│     └─ commands/               app, servers, content, import, players,
│                                properties, backups, mods, map, network, stats
└─ src/                          interfaz
   ├─ main.tsx, App.tsx
   ├─ assets/logo.svg            el logo sin fondo
   ├─ i18n/                      es (referencia), en, pt, fr, de
   ├─ styles/                    tokens.css (colores, tipografía), base, componentes, vistas
   ├─ components/                barra lateral, métricas, consola, gráficas, diálogos
   ├─ views/                     inicio, ajustes, asistente (wizard/) y las
   │                             pestañas de cada servidor (server/)
   ├─ lib/                       api.ts, types.ts, useAppData.ts, logs.ts, format.ts
   └─ dev/mock.ts                backend simulado para `npm run dev` en el navegador
```

Datos del usuario (fuera del repo):

```
Linux:   ~/.local/share/micra/
Windows: %APPDATA%\Micra\Micra\data\
  ├─ runtimes/java-8, java-17, java-21, …
  ├─ servers/<id>/  (mundo, mods, micra.json, eula.txt, server.properties)
  ├─ backups/<id>/  (copias en ZIP)
  ├─ stats/<id>/    (un CSV por día)
  ├─ tools/         (agente de playit.gg)
  ├─ settings.json  (idioma, avisos, actualizaciones)
  └─ cache/         (descargas temporales)
```

## 4. Decisiones importantes

- **EULA**: legalmente lo acepta la persona, no la app. El asistente muestra
  el enlace y una casilla; `eula.txt` solo se escribe si llega
  `eulaAccepted: true`. Es "automático" en el sentido de que nadie edita
  archivos.
- **API key de CurseForge**: la API exige clave (gratis en
  console.curseforge.com). La persona puede ponerla en Ajustes (se guarda en
  el llavero) o se lee de `MICRA_CURSEFORGE_API_KEY` en ejecución o al
  compilar. Si la app se distribuye públicamente, conviene un pequeño proxy
  propio para no incrustar la clave en el binario. Algunos autores bloquean
  la descarga por terceros: la app lo detecta y lo explica.
- **Seguro por defecto**: todo servidor nace con `online-mode` y lista blanca
  obligatoria; si la persona los quita, el panel lo avisa. RCON y query
  quedan apagados (Micra usa la consola); si se activa RCON, Micra le pone una
  contraseña de 32 caracteres y nunca abre su puerto. UPnP y el túnel están
  apagados hasta que la persona los activa.
- **Secretos** (webhook de Discord, clave de CurseForge, clave del agente de
  playit.gg) en el llavero del sistema; sin llavero, en un archivo que solo
  lee el usuario.
- **El escáner de malware no es un antivirus**: busca indicadores concretos
  de familias conocidas (fractureiser y ladrones de cuentas) en las cadenas de
  las clases, también en jars anidados. Reduce el riesgo; no lo elimina.
- **Afinidad en Windows**: `SetProcessAffinityMask` cubre el primer grupo de
  64 CPUs lógicas; suficiente para equipos domésticos.
- **Flags de la JVM**: flags de G1 de Aikar activados por defecto
  (desactivables), `-Xms`/`-Xmx` del asistente, y límite de RAM validado
  contra la RAM total para no dejar sin memoria al sistema.

## 5. Distribución e instalación

El usuario final no instala ninguna herramienta de desarrollo. Node y Rust
solo existen en la máquina que compila, y esa máquina es GitHub Actions
(`.github/workflows/build.yml`). El resultado es un binario nativo.

| | Windows | Linux |
|---|---|---|
| Qué descarga el usuario | `Micra_x.y.z_x64-setup.exe` (NSIS) | `micra-install.sh` |
| Dependencias del sistema | WebView2: viene con Windows 10/11; si falta, el instalador lo descarga solo | WebKitGTK y GTK: apt/dnf/zypper las instalan al instalar el `.deb`/`.rpm`; el AppImage las lleva dentro |
| Permisos | Instalación por usuario, sin administrador | sudo solo para `.deb`/`.rpm`; `--appimage` no lo necesita |
| Java | Lo descarga la app al crear cada servidor (versión exacta) | Igual |

`micra-install.sh` usa primero los paquetes que haya junto a él (útil con los
artefactos de Actions o con el repositorio privado) y, si no hay, descarga la
release de GitHub. En distribuciones sin FUSE 2, el lanzador del AppImage
activa `APPIMAGE_EXTRACT_AND_RUN` para que arranque igual.

Para una distribución pública:

- **Firma de código en Windows**: el flujo de Actions firma el `.exe` si
  tiene un certificado (`.pfx` o Azure Trusted Signing; ver
  [DEVELOPMENT.md](DEVELOPMENT.md#firma-del-exe-en-windows)). Sin él,
  SmartScreen muestra "Windows protegió su PC" la primera vez (se abre con
  *Más información* > *Ejecutar de todas formas*).
- **Actualizaciones automáticas** con `tauri-plugin-updater`: cada versión
  publica un `latest.json` con los paquetes firmados con la clave de Micra.
  Se instalan solas en Windows y con el AppImage; con `.deb`/`.rpm` la app
  avisa y basta con volver a ejecutar `micra-install.sh`.
- **Repositorio público** (o releases en otro sitio): con el repositorio
  privado nadie más puede descargar los instaladores ni las actualizaciones.

## 6. Redes

1. **UPnP** con el crate `igd-next`: abre solo el puerto TCP del juego con
   una concesión de una hora que se renueva mientras el servidor está
   encendido, lo cierra al apagar y muestra la IP pública.
2. **Detección de CGNAT**: si la IP WAN que da el router es privada
   (100.64.0.0/10, 10/8…), abrir el puerto no sirve y la interfaz propone el
   túnel.
3. **Túnel de playit.gg**: Micra vincula la cuenta (código que se acepta en
   el navegador), crea un túnel `minecraft-java` por servidor y ejecuta el
   agente oficial, descargado de sus releases y comprobado con SHA-256. Da
   direcciones del tipo `nombre.joinmc.link`.
4. **Comprobar**: se pregunta a mcstatus.io si el servidor responde en la
   dirección pública, para confirmar que se puede entrar desde internet.
5. **Pendiente: firewall de Windows**. La primera vez que Java abre el
   puerto, Windows pregunta si permitirlo. La app podría añadir la regla por
   adelantado (`netsh advfirewall`, requiere confirmar como administrador
   una vez).

## 7. Hoja de ruta

1. ✅ Núcleo: Java, proveedores, lanzamiento, consola, CPU/RAM.
2. ✅ Instaladores: `.exe` (Windows), `micra-install.sh` (Linux) y compilación en GitHub Actions.
3. ✅ Interfaz: logo, panel en tiempo real, asistente de creación y buscador de Modrinth/CurseForge.
4. ✅ Redes: UPnP, detección de CGNAT y túnel de playit.gg.
5. ✅ Copias de seguridad, reinicios programados, editor de `server.properties`,
   jugadores, mods y actualizaciones de modpacks, importar servidores, BlueMap,
   avisos, estadísticas y cinco idiomas.
6. ✅ Seguridad: valores seguros por defecto, RCON protegido, bloqueo de IPs,
   permisos mínimos, análisis de malware y secretos en el llavero.
7. ✅ Firma de código (con el certificado de quien publica) y actualizaciones
   automáticas.
8. Regla del firewall de Windows.
9. Control desde el móvil.

## Desarrollo

Requisitos (solo para desarrollar, nunca para usar la app): Rust estable,
Node 20+, y en Linux las dependencias de Tauri (`libwebkit2gtk-4.1-dev`,
`libayatana-appindicator3-dev`, `librsvg2-dev`).

```bash
npm install
npm run tauri dev                # app con recarga en caliente
cargo test -p micra-core         # tests del núcleo
MICRA_CURSEFORGE_API_KEY=… npm run tauri dev   # con CurseForge
```
