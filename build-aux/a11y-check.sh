#!/bin/sh
# Vérification d'accessibilité (voir a11y-check.py). Usage :
#   build-aux/a11y-check.sh chemin/vers/tsubame
# Les tailles en palier mobile dépendent de la police : celle de GNOME
# (Adwaita Sans, proche d'Inter) ; avec une police plus large comme DejaVu
# Sans, le contenu peut dépasser 360 px.
# Code 77 (test ignoré par Meson) si un outil manque : xvfb-run, xdotool,
# dbus-run-session, at-spi-bus-launcher, Python avec pyatspi.
set -u
BIN=$(realpath "$1")
HERE=$(dirname "$(realpath "$0")")
for tool in xvfb-run xdotool dbus-run-session; do
    command -v "$tool" >/dev/null 2>&1 || { echo "$tool manque : test ignoré"; exit 77; }
done
LAUNCHER=""
for path in /usr/libexec/at-spi-bus-launcher /usr/lib/at-spi2-core/at-spi-bus-launcher; do
    [ -x "$path" ] && LAUNCHER=$path
done
[ -n "$LAUNCHER" ] || { echo "at-spi-bus-launcher manque : test ignoré"; exit 77; }
PYTHON=""
for py in python3 python3.12 python3.13 python3.11; do
    if command -v "$py" >/dev/null 2>&1 && "$py" -c "import pyatspi" >/dev/null 2>&1; then
        PYTHON=$py; break
    fi
done
[ -n "$PYTHON" ] || { echo "pyatspi manque : test ignoré"; exit 77; }

CONFIG=$(mktemp -d)
trap 'rm -rf "$CONFIG"' EXIT
# Police d'interface fixée, pour des mesures identiques partout : Inter,
# base d'Adwaita Sans, la police de GNOME. Sans bureau, GTK demanderait
# Cantarell, que fontconfig remplace selon la machine (DejaVu Sans, plus
# large, sur les runners GitHub).
mkdir -p "$CONFIG/gtk-4.0"
printf '[Settings]\ngtk-font-name=Inter 11\n' > "$CONFIG/gtk-4.0/settings.ini"
export LAUNCHER BIN PYTHON HERE CONFIG
exec dbus-run-session -- xvfb-run -a -s "-screen 0 1400x900x24" sh -c '
    "$LAUNCHER" --launch-immediately &
    sleep 1.5
    export GDK_BACKEND=x11 GSK_RENDERER=cairo GTK_A11Y=atspi \
        GSETTINGS_BACKEND=keyfile XDG_CONFIG_HOME="$CONFIG" XDG_DATA_HOME="$CONFIG" \
        LANG=C.UTF-8 LANGUAGE=
    "$BIN" >/dev/null 2>&1 &
    APP=$!
    "$PYTHON" "$HERE/a11y-check.py" 2>&1
    STATUS=$?
    kill $APP 2>/dev/null
    exit $STATUS
' 2>/dev/null
