#!/bin/zsh
# Renders the Mac settings window in each state, light and dark, to PNG files, from
# fixed status lines: nothing runs, nothing is sent (RULES.md B4). With languages
# after the directory, each of them too (RULES.md U14): en zh-Hans zh-Hant ja ko.
# usage: scripts/ui-states.sh [out-dir [language...]]   (default: target/ui-states, the system's)
set -euo pipefail
cd "${0:A:h}/.."
out=${1:-target/ui-states}
langs=(${@:2})
mkdir -p $out target/ui-harness
# the window's words sit beside the renderer, where Bundle.main finds them
cp -R ui/mac/*.lproj target/ui-harness/
# the window's source without its @main, plus a small renderer
sed 's/^@main$//' ui/mac/WispSettings.swift > target/ui-harness/Settings.swift
cat > target/ui-harness/Render.swift <<'EOF'
import SwiftUI

@main
struct Render {
    @MainActor static func main() {
        let out = CommandLine.arguments[1]
        let now = Int(Date().timeIntervalSince1970)
        let base = "ok\tpaused=false\tedge=top\tspeed=1.5\tscroll=3\tpair_left=0\tpair_tries=0\tbridge=1"
        let states: [(String, String?, String?)] = [
            ("connected", "\(base)\tstate=connected\tpeer=192.168.1.12:50000\tscreen=2560x1440\tpaired=FANG\tpairing=", nil),
            ("connected-left", "\(base.replacingOccurrences(of: "edge=top", with: "edge=left"))\tstate=connected\tpeer=192.168.1.12:50000\tscreen=2560x1440\tpaired=FANG\tpairing=", nil),
            ("placed-top-125", "\(base.replacingOccurrences(of: "\tbridge=1", with: ""))\tstate=connected\tpeer=192.168.1.12:50000\tscreen=2560x1440\tscale=1.25\toffset=600\tpaired=FANG\tpairing=", nil),
            ("placed-left-up", "\(base)\tedge=left\toffset=-400\tstate=connected\tpeer=192.168.1.12:50000\tscreen=2560x1440\tscale=1.25\tpaired=FANG\tpairing=", nil),
            ("placed-right-small", "\(base)\tedge=right\toffset=300\tstate=connected\tpeer=192.168.1.12:50000\tscreen=1280x720\tscale=1\tpaired=FANG\tpairing=", nil),
            ("paused", "\(base.replacingOccurrences(of: "paused=false", with: "paused=true"))\tstate=connected\tpeer=192.168.1.12:50000\tscreen=2560x1440\tpaired=FANG\tpairing=", nil),
            ("searching", "\(base)\tstate=waiting\tpaired=FANG\tpairing=\tdrop=\(now - 150):slept", nil),
            ("unpaired", "\(base)\tstate=waiting\tpaired=\tpairing=", nil),
            ("pairing", "\(base.replacingOccurrences(of: "pair_left=0\tpair_tries=0", with: "pair_left=104\tpair_tries=3"))\tstate=waiting\tpaired=\tpairing=482917", nil),
            ("not-running", nil, "Wisp 没有在运行"),
        ]
        for (name, line, problem) in states {
            for dark in [false, true] {
                NSAppearance.current = NSAppearance(named: dark ? .darkAqua : .aqua)
                let view = SettingsView(core: Core(fixed: line.flatMap(Status.init(reply:)), problem: problem))
                    .environment(\.colorScheme, dark ? .dark : .light)
                let r = ImageRenderer(content: view)
                r.scale = 2
                guard let img = r.nsImage, let tiff = img.tiffRepresentation,
                      let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) else { continue }
                try? png.write(to: URL(fileURLWithPath: "\(out)/\(name)\(dark ? "-dark" : "").png"))
            }
        }
    }
}
EOF
swiftc -O -parse-as-library -target $(uname -m)-apple-macos14 target/ui-harness/Settings.swift target/ui-harness/Render.swift -o target/ui-harness/render
if (( ${#langs} )); then
  for l in $langs; do mkdir -p $out/$l && target/ui-harness/render $out/$l -AppleLanguages "($l)"; done
else
  target/ui-harness/render $out
fi
ls -R $out | head -40
