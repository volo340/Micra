#!/usr/bin/env bash
#
# Instalador de Micra para Linux.
#
#   bash micra-install.sh               instala o actualiza Micra
#   bash micra-install.sh --appimage    instala solo para tu usuario, sin sudo
#   bash micra-install.sh --uninstall   desinstala (tus servidores y mundos se conservan)
#
# Qué hace:
#   - Si junto al script hay un paquete de Micra (.deb, .rpm o .AppImage), usa
#     ese. Si no, descarga la última versión publicada en GitHub.
#   - Debian, Ubuntu, Mint, Pop!_OS...: instala el .deb con apt, que añade por
#     su cuenta las librerías del sistema que falten (WebKitGTK, GTK...).
#   - Fedora y openSUSE: lo mismo con el .rpm (dnf o zypper).
#   - Cualquier otra distribución, o si no hay sudo: instala el AppImage en tu
#     carpeta de usuario y crea el acceso directo en el menú de aplicaciones.
#
# No hace falta instalar Java, Node ni Rust: Micra descarga Java ella sola
# cuando creas un servidor, y Node y Rust solo se usan para compilar la app.

set -euo pipefail

REPO="volo340/Micra"
# Al publicar una versión, el flujo de GitHub Actions fija aquí su etiqueta y
# la huella SHA-256 de cada paquete ("huella  nombre", una por línea).
DEFAULT_VERSION="latest"
PACKAGE_SHA256=""
MICRA_VERSION="${MICRA_VERSION:-$DEFAULT_VERSION}"
API_BASE="${MICRA_API_BASE:-https://api.github.com}"

SHARE_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
APP_DIR="$SHARE_DIR/micra-app"
DATA_DIR="$SHARE_DIR/micra"
DESKTOP_DIR="$SHARE_DIR/applications"
BIN_DIR="$HOME/.local/bin"
LAUNCHER_MARK="# micra-install.sh"

say()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mAviso:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31mError:\033[0m %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

usage() {
  cat <<'USAGE'
Uso: bash micra-install.sh [opciones]

  (sin opciones)    instala o actualiza Micra
  --appimage        instala solo para tu usuario, sin sudo
  --from CARPETA    usa los paquetes de esa carpeta en vez de descargarlos
  --uninstall       desinstala Micra (tus servidores y mundos se conservan)
USAGE
}

# --- Utilidades ------------------------------------------------------------

SUDO=""
if [ "$(id -u)" -ne 0 ]; then
  if have sudo; then SUDO="sudo"; elif have doas; then SUDO="doas"; fi
fi
can_root() { [ "$(id -u)" -eq 0 ] || [ -n "$SUDO" ]; }

fetch() { # fetch URL DESTINO [ACCEPT]
  local url=$1 dest=$2 accept=${3:-application/octet-stream}
  # Solo HTTPS, también en las redirecciones (GitHub redirige a su CDN).
  case $url in
    https://*) ;;
    *) die "dirección no segura, no descargo: $url" ;;
  esac
  if have curl; then
    curl -fL --proto '=https' --proto-redir '=https' --tlsv1.2 --retry 3 \
      --progress-bar -H "Accept: $accept" -o "$dest" "$url"
  elif have wget; then
    local https_only=()
    if wget --help 2>&1 | grep -q -- '--https-only'; then https_only=(--https-only); fi
    wget -q --show-progress ${https_only[@]+"${https_only[@]}"} --header="Accept: $accept" -O "$dest" "$url"
  else
    die "necesitas curl o wget para descargar Micra"
  fi
}

