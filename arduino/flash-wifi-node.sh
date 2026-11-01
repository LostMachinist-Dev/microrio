#!/usr/bin/env bash
# Build and upload the WiFi node with credentials that never touch the project.
#
#   arduino/flash-wifi-node.sh [port] [wifi file]
#
# The wifi file is two lines: network name, then password. It defaults to
# ~/.config/microrio/wifi.txt. The sketch is copied to a temporary folder, the
# credentials are written beside it as wifi_secrets.h, and the folder is
# deleted afterwards whatever happens.
set -euo pipefail
cd "$(dirname "$0")/.."

PORT="${1:-/dev/ttyUSB0}"
WIFI="${2:-$HOME/.config/microrio/wifi.txt}"
FQBN="${FQBN:-esp32:esp32:esp32-poe-iso}"

[ -r "$WIFI" ] || { echo "no wifi file at $WIFI (line 1 name, line 2 password)"; exit 1; }

# Windows line endings would put a carriage return inside the password, and a
# file written by hand often says "Name: ..." and "Password: ...". Take the
# value either way.
field() {
    sed -n "${1}p" "$WIFI" | tr -d '\r' |
        sed -E 's/^[[:space:]]*(ssid|name|network|wifi|pass|password|pw|key)[[:space:]]*[:=][[:space:]]?//I'
}
SSID="$(field 1)"
PASS="$(field 2)"
[ -n "$SSID" ] || { echo "$WIFI: first line (network name) is empty"; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir "$TMP/Esp32WifiNode"
cp arduino/MicroRIO/examples/Esp32WifiNode/Esp32WifiNode.ino "$TMP/Esp32WifiNode/"
chmod 700 "$TMP"

# C string literals: escape backslashes and quotes.
esc() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'; }
{
    echo "#define WIFI_SSID \"$(esc "$SSID")\""
    echo "#define WIFI_PASS \"$(esc "$PASS")\""
} > "$TMP/Esp32WifiNode/wifi_secrets.h"

arduino-cli compile -b "$FQBN" --library arduino/MicroRIO -u -p "$PORT" "$TMP/Esp32WifiNode"
