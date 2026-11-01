#!/usr/bin/env bash
# Build everything that gets handed over, into ~/deliverables/microrio/.
#
#   packaging/release.sh
#
# Produces:
#   microrio-station                     Linux binary
#   microrio-station_<ver>_amd64.deb     Linux installer
#   microrio-station.exe                 Windows binary
#   MicroRIO-Setup-<ver>.exe             Windows installer (needs makensis)
#   MicroRIO-<ver>.zip                   Arduino library
#   the docs
#
# Each step says what it wrote. A missing cross-compiler or makensis skips
# that one step with a note rather than failing the run, so a Linux-only box
# still gets a complete Linux release.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=~/deliverables/microrio
VER=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$OUT"

say() { printf '  %s\n' "$*"; }
step() { printf '\n== %s\n' "$*"; }

# Forces the version stamp to be recomputed, so an installer built from an
# uncommitted tree says so.
MICRORIO_STAMP=$(git describe --always --dirty=+ 2>/dev/null || echo none)
export MICRORIO_STAMP

step "MicroRIO $VER  ($MICRORIO_STAMP)"

# ---------------------------------------------------------------- Linux
step "Linux binary"
cargo build --release -p microrio-station
install -m755 target/release/microrio-station "$OUT/microrio-station"
say "$OUT/microrio-station"

step "Linux installer (.deb)"
DEB="$STAGE/deb"
mkdir -p "$DEB/DEBIAN" \
         "$DEB/usr/bin" \
         "$DEB/usr/share/applications" \
         "$DEB/usr/share/icons/hicolor/scalable/apps" \
         "$DEB/usr/share/doc/microrio-station"
install -m755 target/release/microrio-station "$DEB/usr/bin/"
install -m644 packaging/microrio-station.desktop "$DEB/usr/share/applications/"
install -m644 packaging/microrio-station.svg \
        "$DEB/usr/share/icons/hicolor/scalable/apps/"
install -m644 RUNNING.md DESIGN.md "$DEB/usr/share/doc/microrio-station/"
install -m644 LICENSE "$DEB/usr/share/doc/microrio-station/copyright"
# The matching library, so a board can be set up from the same install.
mkdir -p "$DEB/usr/share/microrio"
arduino/package-library.sh >/dev/null
install -m644 "$(ls -t "$OUT"/MicroRIO-*.zip | head -1)" "$DEB/usr/share/microrio/"
SIZE=$(du -ks "$DEB" | cut -f1)
cat > "$DEB/DEBIAN/control" <<EOF
Package: microrio-station
Version: $VER
Section: electronics
Priority: optional
Architecture: amd64
Maintainer: MicroRIO <microrio@localhost>
Installed-Size: $SIZE
Depends: libc6, libgcc-s1, libudev1
Recommends: libgl1, libx11-6, libxkbcommon0, libwayland-client0
Description: MicroRIO driver station
 Live telemetry, control, charting and logging for microcontroller nodes
 over UDP. Channels are named once at connect and carried as numbered slots
 after that, so a 2 KB board spends its memory on the sketch rather than on
 channel names.
 .
 Boards speak the protocol through the MicroRIO Arduino library.
EOF
if command -v dpkg-deb >/dev/null; then
    dpkg-deb --build --root-owner-group -Zxz "$DEB" \
        "$OUT/microrio-station_${VER}_amd64.deb" >/dev/null
    say "$OUT/microrio-station_${VER}_amd64.deb"
else
    say "skipped: dpkg-deb is not installed"
fi

# -------------------------------------------------------------- Windows
step "Windows binary"
WIN=target/x86_64-pc-windows-gnu/release/microrio-station.exe
if cargo build --release -p microrio-station --target x86_64-pc-windows-gnu 2>"$STAGE/win.log"; then
    install -m755 "$WIN" "$OUT/microrio-station.exe"
    say "$OUT/microrio-station.exe"
else
    say "skipped: no Windows cross-compiler"
    say "  $(tail -1 "$STAGE/win.log")"
    WIN=
fi

step "Arduino library"
arduino/package-library.sh | sed 's/^/  /'
LIB=$(ls -t "$OUT"/MicroRIO-*.zip | head -1)
say "also inside the .deb, at /usr/share/microrio/"

step "Windows installer"
if [ -z "${WIN:-}" ]; then
    say "skipped: there is no .exe to install"
elif ! command -v makensis >/dev/null; then
    say "skipped: makensis is not installed (sudo apt install nsis)"
else
    SRC="$STAGE/win"
    mkdir -p "$SRC/arduino"
    install -m644 "$WIN" "$SRC/microrio-station.exe"
    install -m644 "$LIB" "$SRC/arduino/"
    # Markdown opens in Notepad on Windows, which wants CRLF.
    sed 's/$/\r/' RUNNING.md > "$SRC/RUNNING.md"
    sed 's/$/\r/' DESIGN.md  > "$SRC/MicroRIO-design.md"
    sed 's/$/\r/' LICENSE    > "$SRC/LICENSE.txt"
    # The icon the Start-menu shortcuts carry.
    if command -v convert >/dev/null; then
        convert -background none packaging/microrio-station.svg \
                -define icon:auto-resize=256,128,64,48,32,16 "$SRC/microrio.ico"
    else
        say "note: ImageMagick missing, using the fallback icon"
        cp packaging/microrio.ico "$SRC/microrio.ico"
    fi
    makensis -V2 \
        "-DVERSION=$VER" \
        "-DSRC=$SRC" \
        "-DOUT=$OUT/MicroRIO-Setup-$VER.exe" \
        packaging/microrio-station.nsi
    say "$OUT/MicroRIO-Setup-$VER.exe"
fi

step "Documents"
for f in RUNNING.md HARDWARE.md MicroRIO-origins.md; do
    [ -f "$f" ] || continue
    case "$f" in
        HARDWARE.md) install -m644 "$f" "$OUT/MicroRIO-hardware.md" ;;
        *)           install -m644 "$f" "$OUT/$f" ;;
    esac
done
install -m644 DESIGN.md "$OUT/MicroRIO-design.md"
say "RUNNING.md, MicroRIO-design.md, MicroRIO-hardware.md, MicroRIO-origins.md"

step "In $OUT"
ls -lh "$OUT" | tail -n +2 | awk '{printf "  %-34s %s\n", $9, $5}'
