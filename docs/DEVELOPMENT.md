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
```

## Publicar una versión

No hace falta compilar en tu PC: GitHub Actions genera los instaladores.

1. Sube la versión en `src-tauri/tauri.conf.json`, `package.json` y
   `Cargo.toml`.
2. Crea y sube la etiqueta: `git tag v0.1.0 && git push origin v0.1.0`.
3. Cuando termine el flujo **Build**, revisa el borrador en
   [Releases](https://github.com/volo340/Micra/releases) y publícalo.

Cada pull request también compila los instaladores; se descargan desde la
ejecución del flujo en la pestaña Actions, en el apartado *Artifacts*.
