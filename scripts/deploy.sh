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
# split like the shell would, so a quoted path with spaces stays one word
BRIDGE=(${(Q)${(z)WISP_WIN_BRIDGE:-}})
WIN_DIR=${WISP_WIN_DIR:-}
target=${1:-all}

scripts/verify.sh

if [[ $target == all || $target == mac ]]; then
  # a stable signing identity keeps the Accessibility grant valid across rebuilds;
  # the core goes inside the app, and the app installs it (RULES.md I4)
  app=target/Wisp.app
  # your own machines get the developer bridge (RULES.md X1); releases never do
  cargo build --release --quiet --features bridge
  cp target/release/wisp $app/Contents/Helpers/wisp
  codesign --force -s "$SIGN_ID" -i "$LABEL" $app/Contents/Helpers/wisp
  codesign --force -s "$SIGN_ID" -i "$LABEL.menu" $app/Contents/Helpers/WispMenu
  codesign --force -s "$SIGN_ID" $app
  pkill -f "/Applications/Wisp.app/Contents/MacOS/Wisp" || true
  rm -rf /Applications/Wisp.app && cp -R $app /Applications/
  /Applications/Wisp.app/Contents/MacOS/Wisp --install
  echo "mac: installed and restarted"
fi

if [[ $target == all || $target == win ]]; then
  # cargo may leave the last other-feature build in place of wisp.exe: force the link,
  # and refuse to send an exe without the bridge, which would cut this very link off
  touch src/main.rs
  cargo build --release --quiet --target x86_64-pc-windows-gnu --features bridge
  LC_ALL=C grep -qa "bridge-exec" target/x86_64-pc-windows-gnu/release/wisp.exe || LC_ALL=C grep -qa "开发者通道" target/x86_64-pc-windows-gnu/release/wisp.exe || { print -u2 "deploy: wisp.exe has no developer bridge; not sending it"; exit 1; }
  (( ${#BRIDGE} )) && [[ -n $WIN_DIR ]] || { print -u2 "deploy: set WISP_WIN_BRIDGE and WISP_WIN_DIR in scripts/local.env"; exit 1; }
  # the Mac core may have just restarted: Wisp's own bridge needs it up and the PC back
  for _ in {1..60}; do
    "$HOME/Library/Application Support/Wisp/wisp" ctl status 2>/dev/null | grep -q state=connected && break
    sleep 0.5
  done
  "${BRIDGE[@]}" put target/x86_64-pc-windows-gnu/release/wisp.exe "$WIN_DIR/wisp.new.exe" >/dev/null
  if "${BRIDGE[@]}" exec "sc query WispService >nul" --timeout 15 >/dev/null 2>&1; then
    # WispService on (RULES.md W5): this user's copy without UAC, then the service's
    # client over the bridge, which the service puts in place (restarting the link)
    "${BRIDGE[@]}" exec "\"${WIN_DIR//\//\\}\\wisp.new.exe\" install --quiet --user-only" --timeout 60
    "${BRIDGE[@]}" put target/x86_64-pc-windows-gnu/release/wisp.exe wisp:update >/dev/null
  else
    # the exe installs itself (RULES.md I1); quietly, without opening the status window
    # started, not waited for: installing restarts the client, which may be the very
    # link this command travels over
    "${BRIDGE[@]}" exec "start \"\" \"${WIN_DIR//\//\\}\\wisp.new.exe\" install --quiet" --timeout 5 || true   # the answer may not make it back
  fi
  sleep 3
fi

# give the client a moment to find the server, then check the real install
sleep 5
scripts/verify.sh --live
