// The optional menu bar icon (RULES.md U12): a tiny AppKit-only process, started by its
// own LaunchAgent only when the user turns it on in the settings window. It asks the
// core for its status over the local control port (U6) every two seconds and offers
// pause, the settings window and quit. No SwiftUI, no windows: it stays small.
import AppKit

@main
enum MenuBar {
    static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        let controller = Controller()
        app.delegate = controller
        withExtendedLifetime(controller) { app.run() }
    }
}

/// The menu's few words in the system's language (RULES.md U14). This process is
/// copied out of the app, away from its .lproj files, so they live here: English,
/// Simplified Chinese, Traditional Chinese, Japanese, Korean.
enum Words {
    static let column: Int = {
        let tag = (Locale.preferredLanguages.first ?? "en").lowercased()
        if tag.hasPrefix("zh") {
            return tag.contains("hant") || tag.hasSuffix("-tw") || tag.hasSuffix("-hk") || tag.hasSuffix("-mo") ? 2 : 1
        }
        return tag.hasPrefix("ja") ? 3 : tag.hasPrefix("ko") ? 4 : 0
    }()

    static func of(_ all: [String]) -> String { all[column] }

    static var notRunning: String { of(["Wisp isn't running", "Wisp 没有在运行", "Wisp 未執行", "Wisp は動作していません", "Wisp가 실행 중이 아닙니다"]) }
    static var waitingToPair: String { of(["Waiting to pair", "正在等待配对", "正在等待配對", "ペアリング待ち", "페어링 대기 중"]) }
    static var notPaired: String { of(["Not paired", "还没有配对", "尚未配對", "未ペアリング", "페어링 안 됨"]) }
    static var paused: String { of(["Switching paused", "已暂停切屏", "已暫停切換", "切り替えを一時停止中", "전환 일시 정지됨"]) }
    static func connected(_ name: String) -> String { of(["Connected to \(name)", "已连接 \(name)", "已連線 \(name)", "\(name) に接続中", "\(name)에 연결됨"]) }
    static func waiting(_ name: String) -> String { of(["Waiting for \(name)", "等待 \(name) 连接", "等待 \(name) 連線", "\(name) の接続を待っています", "\(name) 연결을 기다리는 중"]) }
    static var pause: String { of(["Pause Switching", "暂停切屏", "暫停切換", "切り替えを一時停止", "전환 일시 정지"]) }
    static var start: String { of(["Start Wisp", "启动 Wisp", "啟動 Wisp", "Wisp を起動", "Wisp 시작"]) }
    static var settings: String { of(["Open Wisp Settings…", "打开 Wisp 设置…", "開啟 Wisp 設定…", "Wisp の設定を開く…", "Wisp 설정 열기…"]) }
    static var quit: String { of(["Quit Wisp", "退出 Wisp", "結束 Wisp", "Wisp を終了", "Wisp 종료"]) }
}

/// One `status` reply from the core, as far as the menu needs it.
struct Snapshot: Equatable {
    var running = false
    var connected = false
    var paused = false
    var paired = ""
    var pairing = false

    init() {}

    init?(reply: String) {
        guard reply.hasPrefix("ok\t") else { return nil }
        var f: [Substring: Substring] = [:]
        for word in reply.split(separator: "\t") {
            let kv = word.split(separator: "=", maxSplits: 1)
            if kv.count == 2 { f[kv[0]] = kv[1] }
        }
        running = true
        connected = f["state"] == "connected"
        paused = f["paused"] == "true"
        paired = String(f["paired"] ?? "")
        pairing = !(f["pairing"] ?? "").isEmpty
    }

    var words: String {
        if !running { return Words.notRunning }
        if pairing { return Words.waitingToPair }
        if paired.isEmpty { return Words.notPaired }
        if paused { return Words.paused }
        return connected ? Words.connected(paired) : Words.waiting(paired)
    }

    /// The icon is a template, drawn by macOS in the menu bar's own colour; paused or not
    /// running shows it faint, and the menu says the rest.
    var opacity: CGFloat { !running || paused ? 0.4 : 1 }
}