# expected_sha256 NOMBRE [api]: huella con la que se publicó ese paquete. Primero
# la que lleva este script; con «api», si no la lleva, la que calculó GitHub al
# subirlo.
expected_sha256() {
  local name=$1 sum
  sum=$(printf '%s\n' "$PACKAGE_SHA256" |
    awk -v n="$name" '$2 == n || $2 == "*" n { print $1; exit }')
  if [ -z "$sum" ] && [ "${2:-}" = api ] && [ -f "$TMP/release.json" ] && have python3; then
    sum=$(python3 -I -c '
import json, sys
with open(sys.argv[1], encoding="utf-8") as f:
    release = json.load(f)
for asset in release.get("assets") or []:
    digest = asset.get("digest") or ""
    if asset.get("name") == sys.argv[2] and digest.startswith("sha256:"):
        print(digest[len("sha256:"):])
        break
' "$TMP/release.json" "$name" 2>/dev/null || true)
  fi
  case $sum in
    "" | *[!0-9a-fA-F]*) ;;
    *) if [ ${#sum} -eq 64 ]; then printf '%s\n' "$sum" | tr 'A-F' 'a-f'; fi ;;
  esac
}

# check_sha256 ARCHIVO NOMBRE [api]: para la instalación si el paquete no es
# exactamente el publicado (descarga dañada o alterada).
check_sha256() {
  local file=$1 name=$2 expected actual
  expected=$(expected_sha256 "$name" "${3:-}")
  if [ -z "$expected" ]; then
    if [ "${3:-}" = api ]; then
      warn "no hay huella publicada de $name; no puedo comprobar que la descarga esté intacta"
    fi
    return 0
  fi
  if ! have sha256sum; then
    warn "falta sha256sum; no puedo comprobar que $name esté intacto"
    return 0
  fi
  actual=$(sha256sum "$file" | cut -d ' ' -f 1)
  if [ "$actual" != "$expected" ]; then
    die"$name no coincide con el paquete publicado (puede estar dañado o alterado), así que no lo instalo. Vuelve a descargarlo de https://github.com/$REPO/releases"
  fi
  say "$name comprobado (SHA-256)."
}

detect_arch() {
  case "$(uname -m)" in
    x86_64 | amd64) DEB_ARCH=amd64 RPM_ARCH=x86_64 APPIMAGE_ARCH=amd64 ;;
    aarch64 | arm64) DEB_ARCH=arm64 RPM_ARCH=aarch64 APPIMAGE_ARCH=aarch64 ;;
    *) die "arquitectura no soportada: $(uname -m)" ;;
  esac
}

# Carpeta donde buscar paquetes ya descargados (la del propio script).
SOURCE_DIR=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
  SOURCE_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
fi

ASSETS_LOADED=0
ASSETS=""
load_assets() {
  [ "$ASSETS_LOADED" -eq 1 ] && return 0
  local url
  if [ "$MICRA_VERSION" = latest ]; then
    url="$API_BASE/repos/$REPO/releases/latest"
  else
    url="$API_BASE/repos/$REPO/releases/tags/$MICRA_VERSION"
  fi
  if ! fetch "$url" "$TMP/release.json" application/vnd.github+json 2>/dev/null; then
    die "no pude consultar las versiones de Micra en GitHub. Si el repositorio es privado, descarga los instaladores a mano y ejecuta este script en la misma carpeta."
  fi
  # Sin depender de jq: una clave por línea y nos quedamos con las URLs, solo
  # las de las versiones de este repositorio.
  ASSETS=$(tr ',' '\n' <"$TMP/release.json" |
    sed -n 's/.*"browser_download_url"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' |
    awk -v p="https://github.com/$REPO/releases/download/" 'index($0, p) == 1')
  ASSETS_LOADED=1
}

# obtain REGEX: deja en ASSET_FILE la ruta a un paquete que encaje, local o descargado.
ASSET_FILE=""
obtain() {
  local pattern=$1 url
  ASSET_FILE=""
  if [ -n "$SOURCE_DIR" ]; then
    ASSET_FILE=$(find "$SOURCE_DIR" -maxdepth 1 -type f -iname 'micra*' 2>/dev/null |
      grep -E "$pattern" | sort -V | tail -n 1 || true)
    if [ -n "$ASSET_FILE" ]; then
      say "Usando $(basename "$ASSET_FILE")"
      # Si es uno de los paquetes de esta versión, se comprueba que esté intacto.
      check_sha256 "$ASSET_FILE" "$(basename "$ASSET_FILE")"
      return 0
    fi
  fi
  load_assets
  url=$(printf '%s\n' "$ASSETS" | grep -E "$pattern" | head -n 1 || true)
  [ -n "$url" ] || return 1
  ASSET_FILE="$TMP/$(basename "$url")"
  say "Descargando $(basename "$url")..."
  fetch "$url" "$ASSET_FILE" || return 1
  check_sha256 "$ASSET_FILE" "$(basename "$url")" api
}

# --- Instalación -------------------------------------------------------------

install_deb() {
  obtain "_${DEB_ARCH}\.deb$" || return 1
  say "Instalando Micra con apt (añadirá las librerías del sistema que falten)..."
  # apt lee el paquete con un usuario sin privilegios: debe poder leerlo.
  chmod 644 "$ASSET_FILE" 2>/dev/null || true
  chmod 755 "$TMP" 2>/dev/null || true
  $SUDO apt-get update -qq || warn "apt-get update falló; sigo con la lista de paquetes actual"
  $SUDO apt-get install -y "$ASSET_FILE" || return 1
}

install_rpm() {
  obtain "\.${RPM_ARCH}\.rpm$" || return 1
  say "Instalando Micra (se añadirán las librerías del sistema que falten)..."
  if have dnf; then
    $SUDO dnf install -y "$ASSET_FILE" || return 1
  else
    $SUDO zypper --non-interactive install --allow-unsigned-rpm "$ASSET_FILE" || return 1
  fi
}

