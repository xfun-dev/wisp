#!/bin/zsh
# Build, sign and install wisp on this Mac and on the Windows box (via the ComfyUI bridge).
# usage: scripts/deploy.sh [mac|win]   (default: both)
set -euo pipefail
cd "${0:A:h}/.."
source ~/.cargo/env

SIGN_ID=${WISP_SIGN_ID:-"Wisp Dev"}
LABEL="fun.xfun.wisp"
# the Windows machine is reached through a bridge configured in scripts/local.env (see README)
[[ -f scripts/local.env ]] && source scripts/local.env
BRIDGE=(${=WISP_WIN_BRIDGE:-})
WIN_DIR=${WISP_WIN_DIR:-}
target=${1:-all}

scripts/verify.sh

if [[ $target == all || $target == mac ]]; then
  # a stable signing identity keeps the Accessibility grant valid across rebuilds;
  # the core goes inside the app, and the app installs it (RULES.md I4)
  app=target/Wisp.app
  codesign --force -s "$SIGN_ID" -i "$LABEL" $app/Contents/Helpers/wisp
  codesign --force -s "$SIGN_ID" $app
  pkill -f "/Applications/Wisp.app/Contents/MacOS/Wisp" || true
  rm -rf /Applications/Wisp.app && cp -R $app /Applications/
  /Applications/Wisp.app/Contents/MacOS/Wisp --install
  echo "mac: installed and restarted"
fi

if [[ $target == all || $target == win ]]; then
  cargo build --release --quiet --target x86_64-pc-windows-gnu
  (( ${#BRIDGE} )) && [[ -n $WIN_DIR ]] || { print -u2 "deploy: set WISP_WIN_BRIDGE and WISP_WIN_DIR in scripts/local.env"; exit 1; }
  "${BRIDGE[@]}" put target/x86_64-pc-windows-gnu/release/wisp.exe "$WIN_DIR/wisp.new.exe" >/dev/null
  # the exe installs itself (RULES.md I1); quietly, without opening the status window
  "${BRIDGE[@]}" exec "${WIN_DIR//\//\\}\\wisp.new.exe install --quiet" --timeout 60
fi

# give the client a moment to find the server, then check the real install
sleep 5
scripts/verify.sh --live
