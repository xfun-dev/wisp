#!/bin/zsh
# Checks that must pass before anything ships. Exits non-zero on the first failure.
#   scripts/verify.sh          tests + warning-free builds for both platforms
#   scripts/verify.sh --live   the above, plus the running install: both sides
#                              connected and within the memory budget (RULES.md R1)
set -euo pipefail
cd "${0:A:h}/.."
source ~/.cargo/env

MAC_BUDGET_KB=4096   # phys_footprint of the Mac server
WIN_BUDGET_KB=2048   # private bytes of the Windows client
# the Windows machine is reached through a bridge configured in scripts/local.env (see README)
[[ -f scripts/local.env ]] && source scripts/local.env
BRIDGE=(${=WISP_WIN_BRIDGE:-})

fail() { print -u2 "verify: FAIL: $*"; exit 1; }
pass() { print "verify: ok: $*"; }

cargo test --release --quiet 2>&1 | tail -n 3 | grep -q "test result: ok" || { cargo test --release; fail "tests"; }
pass "tests"

for target in "" "x86_64-pc-windows-gnu"; do
  name=${target:-macos}
  args=(--release --message-format short)
  [[ -n $target ]] && args+=(--target $target)
  out=$(cargo build "${args[@]}" 2>&1) || { print -u2 "$out"; fail "build $name"; }
  if print -r -- "$out" | grep -q "warning:"; then print -u2 "$out"; fail "warnings in $name build"; fi
  pass "build $name, no warnings"
done

# the settings window: build the bundle deploy.sh installs
app=target/Wisp.app
rm -rf $app && mkdir -p $app/Contents/MacOS $app/Contents/Resources $app/Contents/Helpers && cp ui/mac/Info.plist $app/Contents/
# the app carries the core and installs it on launch (RULES.md I4)
cp target/release/wisp $app/Contents/Helpers/wisp
cp ui/mac/AppIcon.icns ui/mac/fonts/* $app/Contents/Resources/
out=$(swiftc -O -parse-as-library -target arm64-apple-macos14 ui/mac/WispSettings.swift -o $app/Contents/MacOS/Wisp 2>&1) || { print -u2 "$out"; fail "build settings window"; }
[[ -z $out ]] || { print -u2 "$out"; fail "warnings in settings window build"; }
pass "build settings window, no warnings"
out=$(swiftc -O -parse-as-library -target arm64-apple-macos14 ui/mac/WispMenuBar.swift -o $app/Contents/Helpers/WispMenu 2>&1) || { print -u2 "$out"; fail "build menu bar icon"; }
[[ -z $out ]] || { print -u2 "$out"; fail "warnings in menu bar icon build"; }
pass "build menu bar icon, no warnings"

[[ ${1:-} == --live ]] || exit 0

pid=$(pgrep -f "Wisp/wisp server") || fail "Mac server not running"
kb=$(footprint "$pid" 2>/dev/null | awk '/phys_footprint:/ {v=$2; u=$3; if (u=="MB") v*=1024; print int(v); exit}')
[[ -n $kb ]] || fail "could not read Mac footprint"
(( kb <= MAC_BUDGET_KB )) || fail "Mac footprint ${kb} KB > ${MAC_BUDGET_KB} KB"
pass "Mac footprint ${kb} KB (budget ${MAC_BUDGET_KB})"

ctl_status=$("$HOME/Library/Application Support/Wisp/wisp" ctl status 2>&1) || fail "wisp ctl status: $ctl_status"
[[ $ctl_status == *state=connected* ]] || fail "Mac server has no client: $ctl_status"
pass "Mac server has a client"

if (( ${#BRIDGE} == 0 )); then
  print "verify: skip: Windows checks (no WISP_WIN_BRIDGE in scripts/local.env)"
  exit 0
fi
# the background client by its command line: a status window is a wisp process too.
# A failing remote command must say so, not end this script silently (set -e, pipefail).
raw=$("${BRIDGE[@]}" exec 'powershell -NoProfile -Command "$c = Get-CimInstance Win32_Process | Where-Object { $_.Name -eq \"wisp.exe\" -and $_.CommandLine -match \" client\" } | Select-Object -First 1; if ($c) { [int]((Get-Process -Id $c.ProcessId).PrivateMemorySize64/1KB) } else { 0 }"' --timeout 30 2>&1) || true
win=$(print -r -- "$raw" | grep -Eo '^[0-9]+' | head -1 || true)
[[ $win == <1-> ]] || fail "Windows client not running: $raw"
(( win <= WIN_BUDGET_KB )) || fail "Windows private memory ${win} KB > ${WIN_BUDGET_KB} KB"
pass "Windows private memory ${win} KB (budget ${WIN_BUDGET_KB})"

win_status=$("${BRIDGE[@]}" exec 'powershell -NoProfile -Command "Get-Content -Encoding UTF8 $env:LOCALAPPDATA\Wisp\status -ErrorAction SilentlyContinue"' --timeout 30 2>&1) || true
[[ $win_status == *state=connected* ]] || fail "Windows status file does not say connected: $win_status"
pass "Windows status file says connected"