install_appimage() {
  obtain "_${APPIMAGE_ARCH}\.AppImage$" ||
    die "no encontré un AppImage de Micra para $(uname -m)"
  say "Instalando Micra en $APP_DIR..."
  mkdir -p "$APP_DIR" "$BIN_DIR" "$DESKTOP_DIR"
  install -m 755 "$ASSET_FILE" "$APP_DIR/Micra.AppImage"

  # Icono: se extrae del propio AppImage (no necesita FUSE).
  local icon=""
  if (cd "$TMP" && "$APP_DIR/Micra.AppImage" --appimage-extract '*.png' >/dev/null 2>&1); then
    icon=$(find "$TMP/squashfs-root" -name '*.png' -type f -exec ls -S {} + 2>/dev/null | head -n 1 || true)
  fi
  if [ -n "$icon" ]; then
    install -m 644 "$icon" "$APP_DIR/micra.png"
  fi

  cat >"$BIN_DIR/micra" <<EOF
#!/bin/sh
$LAUNCHER_MARK: lanzador del AppImage de Micra.
# Sin FUSE 2 el AppImage no se puede montar, así que se extrae al vuelo.
if ! { ldconfig -p 2>/dev/null || /sbin/ldconfig -p 2>/dev/null; } | grep -q 'libfuse\.so\.2'; then
  export APPIMAGE_EXTRACT_AND_RUN=1
fi
exec "$APP_DIR/Micra.AppImage" "\$@"
EOF
  chmod 755 "$BIN_DIR/micra"

  cat >"$DESKTOP_DIR/micra.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Micra
GenericName=Servidor de Minecraft
Comment=Convierte tu PC en un servidor de Minecraft
Exec="$BIN_DIR/micra"
Icon=$APP_DIR/micra.png
Terminal=false
Categories=Game;
StartupWMClass=micra
EOF
  if have update-desktop-database; then
    update-desktop-database "$DESKTOP_DIR" >/dev/null 2>&1 || true
  fi
}

uninstall() {
  local removed=0
  if have dpkg && dpkg -s micra >/dev/null 2>&1; then
    say "Desinstalando el paquete .deb..."
    $SUDO apt-get remove -y micra
    removed=1
  fi
  if have rpm && rpm -q micra >/dev/null 2>&1; then
    say "Desinstalando el paquete .rpm..."
    if have dnf; then
      $SUDO dnf remove -y micra
    elif have zypper; then
      $SUDO zypper --non-interactive remove micra
    else
      $SUDO rpm -e micra
    fi
    removed=1
  fi
  if [ -e "$APP_DIR/Micra.AppImage" ]; then
    say "Borrando el AppImage..."
    rm -rf "$APP_DIR" "$DESKTOP_DIR/micra.desktop"
    if grep -q "$LAUNCHER_MARK" "$BIN_DIR/micra" 2>/dev/null; then
      rm -f "$BIN_DIR/micra"
    fi
    removed=1
  fi
  if [ "$removed" -eq 0 ]; then
    say "Micra no estaba instalada."
  else
    say "Micra se ha desinstalado."
  fi
  if [ -d "$DATA_DIR" ]; then
    say "Tus servidores y mundos siguen en $DATA_DIR (bórralo a mano si ya no los quieres)."
  fi
}

finish() {
  say "¡Listo! Abre Micra desde el menú de aplicaciones."
  if [ "$1" = appimage ]; then
    case ":$PATH:" in
      *":$BIN_DIR:"*) say "También puedes escribir «micra» en una terminal." ;;
    esac
  fi
}

main() {
  local mode=auto
  while [ $# -gt 0 ]; do
    case $1 in
      --appimage) mode=appimage ;;
      --uninstall) mode=uninstall ;;
      --from)
        [ $# -ge 2 ] || die "--from necesita una carpeta"
        SOURCE_DIR=$(cd "$2" && pwd) || die "no existe la carpeta $2"
        shift
        ;;
      -h | --help)
        usage
        return 0
        ;;
      *) die "opción desconocida: $1 (usa --help)" ;;
    esac
    shift
  done

  [ "$(uname -s)" = Linux ] || die "este instalador es para Linux; en Windows usa el instalador .exe"
  if [ "$mode" = uninstall ]; then
    uninstall
    return 0
  fi

  detect_arch
  TMP=$(mktemp -d)
  trap 'rm -rf "$TMP"' EXIT

  if [ "$mode" = auto ] && can_root; then
    if have apt-get && have dpkg; then
      if install_deb; then finish deb; return 0; fi
      warn "no se pudo instalar el .deb; pruebo con el AppImage"
    elif have dnf || have zypper; then
      if install_rpm; then finish rpm; return 0; fi
      warn "no se pudo instalar el .rpm; pruebo con el AppImage"
    fi
  fi
  install_appimage
  finish appimage
}

main "$@"
