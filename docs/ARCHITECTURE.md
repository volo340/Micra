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
│  React + TS: asistente de creación, buscador, panel (consola, CPU/RAM,   │
│  jugadores), ajustes                                                     │
└───────────────▲──────────────────────────────────────┬───────────────────┘
                │ eventos: server://event,              │ invoke("start_server", …)
                │ server://stats, provision://progress  │
┌───────────────┴──────────────────────────────────────▼───────────────────┐
│ src-tauri (capa fina): comandos, estado global, reenvío de eventos       │
└───────────────▲──────────────────────────────────────┬───────────────────┘
                │                                       │
┌───────────────┴──────────────── micra-core ──────────▼───────────────────┐
│ provision   ─ orquesta el asistente: contenido → Java → loader → EULA    │
│ providers   ─ mojang · modrinth (+ .mrpack) · curseforge · loaders       │
│ java        ─ runtimes Temurin aislados (API de Adoptium)                │
│ server      ─ config · launcher (flags) · affinity · process · monitor   │
│ download / archive ─ descargas con hash, extracción segura               │
│ net (fase 2) ─ UPnP/NAT-PMP · túnel (playit.gg)                          │
└──────────────────────────────────────────────────────────────────────────┘
                │ proceso hijo: <runtimes>/java-21/bin/java -Xmx… -jar …
                ▼
         Servidor de Minecraft (Vanilla / Fabric / Forge / NeoForge)
```

### Flujo "Crear servidor"

1. La interfaz manda un `NewServerRequest` (nombre, fuente, RAM, núcleos,
   puerto, EULA aceptado). Se valida **antes** de descargar nada.
2. Se obtiene el contenido:
   - Vanilla/Fabric: versión elegida.
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
6. Escribe `eula.txt`, `server.properties` (puerto, MOTD) y `micra.json`.

### Flujo "Encender"

`launcher::build_command` monta `java -Xms -Xmx [flags de Aikar] -jar X nogui`,
con la afinidad aplicada en el hijo antes del `exec` (Linux) o justo tras crear
el proceso (Windows). `ServerHandle` lee stdout/stderr línea a línea, detecta
`Done (…)` (en línea), `X joined/left the game` (jugadores), y permite enviar
comandos por stdin. *Apagar* envía `stop` y solo mata el proceso si no termina
a tiempo. Al cerrar la app se apagan todos los servidores de forma ordenada.

## 3. Estructura de carpetas

```
Micra/
├─ Cargo.toml                    workspace de Rust
├─ package.json, vite.config.ts  frontend (Vite + React + TS)
├─ index.html
├─ app-icon.svg                  logo original (npx tauri icon genera los iconos)
├─ .github/workflows/build.yml   compila los instaladores en la nube
├─ scripts/install.sh            instalador de Linux (micra-install.sh)
├─ docs/ARCHITECTURE.md          este documento
├─ docs/DESIGN.md                logo, colores y reglas de la interfaz
├─ crates/micra-core/            lógica de negocio, sin Tauri, con tests
│  ├─ src/
│  │  ├─ lib.rs, error.rs, paths.rs, http.rs, hardware.rs, net.rs
│  │  ├─ download.rs             descargas en streaming + SHA-1/256/512
│  │  ├─ archive.rs              ZIP / tar.gz con protección zip-slip
│  │  ├─ provision.rs            orquestación del asistente
│  │  ├─ java/
│  │  │  ├─ mod.rs               JavaManager, heurística de versión
│  │  │  └─ adoptium.rs          API de Eclipse Temurin
│  │  ├─ providers/
│  │  │  ├─ mojang.rs            manifiesto oficial de versiones
│  │  │  ├─ modrinth.rs          búsqueda y versiones (API v2)
│  │  │  ├─ mrpack.rs            instalación de .mrpack en modo servidor
│  │  │  ├─ curseforge.rs        búsqueda, archivos, server packs
│  │  │  └─ loaders.rs           Vanilla, Fabric, Forge, NeoForge
│  │  └─ server/
│  │     ├─ config.rs            micra.json + validación
│  │     ├─ launcher.rs          argumentos de la JVM
│  │     ├─ affinity.rs          afinidad de CPU (Linux / Windows)
│  │     ├─ process.rs           proceso vivo: consola, comandos, jugadores
│  │     ├─ monitor.rs           CPU/RAM del proceso
│  │     ├─ detect.rs            detectar jar / args file
│  │     ├─ eula.rs, properties.rs
│  └─ tests/process_lifecycle.rs
├─ src-tauri/                    capa de escritorio
│  ├─ tauri.conf.json, capabilities/, icons/
│  └─ src/ main.rs, state.rs, commands.rs
└─ src/                          interfaz
   ├─ main.tsx, App.tsx
   ├─ assets/logo.svg            el logo sin fondo
   ├─ styles/                    tokens.css (colores, tipografía), base, componentes, vistas
   ├─ components/                barra lateral, métricas, consola, jugadores, avisos
   ├─ views/                     inicio, panel, ajustes y el asistente (wizard/)
   ├─ lib/                       api.ts, types.ts, useAppData.ts, logs.ts, format.ts
   └─ dev/mock.ts                backend simulado para `npm run dev` en el navegador
