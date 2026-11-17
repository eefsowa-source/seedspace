#!/usr/bin/env bash
# Builds dist/Seedspace-<version>.pkg installing the VST3 and CLAP into /Library/Audio/Plug-Ins.
# shortcut: ad-hoc signed and not notarized (no paid Apple account), so other Macs show a
# Gatekeeper warning; arm64 only. Upgrade with a Developer ID + notarytool, and a universal build.
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cargo xtask bundle seedspace --release

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/root/Library/Audio/Plug-Ins/VST3" "$stage/root/Library/Audio/Plug-Ins/CLAP" \
    "$stage/root/Library/Audio/Plug-Ins/Seedspace"
ditto --norsrc --noextattr --noqtn target/bundled/Seedspace.vst3 "$stage/root/Library/Audio/Plug-Ins/VST3/Seedspace.vst3"
ditto --norsrc --noextattr --noqtn target/bundled/Seedspace.clap "$stage/root/Library/Audio/Plug-Ins/CLAP/Seedspace.clap"
# Keep the license outside the signed bundles, or their signatures break.
ditto --norsrc --noextattr --noqtn LICENSE "$stage/root/Library/Audio/Plug-Ins/Seedspace/LICENSE"

# Without this, Installer "relocates" onto any copy with the same bundle id, e.g. target/bundled
pkgbuild --analyze --root "$stage/root" "$stage/components.plist"
# One entry per bundle -- counted, so adding a format later cannot silently skip the fix
components=$(/usr/libexec/PlistBuddy -c "Print" "$stage/components.plist" | grep -c RootRelativeBundlePath)
for i in $(seq 0 $((components - 1))); do
    /usr/libexec/PlistBuddy -c "Add :$i:BundleIsRelocatable bool false" "$stage/components.plist"
done

mkdir -p dist
pkgbuild --root "$stage/root" --component-plist "$stage/components.plist" \
    --identifier com.sungha.seedspace --version "$version" --install-location / \
    "dist/Seedspace-$version.pkg"
