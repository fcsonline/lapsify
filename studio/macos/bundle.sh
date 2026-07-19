#!/bin/sh
# Assemble Lapsify Studio.app from the release binary.
# Usage: studio/macos/bundle.sh   (from the repo root; builds release first)
set -e

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APP="$ROOT/dist/Lapsify Studio.app"

cargo build --release -p lapsify-studio

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$ROOT/target/release/lapsify-studio" "$APP/Contents/MacOS/"
cp "$ROOT/studio/macos/Info.plist" "$APP/Contents/"

# Build the .icns from the mark.
ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET"
SRC="$ROOT/studio/assets/icon-256.png"
for size in 16 32 64 128 256; do
  sips -z $size $size "$SRC" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double "$SRC" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

echo "Bundled: $APP"
