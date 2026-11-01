#!/bin/sh
# Assemble Mojito.app from the release binary, hex build-app.sh's shape:
# build, lay out Contents/{MacOS,Resources}, copy the Info.plist, ad-hoc sign.
# Bundle before first launch so the TCC grant attaches to com.mojito.app and
# survives rebuilds.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export MACOSX_DEPLOYMENT_TARGET=15.0

cargo build --release --manifest-path "$root/Cargo.toml"

bundle="$root/target/app/Mojito.app"
executable="$bundle/Contents/MacOS/mojito"

rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$root/target/release/mojito" "$executable"
cp "$root/app/Info.plist" "$bundle/Contents/Info.plist"

codesign --force --sign - "$bundle"
codesign --verify --strict "$bundle"
echo "$bundle"
