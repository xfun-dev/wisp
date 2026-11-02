#!/bin/zsh
# Build, sign and install wisp on this Mac and on the Windows box (via the ComfyUI bridge).
# usage: scripts/deploy.sh [mac|win]   (default: both)
set -euo pipefail
cd "${0:A:h}/.."
source ~/.cargo/env

SIGN_ID=${WISP_SIGN_ID:-"Wisp Dev"}
LABEL="fun.xfun.wisp"
MAC_DIR="$HOME/Library/Application Support/Wisp"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
# the Windows machine is reached through a bridge configured in scripts/local.env (see README)
[[ -f scripts/local.env ]] && source scripts/local.env
BRIDGE=(${=WISP_WIN_BRIDGE:-})
WIN_DIR=${WISP_WIN_DIR:-}
target=${1:-all}

scripts/verify.sh

if [[ $target == all || $target == mac ]]; then
  cargo build --release --quiet
  # a stable signing identity keeps the Accessibility grant valid across rebuilds
  codesign --force -s "$SIGN_ID" -i "$LABEL" target/release/wisp
  mkdir -p "$MAC_DIR"
  cp target/release/wisp "$MAC_DIR/wisp"
  cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>$LABEL</string>
    <key>ProgramArguments</key>
    <array>
        <string>$MAC_DIR/wisp</string>
        <string>server</string>
    </array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
    <key>LimitLoadToSessionType</key><string>Aqua</string>
    <key>ProcessType</key><string>Interactive</string>
    <key>StandardErrorPath</key><string>$HOME/Library/Logs/wisp.log</string>
</dict>
</plist>
EOF
  plutil -lint -s "$PLIST"
  # bootout returns before the service is gone, and bootstrapping too early fails
  # with "5: Input/output error" and leaves wisp stopped: wait, then retry
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
  for _ in {1..50}; do launchctl print "gui/$(id -u)/$LABEL" >/dev/null 2>&1 || break; sleep 0.1; done
  for attempt in 1 2 3; do
    launchctl bootstrap "gui/$(id -u)" "$PLIST" && break
    (( attempt == 3 )) && { print -u2 "deploy: could not start $LABEL"; exit 1; }
    sleep 1
  done
  # settings window, built by verify.sh; an open copy is closed so the new one replaces it cleanly
  codesign --force -s "$SIGN_ID" target/Wisp.app
  pkill -f "/Applications/Wisp.app/Contents/MacOS/Wisp" || true
  rm -rf /Applications/Wisp.app && cp -R target/Wisp.app /Applications/
  echo "mac: installed and restarted"
fi

if [[ $target == all || $target == win ]]; then
  cargo build --release --quiet --target x86_64-pc-windows-gnu
  (( ${#BRIDGE} )) && [[ -n $WIN_DIR ]] || { print -u2 "deploy: set WISP_WIN_BRIDGE and WISP_WIN_DIR in scripts/local.env"; exit 1; }
  "${BRIDGE[@]}" put target/x86_64-pc-windows-gnu/release/wisp.exe "$WIN_DIR/wisp.new.exe" >/dev/null
  "${BRIDGE[@]}" put scripts/install.ps1 "$WIN_DIR/install.ps1" >/dev/null
  "${BRIDGE[@]}" exec "powershell -NoProfile -ExecutionPolicy Bypass -File ${WIN_DIR//\//\\}\\install.ps1" --timeout 60
fi

# give the client a moment to find the server, then check the real install
sleep 5
scripts/verify.sh --live
