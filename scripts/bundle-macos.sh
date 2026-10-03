#!/bin/bash
set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
    echo "This script requires macOS." >&2
    exit 1
fi
profile=debug
build_args=()
case "${1:-}" in
    '') ;;
    --release) profile=release; build_args+=(--release) ;;
    *) echo "Usage: $0 [--release]" >&2; exit 1 ;;
esac
if [[ $# -gt 1 ]]; then
    echo "Usage: $0 [--release]" >&2
    exit 1
fi
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
cargo build --locked --bin inkstone --target-dir "$root/target" ${build_args[@]+"${build_args[@]}"}
bundle="$root/target/$profile/墨砚.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
iconset="$root/target/$profile/Inkstone.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" assets/branding/inkstone-icon.png \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double_size=$((size * 2))
    sips -z "$double_size" "$double_size" assets/branding/inkstone-icon.png \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$bundle/Contents/Resources/Inkstone.icns"
cp packaging/macos/Info.plist "$bundle/Contents/Info.plist"
cp "target/$profile/inkstone" "$bundle/Contents/MacOS/inkstone"
codesign --force --sign - "$bundle"
printf 'Built %s\n' "$bundle"
