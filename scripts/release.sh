#!/bin/zsh
# Builds the downloadable release into dist/ (RULES.md I1, I4):
#   dist/Wisp-<version>.dmg   Wisp.app for Apple silicon and Intel; drag to Applications, open it
#   dist/Wisp-<version>.exe   Windows: double-click to install
# usage: scripts/release.sh
set -euo pipefail
cd "${0:A:h}/.."
source ~/.cargo/env

SIGN_ID=${WISP_SIGN_ID:-"Wisp Dev"}
LABEL="fun.xfun.wisp"
version=$(awk -F'"' '/^version/ {print $2; exit}' Cargo.toml)

scripts/verify.sh

rm -rf dist && mkdir -p dist/stage
# the core, for both Mac architectures
for t in aarch64-apple-darwin x86_64-apple-darwin; do cargo build --release --quiet --target $t; done
lipo -create -output dist/wisp target/aarch64-apple-darwin/release/wisp target/x86_64-apple-darwin/release/wisp

# the settings window, for both too
app=dist/stage/Wisp.app
mkdir -p $app/Contents/MacOS $app/Contents/Resources $app/Contents/Helpers
sed -e "s#<key>CFBundleShortVersionString</key><string>[^<]*</string>#<key>CFBundleShortVersionString</key><string>$version</string>#" ui/mac/Info.plist > $app/Contents/Info.plist
cp ui/mac/AppIcon.icns ui/mac/fonts/* $app/Contents/Resources/
for arch in arm64 x86_64; do
  swiftc -O -parse-as-library -target $arch-apple-macos14 ui/mac/WispSettings.swift -o dist/Wisp-$arch
done
lipo -create -output $app/Contents/MacOS/Wisp dist/Wisp-arm64 dist/Wisp-x86_64
for arch in arm64 x86_64; do
  swiftc -O -parse-as-library -target $arch-apple-macos14 ui/mac/WispMenuBar.swift -o dist/WispMenu-$arch
done
lipo -create -output $app/Contents/Helpers/WispMenu dist/WispMenu-arm64 dist/WispMenu-x86_64
mv dist/wisp $app/Contents/Helpers/wisp
codesign --force -s "$SIGN_ID" -i "$LABEL" $app/Contents/Helpers/wisp
codesign --force -s "$SIGN_ID" -i "$LABEL.menu" $app/Contents/Helpers/WispMenu
codesign --force -s "$SIGN_ID" $app
codesign --verify --deep --strict $app
for f in $app/Contents/MacOS/Wisp $app/Contents/Helpers/wisp $app/Contents/Helpers/WispMenu; do print "$f: $(lipo -archs $f)"; done

ln -s /Applications dist/stage/Applications
hdiutil create -quiet -volname "Wisp $version" -srcfolder dist/stage -fs HFS+ -format UDZO dist/Wisp-$version.dmg

cargo build --release --quiet --target x86_64-pc-windows-gnu
cp target/x86_64-pc-windows-gnu/release/wisp.exe dist/Wisp-$version.exe

# a release never carries the developer bridge (RULES.md X1)
for f in $app/Contents/Helpers/wisp dist/Wisp-$version.exe; do
  # byte-wise (LC_ALL=C, -a): in a UTF-8 locale grep may not read a binary at all
  if LC_ALL=C grep -qa "bridge-exec" $f || LC_ALL=C grep -qa "开发者通道" $f; then print -u2 "release: $f contains the developer bridge"; exit 1; fi
done
print "release: no developer bridge in the binaries"

rm -rf dist/stage dist/Wisp-arm64 dist/Wisp-x86_64 dist/WispMenu-arm64 dist/WispMenu-x86_64
ls -la dist
