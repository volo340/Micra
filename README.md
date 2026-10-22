<p align="center"><img src="app-icon.svg" width="96" alt="Logo de Micra" /></p>

# Micra

Convierte tu PC (Windows o Linux) en un servidor de Minecraft sin escribir
código ni configurar el router: modpacks de Modrinth y CurseForge, Java
gestionado automáticamente, copias de seguridad, juego por internet y panel
en tiempo real.

![Panel de Micra con un servidor encendido](docs/img/panel.png)

## Instalar

No hace falta instalar nada más: ni Java, ni Node, ni Rust. Micra descarga
la versión de Java que necesita cada servidor la primera vez que lo creas.

**Windows**: descarga `Micra_x.y.z_x64-setup.exe` de la página de
[Releases](https://github.com/volo340/Micra/releases) y ábrelo. Se instala
para tu usuario (no pide permisos de administrador) y, si el equipo no tiene
WebView2, lo instala solo.

**Linux**: descarga `micra-install.sh` de la misma página y ejecútalo:

```bash
bash micra-install.sh
```

El script elige el mejor formato para tu distribución:

- Debian, Ubuntu, Mint, Pop!_OS: instala el `.deb` con apt, que añade las
  librerías del sistema que falten.
- Fedora y openSUSE: instala el `.rpm` con dnf o zypper.
- Cualquier otra distribución, o si no tienes sudo: instala el AppImage en tu
  carpeta de usuario y crea el acceso directo en el menú.

Otras opciones: `--appimage` (solo para tu usuario, sin sudo) y `--uninstall`
(desinstala; tus servidores y mundos se conservan).

Mientras el repositorio sea privado, el script no puede descargar nada de
GitHub por su cuenta: descarga los archivos de la release o del artefacto de
Actions y ejecuta `bash micra-install.sh` en esa misma carpeta.

### Comandos para cada distribución

Si prefieres instalarlo a mano, descarga el paquete de tu distribución, abre
una terminal en esa carpeta (normalmente `~/Descargas`) y ejecuta su comando.
Cambia `0.1.0` por la versión que hayas descargado.

**Ubuntu, Debian, Linux Mint, Pop!_OS, Zorin OS y elementary OS**

```bash
sudo apt install ./Micra_0.1.0_amd64.deb
```

**Fedora**

```bash
sudo dnf install ./Micra-0.1.0-1.x86_64.rpm
```

**openSUSE**

```bash
sudo zypper install --allow-unsigned-rpm ./Micra-0.1.0-1.x86_64.rpm
```

**Arch Linux, Manjaro, EndeavourOS y cualquier otra distribución (AppImage)**

```bash
chmod +x Micra_0.1.0_amd64.AppImage
./Micra_0.1.0_amd64.AppImage
```

Si el AppImage no arranca porque falta FUSE, instala `fuse2` (en Arch:
`sudo pacman -S fuse2`) o ábrelo con
`APPIMAGE_EXTRACT_AND_RUN=1 ./Micra_0.1.0_amd64.AppImage`.

Para desinstalar: `sudo apt remove micra`, `sudo dnf remove micra` o
`sudo zypper remove micra`. Si usas el AppImage, basta con borrar el archivo.
Tus servidores y mundos se conservan.

## Qué hace Micra

Montar un servidor de Minecraft suele obligar a descargar archivos a mano,
instalar la versión correcta de Java, editar ficheros de configuración y
abrir una consola. Micra lo hace todo por ti desde una sola ventana:

- **Crea un servidor en cuatro pasos.** Eliges el tipo (Vanilla, Fabric o un
  modpack), la versión de Minecraft, el nombre y la memoria, y Micra lo
  descarga y lo deja listo para encender. Si ya tienes un servidor, elige su
  carpeta: Micra lo copia, detecta la versión y los mods, y lo deja listo.
- **Busca modpacks por ti.** Busca por nombre o por tema y elige entre los
  modpacks de Modrinth y CurseForge. Micra descarga solo lo que necesita el
  servidor e instala el loader que use (Fabric, Forge o NeoForge).
- **Instala Java solo.** Cada versión de Minecraft necesita un Java distinto.
  Micra descarga el correcto y lo guarda en su propia carpeta, sin tocar el
  Java de tu sistema.
- **Ajusta los recursos a tu PC.** Te recomienda cuánta RAM darle según tu
  equipo y te deja elegir qué núcleos del procesador usa el servidor, para
  que el resto quede libre para jugar.
- **Se ocupa del EULA.** Marcas la casilla de aceptación una vez y Micra lo
  deja aceptado en los archivos del servidor.
- **Panel en tiempo real.** Enciende, apaga o reinicia con un botón. Ves la
  consola con colores, el uso de CPU y memoria, los TPS (si el servidor va
  fluido) y quién está conectado, y puedes escribir comandos.
- **Jugadores.** Lista blanca, operadores, baneos y expulsiones desde su
  pestaña, sin escribir comandos. Si alguien que no está en la lista intenta
  entrar, Micra te avisa y puedes dejarle pasar con un clic.
- **Copias de seguridad.** Copia el mundo cada pocas horas sin apagar el
  servidor y restaura cualquier copia con un clic. Antes de restaurar o de
  actualizar un modpack guarda el estado actual, por si quieres volver atrás.
- **Reinicios y apagado automáticos.** Reinicia a una hora fija o cada cierto
  tiempo (avisando antes en el chat) y, si quieres, apaga el servidor cuando
  no queda nadie.
- **Ajustes de Minecraft sin editar archivos.** Dificultad, modo de juego,
  distancia de visión, mensaje de la lista de servidores y el resto de
  `server.properties`, con una explicación en cada opción.
- **Mods y modpacks al día.** Busca e instala mods de Modrinth que valen para
  tu servidor, te avisa cuando hay versiones nuevas y actualiza el modpack
  entero conservando tu mundo y los mods que añadiste tú.
- **Mapa web.** En servidores con Fabric, Forge o NeoForge, instala BlueMap:
  un mapa 3D de tu mundo que se abre en el navegador.
- **Estadísticas.** Gráficas de CPU, memoria, jugadores y TPS de la última
  hora, día, semana o mes.
- **Avisos.** En el escritorio o en un canal de Discord cuando alguien entra,
  el servidor se detiene por un error, hay un problema de seguridad o falla
  una copia.
- **Varios servidores a la vez.** Crea todos los que quieras, cada uno con su
  puerto y sus recursos, y cambia entre ellos desde la barra lateral.
- **En tu idioma.** Español, inglés, portugués, francés y alemán; por defecto,
  el de tu sistema.
- **Se actualiza sola.** Cuando sale una versión nueva, Micra te avisa y se
  instala con un clic (en Windows y con el AppImage; si la instalaste con el
  `.deb` o el `.rpm`, vuelve a ejecutar `micra-install.sh`).
- **Cierra sin miedo.** Al salir de Micra, los servidores se apagan de forma
  ordenada y los mundos se guardan.

## Jugar con tus amigos

1. Enciende el servidor desde su panel.
2. Copia una dirección del recuadro **Cómo entrar**: la de **En tu red** sirve
   para quien esté conectado a tu misma red (Wi-Fi o cable); la de **Desde
   internet**, para todos los demás.
3. En Minecraft, entra en **Multijugador**, pulsa **Añadir servidor** y pega
   esa dirección.

Abrir el servidor a internet lo decides tú: pulsa **Jugar desde internet** en
ese recuadro y activa **Abrir el puerto en el router (UPnP)**. Micra abre el
puerto mientras el servidor está encendido y lo cierra al apagarlo; con
**Comprobar** ves si responde desde fuera. Si tu router no lo permite o tu
compañía comparte tu IP con otros clientes (CGNAT), usa el túnel gratuito de
[playit.gg](https://playit.gg): vincula tu cuenta en **Ajustes > Túnel de
playit.gg** y actívalo en los ajustes del servidor. Tus amigos entrarán con una
dirección del tipo `nombre.joinmc.link`.

Los servidores nuevos solo dejan entrar a quien esté en su lista blanca:
añade a tus amigos en la pestaña **Jugadores** o pulsa **Dejar entrar** cuando
lo intenten.

La primera vez que enciendas un servidor en Windows, el sistema puede
preguntar si Java puede usar la red. Acepta para que tus amigos puedan
entrar.

## Seguridad

- Solo entran cuentas reales de Minecraft que estén en la lista blanca. Si
  desactivas alguna de las dos cosas, el panel te lo recuerda.
- Micra solo abre el puerto del juego. RCON viene apagado; si lo activas, le
  pone una contraseña larga y su puerto nunca se abre a internet.
- Si una IP falla al entrar muchas veces seguidas, Micra la bloquea un rato.
  También puedes limitar cuántos jugadores entran desde una misma IP.
- El servidor se ejecuta con permisos mínimos. En Linux solo puede escribir
  en su propia carpeta, así que un mod malicioso no llega a tus archivos
  personales. En Windows no puede usar el portapapeles ni cambiar la
  configuración del sistema, y en sus ajustes puedes impedir también que
  escriba fuera de su carpeta (experimental).
- Antes de cada arranque, Micra revisa los mods y plugins y no enciende el
  servidor si encuentra malware conocido.
- Todo se descarga por HTTPS. Cuando la fuente publica la huella del archivo
  (Mojang, Modrinth, CurseForge, Java, Forge y NeoForge), Micra la comprueba
  antes de usarlo.
- La clave de CurseForge, el webhook de Discord y la cuenta de playit.gg se
  guardan en el llavero del sistema. Si tu escritorio no tiene llavero, van a
  un archivo que solo puede leer tu usuario.

## Requisitos

- Windows 10 u 11 de 64 bits, o Linux de 64 bits (Ubuntu 22.04 o posterior,
  Debian 12 o posterior, Fedora, openSUSE o cualquier otra con el AppImage).
- Memoria libre para el servidor: unos 4 GB para Vanilla y entre 6 y 8 GB
  para modpacks, además de la que use tu PC.
- Conexión a internet para descargar Minecraft, Java y los modpacks.

## Dónde se guardan tus servidores

Los servidores, los mundos y las versiones de Java que descarga Micra están
en una carpeta de tu usuario. Puedes abrirla desde **Ajustes > Abrir
carpeta**:

- Windows: `%APPDATA%\Micra\Micra\data\`
- Linux: `~/.local/share/micra/`

Desinstalar Micra no borra esta carpeta.

---

¿Quieres compilar Micra o colaborar? Las instrucciones están en
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

Puedes usar Micra gratis. El código tiene todos los derechos reservados: se
puede ver, pero no copiar ni reutilizar sin permiso. Los detalles están en
[LICENSE](LICENSE).

Micra no es un producto oficial de Minecraft. No está aprobado por Mojang ni
por Microsoft, ni asociado con ellos.
