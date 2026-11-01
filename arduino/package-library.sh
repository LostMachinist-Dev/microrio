#!/bin/bash
# Build the Arduino library zip, laid out so it extracts straight into a
# sketchbook libraries folder.
#
#   unzip MicroRIO-x.y.z.zip -d ~/Arduino/libraries/
#
# Regenerate the protocol header first; it is generated from spec/protocol.toml
# and must not drift from the Rust side.
set -e
cd "$(dirname "$0")/.."
cargo xtask gen-header

VER=$(grep '^version=' arduino/MicroRIO/library.properties | cut -d= -f2)
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT

mkdir -p "$STAGE/MicroRIO/src" "$STAGE/MicroRIO/examples"
cp arduino/MicroRIO/src/MicroRIO.h arduino/MicroRIO/src/MicroRIO_protocol.h "$STAGE/MicroRIO/src/"
cp arduino/MicroRIO/library.properties arduino/MicroRIO/keywords.txt arduino/MicroRIO/README.md "$STAGE/MicroRIO/"

# Arduino needs each example in a folder named after its .ino
for d in arduino/MicroRIO/examples/*/; do
  name=$(basename "$d")
  mkdir -p "$STAGE/MicroRIO/examples/$name"
  cp "$d/$name.ino" "$STAGE/MicroRIO/examples/$name/"
done

OUT=~/deliverables/microrio/MicroRIO-$VER.zip
rm -f "$OUT"
(cd "$STAGE" && zip -rq "$OUT" MicroRIO)
echo "wrote $OUT"
unzip -l "$OUT" | tail -3