```

Datos del usuario (fuera del repo):

```
Linux:   ~/.local/share/micra/
Windows: %APPDATA%\Micra\Micra\data\
  ├─ runtimes/java-8, java-17, java-21, …
  ├─ servers/<id>/  (mundo, mods, micra.json, eula.txt, server.properties)
  └─ cache/         (descargas temporales)
```

## 4. Decisiones importantes

- **EULA**: legalmente lo acepta la persona, no la app. El asistente muestra
  el enlace y una casilla; `eula.txt` solo se escribe si llega
  `eulaAccepted: true`. Es "automático" en el sentido de que nadie edita
  archivos.
- **API key de CurseForge**: la API exige clave (gratis en
  console.curseforge.com). Se lee de `MICRA_CURSEFORGE_API_KEY` en ejecución
  o al compilar. Si la app se distribuye públicamente, conviene un pequeño
  proxy propio para no incrustar la clave en el binario. Algunos autores
  bloquean la descarga por terceros: la app lo detecta y lo explica.
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

Pendiente para una distribución pública:

- **Firma de código en Windows**: sin certificado, SmartScreen muestra
  "Windows protegió su PC" la primera vez (se abre con *Más información* >
  *Ejecutar de todas formas*). Se quita firmando el `.exe` en el flujo de
  Actions.
- **Repositorio público** (o releases en otro sitio): con el repositorio
  privado nadie más puede descargar los instaladores.
- **Actualizaciones automáticas** con `tauri-plugin-updater`.

## 6. Redes (fase 2)

1. **UPnP / NAT-PMP** con el crate `igd-next`: descubrir el gateway, abrir
   TCP 25565 (o el puerto elegido) con lease renovable, cerrarlo al apagar, y
   mostrar la IP pública.
2. **Detección de CGNAT**: si la IP WAN que da el router es privada
   (100.64.0.0/10, 10/8…) o no coincide con la IP pública, UPnP no sirve.
3. **Túnel** como alternativa: integrar el agente de playit.gg (código
   abierto, en Rust) como proceso gestionado, que da una dirección del tipo
   `nombre.joinmc.link`. Los nombres personalizados dependen del plan de
   playit; con el plan gratuito la dirección se asigna automáticamente.
4. **Firewall de Windows**: la primera vez que Java abre el puerto, Windows
   pregunta si permitirlo. La app puede añadir la regla por adelantado
   (`netsh advfirewall`, requiere confirmar como administrador una vez).

## 7. Hoja de ruta

1. ✅ Núcleo: Java, proveedores, lanzamiento, consola, CPU/RAM.
2. ✅ Instaladores: `.exe` (Windows), `micra-install.sh` (Linux) y compilación en GitHub Actions.
3. ✅ Interfaz: logo, panel en tiempo real, asistente de creación y buscador de Modrinth/CurseForge.
4. Redes: UPnP, túnel y regla de firewall.
5. Copias de seguridad del mundo, programación de reinicios, editor de
   `server.properties`, gestor de mods y plugins.
6. Firma de código y actualizaciones automáticas (`tauri-plugin-updater`).

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