/// The control port, spoken directly over a socket: no `wisp ctl` process every poll.
enum Control {
    static let port: UInt16 = 24802
    static var token: String? {
        try? String(contentsOfFile: NSHomeDirectory() + "/.config/wisp/ctl-token", encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Sends one command, returns the first line of the reply, or "" if the core is not there.
    static func ask(_ command: String) -> String {
        guard let token else { return "" }
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { return "" }
        defer { close(fd) }
        var timeout = timeval(tv_sec: 1, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
        setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
        var addr = sockaddr_in()
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        addr.sin_addr.s_addr = inet_addr("127.0.0.1")
        let ok = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        guard ok == 0 else { return "" }
        let request = Array("auth \(token)\n\(command)\n".utf8)
        guard write(fd, request, request.count) == request.count else { return "" }
        var reply = [UInt8]()
        var byte: UInt8 = 0
        while read(fd, &byte, 1) == 1, byte != 10, reply.count < 4096 { reply.append(byte) }
        return String(decoding: reply, as: UTF8.self)
    }
}

final class Controller: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    private let menu = NSMenu()
    private var shown = Snapshot()
    private var timer: Timer?

    func applicationDidFinishLaunching(_ notification: Notification) {
        menu.delegate = self
        menu.autoenablesItems = false
        item.menu = menu
        item.button?.setAccessibilityLabel("Wisp")
        show(Snapshot())
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in self?.refresh() }
        timer?.tolerance = 0.5
    }

    private func refresh() {
        DispatchQueue.global(qos: .utility).async {
            let s = Snapshot(reply: Control.ask("status")) ?? Snapshot()
            DispatchQueue.main.async { if s != self.shown { self.show(s) } }
        }
    }

    private func show(_ s: Snapshot) {
        shown = s
        item.button?.image = Self.glyph(s.opacity)
        item.button?.toolTip = "Wisp：\(s.words)"
        rebuild()
    }

    /// The menu is rebuilt from the latest status whenever it changes or opens.
    func menuWillOpen(_ menu: NSMenu) { refresh() }

    private func rebuild() {
        menu.removeAllItems()
        let state = NSMenuItem(title: shown.words, action: nil, keyEquivalent: "")
        state.isEnabled = false
        menu.addItem(state)
        menu.addItem(.separator())
        if shown.running {
            let pause = NSMenuItem(title: Words.pause, action: #selector(togglePause), keyEquivalent: "")
            pause.target = self
            pause.state = shown.paused ? .on : .off
            menu.addItem(pause)
        } else {
            let start = NSMenuItem(title: Words.start, action: #selector(startCore), keyEquivalent: "")
            start.target = self
            menu.addItem(start)
        }
        let settings = NSMenuItem(title: Words.settings, action: #selector(openSettings), keyEquivalent: ",")
        settings.target = self
        menu.addItem(settings)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: Words.quit, action: #selector(quitAll), keyEquivalent: "q")
        quit.target = self
        menu.addItem(quit)
    }

    @objc private func togglePause() {
        let command = shown.paused ? "resume" : "pause"
        DispatchQueue.global(qos: .userInitiated).async {
            _ = Control.ask(command)
            DispatchQueue.main.async { self.refresh() }
        }
    }

    @objc private func startCore() {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        p.arguments = ["kickstart", "gui/\(getuid())/fun.xfun.wisp"]
        try? p.run()
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { self.refresh() }
    }

    @objc private func openSettings() {
        // the settings app, wherever it was installed
        if let app = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "fun.xfun.wisp.settings") {
            NSWorkspace.shared.openApplication(at: app, configuration: NSWorkspace.OpenConfiguration())
        }
    }

    /// Quits all of Wisp (U11): the core (stopped until the next login or until Wisp is
    /// opened), an open settings window, and this icon.
    @objc private func quitAll() {
        _ = Control.ask("quit")
        for app in NSRunningApplication.runningApplications(withBundleIdentifier: "fun.xfun.wisp.settings") {
            app.terminate()
        }
        NSApp.terminate(nil)
    }

    /// The 一缕 glyph, 18 pt, as a template image.
    static func glyph(_ opacity: CGFloat) -> NSImage {
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: true) { _ in
            let p = NSBezierPath()
            let s: CGFloat = 0.19, dx: CGFloat = -0.6, dy: CGFloat = -0.5
            func pt(_ x: CGFloat, _ y: CGFloat) -> NSPoint { NSPoint(x: x * s + dx, y: y * s + dy) }
            p.move(to: pt(16, 38))
            p.curve(to: pt(33, 72), controlPoint1: pt(22, 60), controlPoint2: pt(26, 72))
            p.curve(to: pt(50, 48), controlPoint1: pt(41, 72), controlPoint2: pt(43, 48))
            p.curve(to: pt(67, 72), controlPoint1: pt(57, 48), controlPoint2: pt(59, 72))
            p.curve(to: pt(83, 43), controlPoint1: pt(75, 72), controlPoint2: pt(79, 57))
            p.curve(to: pt(72, 28), controlPoint1: pt(86, 32), controlPoint2: pt(80, 23))
            p.lineWidth = 2.2
            p.lineCapStyle = .round
            p.lineJoinStyle = .round
            NSColor.black.withAlphaComponent(opacity).setStroke()
            p.stroke()
            return true
        }
        image.isTemplate = true
        return image
    }
}
