#!/usr/bin/env bash
set -euo pipefail

APP_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SOURCE="$APP_ROOT/src-tauri/icons/ripple-live-source.png"
ICONS="$APP_ROOT/src-tauri/icons"
ANDROID="$APP_ROOT/src-tauri/gen/android/app/src/main/res"
APPLE="$APP_ROOT/src-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset"
ICONSET="$(mktemp -d -t ripple-live-iconset).iconset"

resize() {
  /usr/bin/sips -z "$1" "$1" "$SOURCE" --out "$2" >/dev/null
}

resize 32 "$ICONS/32x32.png"
resize 64 "$ICONS/64x64.png"
resize 128 "$ICONS/128x128.png"
resize 256 "$ICONS/128x128@2x.png"
resize 512 "$ICONS/icon.png"

for density_size in mdpi:48 hdpi:72 xhdpi:96 xxhdpi:144 xxxhdpi:192; do
  density=${density_size%%:*}
  size=${density_size##*:}
  resize "$size" "$ANDROID/mipmap-$density/ic_launcher.png"
  resize "$size" "$ANDROID/mipmap-$density/ic_launcher_round.png"
done

for density_size in mdpi:108 hdpi:162 xhdpi:216 xxhdpi:324 xxxhdpi:432; do
  density=${density_size%%:*}
  size=${density_size##*:}
  resize "$size" "$ANDROID/mipmap-$density/ic_launcher_foreground.png"
done

while IFS=: read -r file size; do
  resize "$size" "$APPLE/$file"
done <<'APPLE_ICONS'
AppIcon-20x20@1x.png:20
AppIcon-20x20@2x-1.png:40
AppIcon-20x20@2x.png:40
AppIcon-20x20@3x.png:60
AppIcon-29x29@1x.png:29
AppIcon-29x29@2x-1.png:58
AppIcon-29x29@2x.png:58
AppIcon-29x29@3x.png:87
AppIcon-40x40@1x.png:40
AppIcon-40x40@2x-1.png:80
AppIcon-40x40@2x.png:80
AppIcon-40x40@3x.png:120
AppIcon-60x60@2x.png:120
AppIcon-60x60@3x.png:180
AppIcon-76x76@1x.png:76
AppIcon-76x76@2x.png:152
AppIcon-83.5x83.5@2x.png:167
AppIcon-512@2x.png:1024
APPLE_ICONS

mkdir -p "$ICONSET"
resize 16 "$ICONSET/icon_16x16.png"
resize 32 "$ICONSET/icon_16x16@2x.png"
resize 32 "$ICONSET/icon_32x32.png"
resize 64 "$ICONSET/icon_32x32@2x.png"
resize 128 "$ICONSET/icon_128x128.png"
resize 256 "$ICONSET/icon_128x128@2x.png"
resize 256 "$ICONSET/icon_256x256.png"
resize 512 "$ICONSET/icon_256x256@2x.png"
resize 512 "$ICONSET/icon_512x512.png"
resize 1024 "$ICONSET/icon_512x512@2x.png"
/usr/bin/iconutil -c icns "$ICONSET" -o "$ICONS/icon.icns"
