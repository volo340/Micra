<p align="center"><img src="app-icon.svg" width="96" alt="Logo de Micra" /></p>

# Micra

Convierte tu PC (Windows o Linux) en un servidor de Minecraft sin escribir
código ni configurar el router: modpacks de Modrinth y CurseForge, Java
gestionado automáticamente, asistente de RAM/CPU y panel en tiempo real.

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
  descarga y lo deja listo para encender.
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
  consola con colores, el uso de CPU y memoria y quién está conectado. Puedes
  escribir comandos, dar permisos de operador o expulsar a un jugador sin
  abrir ningún archivo.
- **Varios servidores.** Crea todos los que quieras y cambia entre ellos
  desde la barra lateral.
- **Cierra sin miedo.** Al salir de Micra, los servidores se apagan de forma
  ordenada y los mundos se guardan.

## Jugar con tus amigos

1. Enciende el servidor desde su panel.
2. Copia la dirección que aparece en **Dirección para conectarse** (por
   ejemplo, `192.168.1.42:25565`).
3. En Minecraft, entra en **Multijugador**, pulsa **Añadir servidor** y pega
   esa dirección.

Por ahora pueden entrar quienes estén conectados a tu misma red (Wi-Fi o
cable). Jugar por internet sin tocar el router (apertura automática de
puertos y túnel) está en camino.

La primera vez que enciendas un servidor en Windows, el sistema puede
preguntar si Java puede usar la red. Acepta para que tus amigos puedan
entrar.

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

Micra no es un producto oficial de Minecraft. No está aprobado por Mojang ni
por Microsoft, ni asociado con ellos.
