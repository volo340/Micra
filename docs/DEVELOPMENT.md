# Desarrollo de Micra

Esta guía es para quien quiera compilar Micra o colaborar. Para usar la app
no hace falta nada de esto: basta con el instalador del
[README](../README.md#instalar).

## Compilar y probar

Hecho con Tauri 2 + Rust + React/TypeScript. La arquitectura está en
[ARCHITECTURE.md](ARCHITECTURE.md) y el diseño (logo, colores y
reglas de la interfaz) en [DESIGN.md](DESIGN.md).

Requisitos solo para desarrollar: Rust estable, Node 20+ y, en Linux, las
dependencias de Tauri (`libwebkit2gtk-4.1-dev`, `librsvg2-dev`,
`libayatana-appindicator3-dev`).

```bash
npm install
npm run tauri dev           # la app con recarga en caliente
npm run dev                 # solo la interfaz, en el navegador y con datos simulados
cargo test -p micra-core    # tests del núcleo
npx tsc -b                  # tipos del frontend
```

`npm run dev` usa el backend simulado de `src/dev/mock.ts`; los parámetros
de la dirección para probar cada caso están en
[DESIGN.md](DESIGN.md#probar-la-interfaz-sin-compilar-la-app).

## Textos e idiomas

La interfaz está en español, inglés, portugués, francés y alemán
(`src/i18n/`). `es.ts` es la referencia: los demás idiomas tienen su mismo
tipo, así que si añades un texto en español y falta en otro idioma, `tsc` no
compila. Los textos que nacen en Rust están en `crates/micra-core/src/i18n.rs`:
las notificaciones, en los cinco idiomas, y los errores (`tr!`), en español e
inglés (el resto de idiomas los ve en inglés).

## Publicar una versión

No hace falta compilar en tu PC: GitHub Actions genera los instaladores.

1. Sube la versión en `src-tauri/tauri.conf.json`, `Cargo.toml` y
   `package.json` (`npm version 0.2.0 --no-git-tag-version` cambia también
   `package-lock.json`).
2. Crea y sube la etiqueta: `git tag v0.2.0 && git push origin v0.2.0`.
3. Cuando termine el flujo **Build**, revisa el borrador en
   [Releases](https://github.com/volo340/Micra/releases) y publícalo. Al
   publicarlo, las copias de Micra ya instaladas ven la versión nueva.

Cada pull request también compila los instaladores; se descargan desde la
ejecución del flujo en la pestaña Actions, en el apartado *Artifacts*. Esos
instaladores no van firmados (las firmas solo se hacen al publicar una
versión o al lanzar el flujo a mano).

Para que la gente pueda descargar Micra y recibir las actualizaciones, el
repositorio tiene que ser público (o las releases tienen que estar en otro
sitio público): con el repositorio privado, ni `micra-install.sh` ni la app
pueden descargar nada de GitHub.

## Secretos de GitHub Actions

Se añaden en **Settings > Secrets and variables > Actions > New repository
secret**. Todos son opcionales: sin ellos el flujo compila igual y solo se
salta lo que depende de cada uno.

| Secreto | Para qué |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | Firma las actualizaciones automáticas. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Solo si la clave anterior tiene contraseña. |
| `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` | Firma del `.exe` con un certificado `.pfx`. |
| `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_TENANT_ID`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_SIGNING_PROFILE` | Firma del `.exe` con Azure Trusted Signing. |
| `MICRA_CURSEFORGE_API_KEY` | Deja la búsqueda de CurseForge activada en los instaladores. |

### Actualizaciones automáticas

Al abrirse (salvo que se desactive en **Ajustes**), la app lee
`https://github.com/volo340/Micra/releases/latest/download/latest.json`. El
flujo lo genera en cada versión con la dirección y la firma del instalador de
Windows y del AppImage, y la app solo instala un paquete si su firma
corresponde a la clave pública de `plugins.updater.pubkey` en
`src-tauri/tauri.conf.json`.

- Guarda la clave privada en el secreto `TAURI_SIGNING_PRIVATE_KEY` (el
  contenido entero del archivo `.key`). La clave actual no tiene contraseña.
- Si la pierdes, genera otra con `npx tauri signer generate -w micra.key`,
  pon la clave pública nueva en `tauri.conf.json` y la privada en el secreto.
  Las copias instaladas con la clave anterior no aceptarán la actualización:
  habrá que instalar esa versión a mano una vez.
- Con el `.deb` o el `.rpm` la app avisa de la versión nueva pero no se
  actualiza sola: basta con volver a ejecutar `micra-install.sh`.

### Firma del `.exe` en Windows

Sin firma, SmartScreen muestra "Windows protegió su PC" la primera vez que
alguien abre el instalador (se abre con *Más información* > *Ejecutar de
todas formas*). El flujo firma el instalador y el programa de dos formas:

- **Certificado `.pfx`**: conviértelo a base64
  (`[Convert]::ToBase64String([IO.File]::ReadAllBytes("micra.pfx"))` en
  PowerShell o `base64 -w0 micra.pfx` en Linux) y guárdalo en
  `WINDOWS_CERTIFICATE`, con su contraseña en `WINDOWS_CERTIFICATE_PASSWORD`.
  Desde junio de 2023 los certificados nuevos se entregan en un token físico o
  en la nube y no se pueden exportar a `.pfx`; esta opción sirve si tu
  proveedor sí te da el archivo.
- **[Azure Trusted Signing](https://learn.microsoft.com/azure/trusted-signing/)**
  (ahora llamado *Artifact Signing*): crea la cuenta y el perfil de
  certificado, y un registro de aplicación con el rol *Certificate Profile
  Signer* sobre ese perfil. Guarda sus datos en
  `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET` y `AZURE_TENANT_ID`, la dirección
  de la región en `AZURE_SIGNING_ENDPOINT` (por ejemplo
  `https://weu.codesigning.azure.net`), y los nombres de la cuenta y del perfil
  en `AZURE_SIGNING_ACCOUNT` y `AZURE_SIGNING_PROFILE`.

Si están las dos, se usa el `.pfx`. El paso *Reunir instaladores* del flujo
muestra si cada `.exe` quedó firmado.

### CurseForge

La API de CurseForge pide una clave gratuita (console.curseforge.com, página
«API keys»). En la app, cada persona puede poner la suya en **Ajustes**; con
el secreto `MICRA_CURSEFORGE_API_KEY` los instaladores ya la traen. Si Micra
se distribuye públicamente, conviene un pequeño proxy propio para no incrustar
la clave en el binario.
