#!/bin/zsh
# Builds the downloadable release into dist/ (RULES.md I1, I4, X1):
#   dist/Wisp-<version>.dmg       Wisp.app for Apple silicon and Intel; drag to Applications, open it
#   dist/Wisp-<version>.exe       Windows: double-click to install
#   dist/Wisp-<version>-dev.dmg   the same with the developer bridge, for those who want it
#   dist/Wisp-<version>-dev.exe   (both machines need the -dev build for the bridge to work)
# usage: scripts/release.sh
set -euo pipefail
cd "${0:A:h}/.."
source ~/.cargo/env

SIGN_ID=${WISP_SIGN_ID:-"Wisp Dev"}
LABEL="fun.xfun.wisp"
version=$(awk -F'"' '/^version/ {print $2; exit}' Cargo.toml)

scripts/verify.sh

rm -rf dist && mkdir -p dist

# byte-wise (LC_ALL=C, -a): in a UTF-8 locale grep may not read a binary at all
has_bridge() { LC_ALL=C grep -qa "bridge-exec" $1 || LC_ALL=C grep -qa "开发者通道" $1; }

# the settings window and the menu bar icon are the same in both sets: build them once
for arch in arm64 x86_64; do
  swiftc -O -parse-as-library -target $arch-apple-macos14 ui/mac/WispSettings.swift -o dist/Wisp-$arch
  swiftc -O -parse-as-library -target $arch-apple-macos14 ui/mac/WispMenuBar.swift -o dist/WispMenu-$arch
done

# one set: `release_set "" ""` is the plain build, `release_set bridge -dev` the developer one
release_set() {
  local feature=$1 suffix=$2
  local features=(${feature:+--features} ${feature})
  local stage=dist/stage$suffix app=dist/stage$suffix/Wisp.app
  rm -rf $stage && mkdir -p $app/Contents/MacOS $app/Contents/Resources $app/Contents/Helpers

  for t in aarch64-apple-darwin x86_64-apple-darwin; do cargo build --release --quiet --target $t $features; done
  lipo -create -output $app/Contents/Helpers/wisp target/aarch64-apple-darwin/release/wisp target/x86_64-apple-darwin/release/wisp
  lipo -create -output $app/Contents/MacOS/Wisp dist/Wisp-arm64 dist/Wisp-x86_64
  lipo -create -output $app/Contents/Helpers/WispMenu dist/WispMenu-arm64 dist/WispMenu-x86_64
  sed -e "s#<key>CFBundleShortVersionString</key><string>[^<]*</string>#<key>CFBundleShortVersionString</key><string>$version</string>#" ui/mac/Info.plist > $app/Contents/Info.plist
  cp ui/mac/AppIcon.icns ui/mac/fonts/* $app/Contents/Resources/
  codesign --force -s "$SIGN_ID" -i "$LABEL" $app/Contents/Helpers/wisp
  codesign --force -s "$SIGN_ID" -i "$LABEL.menu" $app/Contents/Helpers/WispMenu
  codesign --force -s "$SIGN_ID" $app
  codesign --verify --deep --strict $app
  for f in $app/Contents/MacOS/Wisp $app/Contents/Helpers/wisp $app/Contents/Helpers/WispMenu; do print "$f: $(lipo -archs $f)"; done

  cargo build --release --quiet --target x86_64-pc-windows-gnu $features
  cp target/x86_64-pc-windows-gnu/release/wisp.exe dist/Wisp-$version$suffix.exe

  # the plain set never carries the developer bridge; the -dev set always does (RULES.md X1)
  for f in $app/Contents/Helpers/wisp dist/Wisp-$version$suffix.exe; do
    if [[ -n $feature ]]; then
      has_bridge $f || { print -u2 "release: $f is the developer build but has no bridge"; exit 1; }
    else
      has_bridge $f && { print -u2 "release: $f contains the developer bridge"; exit 1; }
    fi
  done
  if [[ -n $feature ]]; then print "release: developer set checked: bridge present"; else print "release: plain set checked: no bridge"; fi

  ln -s /Applications $stage/Applications
  local name="Wisp $version"
  [[ -n $suffix ]] && name="Wisp $version Developer"
  hdiutil create -quiet -volname "$name" -srcfolder $stage -fs HFS+ -format UDZO dist/Wisp-$version$suffix.dmg
  rm -rf $stage
}

release_set bridge -dev
# the plain set last, so target/ holds what most people download
release_set "" ""

rm -f dist/Wisp-arm64 dist/Wisp-x86_64 dist/WispMenu-arm64 dist/WispMenu-x86_64
ls -la dist
