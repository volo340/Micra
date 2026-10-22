# Diseño de Micra

![Panel de un servidor encendido](img/panel.png)

![Asistente para crear un servidor](img/asistente.png)

## Logo

Un bloque de hierba que también es un servidor: la cara de arriba es la hierba
de Minecraft y la cara derecha lleva las ranuras y los LEDs de un rack.

- `app-icon.svg`: el icono de la app, con su fondo redondeado. Los iconos de
  Windows (`.ico`) y Linux se generan a partir de él:
  `npx tauri icon app-icon.svg -o src-tauri/icons`.
- `src/assets/logo.svg`: solo el bloque, para usar dentro de la interfaz.

## Colores

Salen del logo: el verde de la hierba es el color de marca y los grises
pizarra de las caras del bloque son las superficies. Todos están en
`src/styles/tokens.css`; ningún componente escribe colores sueltos.

| Token | Valor | Uso |
|---|---|---|
| `--brand` | `#2fd27a` | Acción principal, foco, servidor en línea |
| `--bg` | `#0a0e13` | Fondo de la ventana |
| `--surface`, `--surface-2`, `--surface-3` | `#121820`, `#18202a`, `#1f2833` | Tarjetas y controles, de menos a más elevados |
| `--text`, `--text-2`, `--text-3` | `#e9eef5`, `#a7b2c2`, `#6e7a8c` | Texto principal, secundario y discreto |
| `--warning` | `#f5b13d` | Avisos de la consola, servidor arrancando |
| `--critical` | `#f0565a` | Errores, servidor detenido por un fallo |
| `--series-cpu` | `#3987e5` | Gráfica de CPU |
| `--series-ram` | `#d55181` | Gráfica de memoria |

Micra solo tiene tema oscuro.

## Tipografía e iconos

- Inter Variable para el texto y JetBrains Mono Variable para la consola,
  las direcciones y las versiones. Van dentro de la app, así que se ven
  igual sin conexión a internet.
- Iconos de [Lucide](https://lucide.dev).

## Reglas

- Un estado nunca se indica solo con color: siempre lleva un icono, un punto
  o un texto ("En línea", "Iniciando…", "Se detuvo con un error").
- Los dos colores de las gráficas se distinguen también con daltonismo
  (deuteranopia, protanopia y tritanopia) y contrastan con las tarjetas. El
  texto nunca usa el color de una gráfica.
- Las gráficas son líneas finas de 2 px con un área suave debajo; al pasar
  el ratón muestran el valor y hace cuántos segundos se midió.
- Con "reducir movimiento" activado en el sistema, la app no anima nada.

## Probar la interfaz sin compilar la app

`npm run dev` abre la interfaz en el navegador (http://localhost:1420) con un
backend simulado (`src/dev/mock.ts`), sin Rust ni Tauri:

- `http://localhost:1420/?state=running`: un servidor encendido con jugadores.
- `http://localhost:1420/?state=empty`: la primera vez, sin servidores.

El simulador solo existe en desarrollo; no se incluye en los instaladores.
