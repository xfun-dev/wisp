#!/bin/zsh
# Renders the Mac settings window in each state, light and dark, to PNG files, from
# fixed status lines: nothing runs, nothing is sent (RULES.md B4).
# usage: scripts/ui-states.sh [out-dir]   (default: target/ui-states)
set -euo pipefail
cd "${0:A:h}/.."
out=${1:-target/ui-states}
mkdir -p $out target/ui-harness
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
target/ui-harness/render $out
ls $out
