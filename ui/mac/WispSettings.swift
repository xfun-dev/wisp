// Wisp settings window (RULES.md U1-U4, U11): opened on demand, quits when closed;
// starts the background core if it is not running, and can stop it.
// Talks to the running core only through `wisp ctl`, so the protocol lives in one place.
import SwiftUI

@main
struct WispSettings: App {
    @NSApplicationDelegateAdaptor(QuitWhenClosed.self) var delegate

    init() {
        // `Wisp --install`: install or update the background core and quit (scripts/deploy.sh)
        if CommandLine.arguments.contains("--install") {
            let problem = Installer.installIfNeeded()
            if let problem { FileHandle.standardError.write(Data((problem + "\n").utf8)) }
            exit(problem == nil ? 0 : 1)
        }
        // `Wisp --uninstall`: remove everything without asking (for scripts; the menu asks first)
        if CommandLine.arguments.contains("--uninstall") {
            Installer.uninstall()
            exit(0)
        }
    }

    var body: some Scene {
        Window("Wisp", id: "settings") {
            SettingsView()
        }
        .windowResizability(.contentSize)
        // no title bar strip or separator: the traffic lights share a row with the brand mark
        .windowStyle(.hiddenTitleBar)
        .commands {
            CommandGroup(after: .appInfo) {
                Button("卸载 Wisp…") { Installer.confirmUninstall() }
            }
        }
    }
}

/// The app carries the background core at Contents/Helpers/wisp and puts it in place
/// itself (RULES.md I4): copied to ~/Library/Application Support/Wisp, started by a
/// LaunchAgent at login. Dragging a newer app over the old one and opening it updates
/// the core; nothing happens when the installed core is already this one.
enum Installer {
    static let label = "fun.xfun.wisp"
    static var job: String { "gui/\(getuid())/\(label)" }
    static let home = NSHomeDirectory()
    static var core: String { home + "/Library/Application Support/Wisp/wisp" }
    static var plist: String { home + "/Library/LaunchAgents/\(label).plist" }
    /// the optional menu bar icon (RULES.md U12), with its own LaunchAgent while turned on
    static let menuLabel = "fun.xfun.wisp.menu"
    static var menuJob: String { "gui/\(getuid())/\(menuLabel)" }
    static var menu: String { home + "/Library/Application Support/Wisp/WispMenu" }
    static var menuPlist: String { home + "/Library/LaunchAgents/\(menuLabel).plist" }
    static var menuEnabled: Bool { FileManager.default.fileExists(atPath: menuPlist) }

    static var menuPlistText: String {
        """
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0">
        <dict>
            <key>Label</key><string>\(menuLabel)</string>
            <key>ProgramArguments</key>
            <array>
                <string>\(menu)</string>
            </array>
            <key>RunAtLoad</key><true/>
            <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
            <key>LimitLoadToSessionType</key><string>Aqua</string>
            <key>ProcessType</key><string>Interactive</string>
        </dict>
        </plist>

        """
    }

    /// Turns the menu bar icon on (installed, started now and at every login) or off.
    static func setMenu(_ on: Bool) {
        if on {
            try? menuPlistText.write(toFile: menuPlist, atomically: true, encoding: .utf8)
            launchctl(["bootstrap", "gui/\(getuid())", menuPlist])
        } else {
            launchctl(["bootout", menuJob])
            try? FileManager.default.removeItem(atPath: menuPlist)
        }
    }

    static var plistText: String {
        """
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0">
        <dict>
            <key>Label</key><string>\(label)</string>
            <key>ProgramArguments</key>
            <array>
                <string>\(core)</string>
                <string>server</string>
            </array>
            <key>RunAtLoad</key><true/>
            <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
            <key>LimitLoadToSessionType</key><string>Aqua</string>
            <key>ProcessType</key><string>Interactive</string>
            <key>StandardErrorPath</key><string>\(home)/Library/Logs/wisp.log</string>
        </dict>
        </plist>

        """
    }

    /// Returns what went wrong, or nil when the core is in place (or the app has none to install).
    static func installIfNeeded() -> String? {
        let fm = FileManager.default
        let bundled = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/wisp").path
        guard fm.fileExists(atPath: bundled) else { return nil }
        updateMenu()
        let sameCore = fm.contentsEqual(atPath: bundled, andPath: core)
        let samePlist = (try? String(contentsOfFile: plist, encoding: .utf8)) == plistText
        if sameCore && samePlist { return nil }
        // stop the old core; bootout returns before it is gone
        _ = launchctl(["bootout", job])
        for _ in 0..<50 where launchctl(["print", job]) == 0 { usleep(100_000) }
        do {
            try fm.createDirectory(atPath: (core as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
            try fm.createDirectory(atPath: (plist as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
            if !sameCore {
                let fresh = core + ".new"
                try? fm.removeItem(atPath: fresh)
                try fm.copyItem(atPath: bundled, toPath: fresh)
                _ = try fm.replaceItemAt(URL(fileURLWithPath: core), withItemAt: URL(fileURLWithPath: fresh))
            }
            try plistText.write(toFile: plist, atomically: true, encoding: .utf8)
        } catch {
            return "安装后台失败：\(error.localizedDescription)"
        }
        _ = launchctl(["enable", job])
        for attempt in 1...3 {
            if launchctl(["bootstrap", "gui/\(getuid())", plist]) == 0 { return nil }
            if attempt < 3 { sleep(1) }
        }
        return "启动后台失败（launchctl bootstrap）"
    }

    /// Asks a running menu bar icon to quit. A normal quit (exit 0) is not restarted by
    /// launchd (KeepAlive only on failure), and it comes back at the next login.
    static func quitMenu() {
        for app in NSWorkspace.shared.runningApplications where app.executableURL?.lastPathComponent == "WispMenu" {
            app.terminate()
        }
    }

    /// Puts the bundled menu bar icon in place; restarts it if it is turned on and changed.
    private static func updateMenu() {
        let fm = FileManager.default
        let bundled = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers/WispMenu").path
        guard fm.fileExists(atPath: bundled), !fm.contentsEqual(atPath: bundled, andPath: menu) else { return }
        let on = menuEnabled
        if on { launchctl(["bootout", menuJob]) }
        try? fm.createDirectory(atPath: (menu as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        let fresh = menu + ".new"
        try? fm.removeItem(atPath: fresh)
        guard (try? fm.copyItem(atPath: bundled, toPath: fresh)) != nil else { return }
        _ = try? fm.replaceItemAt(URL(fileURLWithPath: menu), withItemAt: URL(fileURLWithPath: fresh))
        if on { setMenu(true) }
    }

    /// Asks, then removes Wisp from this Mac (RULES.md I6).
    @MainActor
    static func confirmUninstall() {
        let alert = NSAlert()
        alert.messageText = "卸载 Wisp？"
        alert.informativeText = "会停止 Wisp，删除它的后台程序、开机启动项、配对和设置，并把 Wisp 移到废纸篓。以后再用需要重新安装和配对。"
        alert.alertStyle = .warning
        alert.addButton(withTitle: "卸载")
        alert.addButton(withTitle: "取消")
        alert.buttons.first?.hasDestructiveAction = true
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        uninstall()
        NSApp.terminate(nil)
    }

    /// Stops the core and removes it, its LaunchAgent, its data and log, and finally
    /// this app (to the Trash). The Accessibility entry stays: macOS keys it by the
    /// core's path, and only an administrator can delete it (`tccutil` cannot).
    static func uninstall() {
        let fm = FileManager.default
        launchctl(["bootout", menuJob])
        launchctl(["bootout", job])
        for _ in 0..<50 where launchctl(["print", job]) == 0 { usleep(100_000) }
        for path in [menuPlist, plist, (core as NSString).deletingLastPathComponent, home + "/.config/wisp", home + "/Library/Logs/wisp.log"] {
            try? fm.removeItem(atPath: path)
        }
        let app = Bundle.main.bundleURL
        if app.pathExtension == "app" {
            NSWorkspace.shared.recycle([app]) { _, _ in }
            // the completion comes on the main queue; give it a moment before exiting
            RunLoop.main.run(until: Date().addingTimeInterval(1))
        }
    }

    @discardableResult
    private static func launchctl(_ args: [String]) -> Int32 {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        p.arguments = args
        p.standardOutput = FileHandle.nullDevice
        p.standardError = FileHandle.nullDevice
        guard (try? p.run()) != nil else { return -1 }
        p.waitUntilExit()
        return p.terminationStatus
    }
}

final class QuitWhenClosed: NSObject, NSApplicationDelegate {
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

/// One `wisp ctl status` reply: `ok`, then tab-separated `key=value` fields (names contain spaces)
struct Status: Equatable {
    var connected = false
    var peer = ""
    var screen = ""
    var paused = false
    var edge = "top"
    var speed = 1.0
    var scroll = 3.0
    /// the paired machine's name, empty while unpaired
    var paired = ""
    /// the open pairing code, empty when none
    var code = ""
    var codeSecondsLeft = 0
    var codeTriesLeft = 0
    /// developer bridge: 1 open, 0 closed; nil in a core built without it (RULES.md X1)
    var bridge: Int?
    /// when and why the last link ended (RULES.md U13), nil before any has
    var drop: (at: Date, reason: String)?

    static func == (a: Status, b: Status) -> Bool {
        a.connected == b.connected && a.peer == b.peer && a.screen == b.screen && a.paused == b.paused
            && a.edge == b.edge && a.speed == b.speed && a.scroll == b.scroll && a.paired == b.paired
            && a.code == b.code && a.codeSecondsLeft == b.codeSecondsLeft && a.codeTriesLeft == b.codeTriesLeft
            && a.bridge == b.bridge && a.drop?.at == b.drop?.at && a.drop?.reason == b.drop?.reason
    }

    init?(reply: String) {
        guard reply.hasPrefix("ok\t") else { return nil }
        var fields: [String: String] = [:]
        for word in reply.dropFirst(3).split(separator: "\t") {
            let pair = word.split(separator: "=", maxSplits: 1)
            if pair.count == 2 { fields[String(pair[0])] = String(pair[1]) }
        }
        connected = fields["state"] == "connected"
        peer = fields["peer"].map { String($0.split(separator: ":").first ?? "") } ?? ""
        screen = fields["screen"]?.replacingOccurrences(of: "x", with: " × ") ?? ""
        paused = fields["paused"] == "true"
        edge = fields["edge"] ?? edge
        speed = fields["speed"].flatMap(Double.init) ?? speed
        scroll = fields["scroll"].flatMap(Double.init) ?? scroll
        paired = fields["paired"] ?? ""
        code = fields["pairing"] ?? ""
        codeSecondsLeft = fields["pair_left"].flatMap(Int.init) ?? 0
        codeTriesLeft = fields["pair_tries"].flatMap(Int.init) ?? 0
        bridge = fields["bridge"].flatMap(Int.init)
        if let d = fields["drop"]?.split(separator: ":"), d.count == 2, let at = TimeInterval(d[0]) {
            let reason = switch d[1] {
            case "slept": "Mac 进入了睡眠"
            case "silent": "对方没有响应"
            default: "对方断开了连接"
            }
            drop = (Date(timeIntervalSince1970: at), reason)
        }
    }
}

@MainActor
final class Core: ObservableObject {
    @Published var status: Status?
    @Published var problem: String?
    private let wisp = NSHomeDirectory() + "/Library/Application Support/Wisp/wisp"
    private let job = "gui/\(getuid())/fun.xfun.wisp"
    private var timer: Timer?

    /// A fixed status, nothing running: for rendering each state in a test (scripts/ui-states.sh).
    init(fixed: Status?, problem: String? = nil) {
        status = fixed
        self.problem = problem
    }

    init() {
        Task {
            if let problem = await Task.detached(operation: { Installer.installIfNeeded() }).value {
                self.problem = problem
            }
            await startIfNeeded()
            refresh()
        }
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    func refresh() {
        Task {
            let reply = await ctl(["status"])
            if let s = Status(reply: reply) {
                status = s
                problem = nil
            } else {
                status = nil
                // no core to answer: plain words, not the socket error
                problem = reply.isEmpty || reply.contains("连不上") ? "Wisp 没有在运行" : reply
            }
        }
    }

    func send(_ args: [String]) {
        Task {
            let reply = await ctl(args)
            problem = reply.hasPrefix("ok") ? nil : reply.replacingOccurrences(of: "error ", with: "")
            refresh()
        }
    }

    /// Opening Wisp means "make Wisp run": after quitting, this brings the core back (RULES.md U11).
    private func startIfNeeded() async {
        // the menu bar icon, if turned on, comes back with Wisp (kickstart leaves a running one alone)
        if Installer.menuEnabled { _ = await run("/bin/launchctl", ["kickstart", Installer.menuJob]) }
        guard Status(reply: await ctl(["status"])) == nil else { return }
        let plist = NSHomeDirectory() + "/Library/LaunchAgents/fun.xfun.wisp.plist"
        // still loaded after a clean quit: kickstart; unloaded entirely: bootstrap
        if await run("/bin/launchctl", ["kickstart", job]).status != 0 {
            _ = await run("/bin/launchctl", ["bootstrap", "gui/\(getuid())", plist])
        }
        for _ in 0..<20 where Status(reply: await ctl(["status"])) == nil {
            try? await Task.sleep(nanoseconds: 200_000_000)
        }
    }

    /// "启动 Wisp" in the window: the same as opening Wisp again.
    func start() {
        Task {
            await startIfNeeded()
            refresh()
        }
    }

    /// Quitting Wisp quits all of it (RULES.md U11): the core, the menu bar icon, this window.
    func quit() {
        Task {
            _ = await ctl(["quit"])
            Installer.quitMenu()
            NSApp.terminate(nil)
        }
    }

    private func ctl(_ args: [String]) async -> String {
        await run(wisp, ["ctl"] + args).output
    }

    private func run(_ path: String, _ args: [String]) async -> (status: Int32, output: String) {
        await Task.detached {
            let p = Process()
            p.executableURL = URL(fileURLWithPath: path)
            p.arguments = args
            let out = Pipe()
            p.standardOutput = out
            p.standardError = out
            guard (try? p.run()) != nil else { return (-1, "") }
            let data = out.fileHandleForReading.readDataToEndOfFile()
            p.waitUntilExit()
            return (p.terminationStatus, String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines))
        }.value
    }
}

// MARK: - Look: paper and ink (RULES.md B4)
//
// One sheet of paper: ink-black words, hairline rules between sections, and a single
// wisp of celadon smoke for "connected". No cards, no shadows, nothing saturated.
// Follows the system's light and dark appearance.

extension NSColor {
    convenience init(hex: UInt32, alpha: CGFloat = 1) {
        self.init(srgbRed: CGFloat(hex >> 16 & 0xFF) / 255, green: CGFloat(hex >> 8 & 0xFF) / 255,
                  blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
    }

    /// One colour for light, one for dark, resolved by whatever it is drawn in.
    static func ink(_ light: UInt32, _ dark: UInt32) -> NSColor {
        NSColor(name: nil) { a in
            NSColor(hex: a.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? dark : light)
        }
    }
}

enum Ink {
    static let paper = NSColor.ink(0xF3EFE6, 0x1C1B19)
    static let wash = NSColor.ink(0xEAE4D8, 0x262421)
    static let text = NSColor.ink(0x23211D, 0xE9E4DA)
    static let soft = NSColor.ink(0x635E55, 0xA9A397)
    static let faint = NSColor.ink(0x8C867A, 0x7D776C)
    static let rule = NSColor.ink(0xDDD5C6, 0x34312C)
    static let ghost = NSColor.ink(0xB9B1A1, 0x4A463F)
    static let sheet = NSColor.ink(0xF8F5EE, 0x23211E)
    static let celadon = NSColor.ink(0x4E7068, 0x8DB3A8)
    static let ochre = NSColor.ink(0x8F6326, 0xD2A266)
    static let rust = NSColor.ink(0xA8452F, 0xE08A72)
}

extension Color {
    init(_ ns: NSColor) { self.init(nsColor: ns) }
}

extension Font {
    /// the headline face: Songti, which every Mac has
    static func serif(_ size: CGFloat) -> Font { .custom("Songti SC", size: size).weight(.semibold) }
    static func mono(_ size: CGFloat) -> Font { .system(size: size, design: .monospaced) }
}

// MARK: - State

enum Mood: Equatable {
    case connected, searching, unpaired, paused, pairing

    init(_ s: Status) {
        if !s.code.isEmpty { self = .pairing }
        else if s.paired.isEmpty { self = .unpaired }
        else if !s.connected { self = .searching }
        else if s.paused { self = .paused }
        else { self = .connected }
    }
}

/// The two speeds as three named steps and a fine setting (RULES.md U2).
struct Steps {
    let slow: Double, middle: Double, fast: Double
    let range: ClosedRange<Double>, step: Double
    let unit: (Double) -> String

    static let speed = Steps(slow: 0.6, middle: 1.0, fast: 1.5, range: 0.2...5, step: 0.1) { "\($0.formatted(.number.precision(.fractionLength(1))))×" }
    static let scroll = Steps(slow: 1.5, middle: 3, fast: 6, range: 0.5...10, step: 0.5) { "\($0.formatted(.number.precision(.fractionLength(0...1)))) 行" }

    /// 0 slow, 1 middle, 2 fast, 3 a value of its own
    func index(_ v: Double) -> Int {
        [slow, middle, fast].firstIndex { abs($0 - v) < 0.01 } ?? 3
    }
}

/// This Mac's address on the local network, if it has one.
func lanAddress() -> String? {
    var list: UnsafeMutablePointer<ifaddrs>?
    guard getifaddrs(&list) == 0, let first = list else { return nil }
    defer { freeifaddrs(list) }
    for p in sequence(first: first, next: { $0.pointee.ifa_next }) {
        let a = p.pointee
        guard let addr = a.ifa_addr, addr.pointee.sa_family == UInt8(AF_INET),
              a.ifa_flags & UInt32(IFF_UP) != 0, a.ifa_flags & UInt32(IFF_LOOPBACK) == 0,
              String(cString: a.ifa_name).hasPrefix("en") else { continue }
        var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
        if getnameinfo(addr, socklen_t(addr.pointee.sa_len), &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST) == 0 {
            return String(cString: host)
        }
    }
    return nil
}

// MARK: - Window

struct SettingsView: View {
    @StateObject private var core: Core
    @State private var speed = 1.0
    @State private var scroll = 3.0
    @State private var editing = false
    @State private var fineSpeed = false
    @State private var fineScroll = false
    @State private var confirmUnpair = false
    @State private var menuIcon = Installer.menuEnabled

    @MainActor init(core: Core? = nil) {
        _core = StateObject(wrappedValue: core ?? Core())
    }

    var body: some View {
        let s = core.status
        let mood = s.map(Mood.init)
        VStack(spacing: 0) {
            header(s, mood)
            Rule()
            stage(s, mood)
                .frame(height: 214)
            Rule()
            if let s, !s.paired.isEmpty, mood != .pairing {
                feel(s)
            }
            Spacer(minLength: 0)
            footer(s)
        }
        // below the title bar, which holds only the traffic lights
        .frame(width: 400, height: 500 + (fineSpeed ? 34 : 0) + (fineScroll ? 34 : 0))
        .background(Color(Ink.paper))
        .background(WindowPaper())
        .foregroundStyle(Color(Ink.text))
        .onChange(of: core.status) { _, s in
            guard let s, !editing else { return }
            speed = s.speed
            scroll = s.scroll
        }
    }

    // MARK: top

    private func header(_ s: Status?, _ mood: Mood?) -> some View {
        HStack(alignment: .bottom, spacing: 16) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 8) {
                    Circle().fill(Color(dot(mood, s))).frame(width: 7, height: 7)
                    Text(label(mood, s)).font(.system(size: 12)).tracking(1).foregroundStyle(Color(dot(mood, s)))
                }
                Text(headline(mood, s))
                    .font(.serif(30))
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
                    .padding(.top, 8)
                Text(detail(mood, s))
                    // figures in the monospaced face; Chinese sentences in the plain one
                    .font(mood == .connected ? .mono(12) : .system(size: 12))
                    .foregroundStyle(Color(Ink.soft))
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 6)
                if let problem = core.problem, s != nil {
                    Text(problem).font(.system(size: 12)).foregroundStyle(Color(Ink.rust)).padding(.top, 6)
                }
            }
            Spacer(minLength: 0)
            if let action = primary(mood, s) {
                Button(action.title, action: action.run).buttonStyle(Outline())
            }
        }
        .padding(.horizontal, 28)
        .padding(.top, 2)
        .padding(.bottom, 20)
        .animation(.easeOut(duration: 0.25), value: mood)
    }

    private func dot(_ mood: Mood?, _ s: Status?) -> NSColor {
        switch mood {
        case .connected: Ink.celadon
        case .searching, .paused, .pairing: Ink.ochre
        case .unpaired, nil: s == nil && core.problem != nil ? Ink.rust : Ink.faint
        }
    }

    private func label(_ mood: Mood?, _ s: Status?) -> String {
        guard let mood else { return core.problem == nil ? "正在打开" : "没有在运行" }
        return switch mood {
        case .connected: "已连接"
        case .searching: "正在寻找"
        case .paused: "已暂停切屏"
        case .unpaired: "还没有配对"
        case .pairing: "等待配对"
        }
    }

    private func headline(_ mood: Mood?, _ s: Status?) -> String {
        guard let s, let mood else { return "Wisp" }
        return switch mood {
        case .connected, .searching, .paused: s.paired
        case .unpaired: "配对一台电脑"
        case .pairing: "配对码"
        }
    }

    private func detail(_ mood: Mood?, _ s: Status?) -> String {
        guard let s, let mood else { return core.problem == nil ? "" : "点右边的按钮启动它" }
        switch mood {
        case .connected: return "\(s.peer) · \(s.screen.replacingOccurrences(of: " ", with: ""))"
        case .paused: return "光标不会切过去；Wisp 重启后自动恢复"
        case .searching:
            guard let drop = s.drop else { return "等它在局域网里出现" }
            return "已经找了 \(since(drop.at))"
        case .unpaired: return "让这台 Mac 的键盘和触控板控制另一台电脑"
        case .pairing: return "在 Windows 上的 Wisp 里输入下面 6 位数字"
        }
    }

    private struct Action { let title: String; let run: () -> Void }

    private func primary(_ mood: Mood?, _ s: Status?) -> Action? {
        guard let mood else { return core.problem == nil ? nil : Action(title: "启动 Wisp") { core.start() } }
        return switch mood {
        case .connected: Action(title: "暂停切屏") { core.send(["pause"]) }
        case .paused: Action(title: "恢复切屏") { core.send(["resume"]) }
        case .unpaired: Action(title: "配对新电脑") { core.send(["pair"]) }
        case .pairing: Action(title: "取消") { core.send(["pair-cancel"]) }
        case .searching: nil
        }
    }

    // MARK: middle

    @ViewBuilder private func stage(_ s: Status?, _ mood: Mood?) -> some View {
        switch mood {
        case .connected, .paused:
            Arrangement(edge: s?.edge ?? "top", name: s?.paired ?? "", live: mood == .connected) { core.send(["set", "edge=\($0)"]) }
        case .searching:
            if let s { Checklist(s: s) }
        case .pairing:
            if let s { PairingCode(s: s) }
        case .unpaired:
            VStack(spacing: 14) {
                Text("先在另一台电脑上装好 Wisp，再点上面的“配对新电脑”。")
                Text("两台电脑要在同一个局域网里。")
            }
            .font(.system(size: 13))
            .foregroundStyle(Color(Ink.soft))
            .multilineTextAlignment(.center)
            .padding(.horizontal, 40)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        case nil:
            Text(core.problem == nil ? "" : "Wisp 的后台没有在运行，键盘和触控板不会切到另一台电脑。")
                .font(.system(size: 13)).foregroundStyle(Color(Ink.soft))
                .multilineTextAlignment(.center).padding(.horizontal, 40)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    // MARK: the feel

    private func feel(_ s: Status) -> some View {
        VStack(spacing: 0) {
            stepRow("鼠标速度", steps: .speed, value: $speed, fine: $fineSpeed, key: "speed")
            Rule().padding(.horizontal, 28)
            stepRow("滚动速度", steps: .scroll, value: $scroll, fine: $fineScroll, key: "scroll")
        }
        .padding(.top, 4)
        .disabled(s.paired.isEmpty)
    }

    private func stepRow(_ title: String, steps: Steps, value: Binding<Double>, fine: Binding<Bool>, key: String) -> some View {
        let picked = fine.wrappedValue ? 3 : steps.index(value.wrappedValue)
        return VStack(spacing: 0) {
            HStack(spacing: 18) {
                Text(title).font(.system(size: 14))
                Spacer()
                ForEach(Array(["慢", "适中", "快", "细调"].enumerated()), id: \.offset) { i, word in
                    Button {
                        if i == 3 {
                            fine.wrappedValue.toggle()
                        } else {
                            fine.wrappedValue = false
                            let v = [steps.slow, steps.middle, steps.fast][i]
                            value.wrappedValue = v
                            core.send(["set", "\(key)=\(v)"])
                        }
                    } label: {
                        Text(word)
                            .font(.system(size: 13, weight: picked == i ? .medium : .regular))
                            .foregroundStyle(Color(picked == i ? Ink.text : Ink.faint))
                            .padding(.bottom, 3)
                            .overlay(alignment: .bottom) {
                                if picked == i { Rectangle().fill(Color(Ink.text)).frame(height: 1.5) }
                            }
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(picked == i ? .isSelected : [])
                    .accessibilityLabel(Text("\(title)\(word)"))
                }
            }
            .frame(height: 52)
            if picked == 3 {
                HStack(spacing: 14) {
                    Hairslider(value: value, range: steps.range, step: steps.step) { on in
                        editing = on
                        if !on { core.send(["set", "\(key)=\(value.wrappedValue)"]) }
                    }
                    .accessibilityLabel(Text(title))
                    Text(steps.unit(value.wrappedValue)).font(.mono(12)).foregroundStyle(Color(Ink.soft))
                        .frame(width: 52, alignment: .trailing)
                }
                .padding(.bottom, 14)
            }
        }
        .padding(.horizontal, 28)
    }

    // MARK: bottom

    private func footer(_ s: Status?) -> some View {
        HStack {
            if let open = s?.bridge, open > 0 {
                Text("开发者通道 · 开").font(.system(size: 12)).foregroundStyle(Color(Ink.soft))
            }
            Spacer()
            Menu {
                Toggle("菜单栏图标（约多占 10 MB 内存）", isOn: Binding(get: { menuIcon }, set: { on in
                    menuIcon = on
                    Task.detached { Installer.setMenu(on) }
                }))
                // only a developer build's core reports the field (RULES.md X1, X2)
                if let open = s?.bridge {
                    Toggle("开发者通道", isOn: Binding(get: { open > 0 }, set: { core.send(["bridge", $0 ? "on" : "off"]) }))
                }
                if let s, !s.paired.isEmpty {
                    Divider()
                    Button("配对另一台电脑…") { core.send(["pair"]) }
                    Button("解除与 \(s.paired) 的配对…", role: .destructive) { confirmUnpair = true }
                }
                Divider()
                Button("退出 Wisp") { core.quit() }
            } label: {
                Text("···").font(.system(size: 16, weight: .semibold)).tracking(2)
            }
            .menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel(Text("更多"))
            .confirmationDialog("解除与 \(s?.paired ?? "") 的配对？", isPresented: $confirmUnpair) {
                Button("解除配对", role: .destructive) { core.send(["unpair"]) }
            } message: {
                Text("这台 Mac 会断开与它的连接，以后要再用就得重新配对。")
            }
        }
        .padding(.horizontal, 28)
        .padding(.top, 12)
        .padding(.bottom, 16)
    }
}

/// "3 分钟", "1 小时 5 分钟" since a unix time.
func since(_ at: Date) -> String {
    let secs = max(0, Int(Date().timeIntervalSince(at)))
    switch secs {
    case ..<60: return "不到 1 分钟"
    case ..<3600: return "\(secs / 60) 分钟"
    default: return "\(secs / 3600) 小时 \(secs % 3600 / 60) 分钟"
    }
}

// MARK: - Pieces

struct Rule: View {
    var body: some View { Rectangle().fill(Color(Ink.rule)).frame(height: 1) }
}

/// A pill drawn with one ink line; filled with ink while pressed.
struct Outline: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 13, weight: .medium))
            .padding(.horizontal, 16)
            .frame(height: 32)
            .foregroundStyle(Color(configuration.isPressed ? Ink.paper : Ink.text))
            .background(Capsule().fill(Color(configuration.isPressed ? Ink.text : .clear)))
            .overlay(Capsule().strokeBorder(Color(Ink.text), lineWidth: 1))
            .contentShape(Capsule())
    }
}

/// Where the other screen stands, drawn in line: the Mac in the middle, the screen
/// above, left or right of it. Drag the screen to another place, or click a place.
struct Arrangement: View {
    let edge: String
    let name: String
    /// connected and not paused: the smoke is drawn in colour
    let live: Bool
    let pick: (String) -> Void
    @State private var drag: CGSize = .zero
    @State private var hover: String?

    private static let slots: [String: CGRect] = [
        "top": CGRect(x: 150, y: 16, width: 100, height: 64),
        "left": CGRect(x: 34, y: 92, width: 100, height: 64),
        "right": CGRect(x: 266, y: 92, width: 100, height: 64),
    ]
    private let mac = CGRect(x: 160, y: 110, width: 80, height: 50)

    var body: some View {
        let at = Self.slots[edge] ?? Self.slots["top"]!
        let screen = at.offsetBy(dx: drag.width, dy: drag.height)
        ZStack(alignment: .topLeading) {
            ForEach(["top", "left", "right"].filter { $0 != edge }, id: \.self) { slot in
                let r = Self.slots[slot]!
                Button { pick(slot) } label: {
                    RoundedRectangle(cornerRadius: 3)
                        .strokeBorder(Color(hover == slot || nearest(screen) == slot && drag != .zero ? Ink.soft : Ink.ghost),
                                      style: StrokeStyle(lineWidth: 1, dash: [3, 4]))
                        .overlay(Text(slot == "left" ? "左侧" : slot == "right" ? "右侧" : "上方")
                            .font(.system(size: 11)).foregroundStyle(Color(Ink.faint)))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .onHover { hover = $0 ? slot : nil }
                .frame(width: r.width, height: r.height)
                .offset(x: r.minX, y: r.minY)
                .accessibilityLabel(Text("把\(name)放到\(slot == "left" ? "左侧" : slot == "right" ? "右侧" : "上方")"))
            }
            Smoke(from: smokeStart, to: smokeEnd(at))
                .stroke(Color(live ? Ink.celadon : Ink.ghost), style: StrokeStyle(lineWidth: 1.6, lineCap: .round))
                .opacity(drag == .zero ? 1 : 0)
            LineMac().frame(width: 108, height: 62).offset(x: mac.minX - 14, y: mac.minY)
            LineScreen(name: name)
                .frame(width: screen.width, height: screen.height)
                .offset(x: screen.minX, y: screen.minY)
                .gesture(DragGesture()
                    .onChanged { drag = $0.translation }
                    .onEnded { _ in
                        let slot = nearest(screen)
                        withAnimation(.spring(response: 0.35, dampingFraction: 0.8)) { drag = .zero }
                        if slot != edge { pick(slot) }
                    })
                .onHover { inside in
                    if inside { NSCursor.openHand.push() } else { NSCursor.pop() }
                }
                .accessibilityLabel(Text("\(name) 在 Mac 的\(edge == "left" ? "左侧" : edge == "right" ? "右侧" : "上方")"))
            Text("拖动屏幕，放到它在桌上的位置")
                .font(.system(size: 11)).foregroundStyle(Color(Ink.faint))
                .frame(width: 400)
                .offset(y: 190)
        }
        .frame(width: 400, height: 214, alignment: .topLeading)
        .animation(.spring(response: 0.4, dampingFraction: 0.82), value: edge)
    }

    private var smokeStart: CGPoint {
        switch edge {
        case "left": CGPoint(x: mac.minX - 2, y: mac.midY - 6)
        case "right": CGPoint(x: mac.maxX + 2, y: mac.midY - 6)
        default: CGPoint(x: mac.midX, y: mac.minY - 2)
        }
    }

    private func smokeEnd(_ r: CGRect) -> CGPoint {
        switch edge {
        case "left": CGPoint(x: r.maxX + 4, y: r.midY + 4)
        case "right": CGPoint(x: r.minX - 4, y: r.midY + 4)
        default: CGPoint(x: r.midX, y: r.maxY + 6)
        }
    }

    private func nearest(_ r: CGRect) -> String {
        Self.slots.min { a, b in
            hypot(a.value.midX - r.midX, a.value.midY - r.midY) < hypot(b.value.midX - r.midX, b.value.midY - r.midY)
        }!.key
    }
}

/// One wisp of smoke between two points: a gentle S, never a straight line.
struct Smoke: Shape {
    let from: CGPoint
    let to: CGPoint

    func path(in rect: CGRect) -> Path {
        var p = Path()
        p.move(to: from)
        let dx = to.x - from.x, dy = to.y - from.y
        let len = max(hypot(dx, dy), 1)
        // perpendicular sway
        let nx = -dy / len * 7, ny = dx / len * 7
        p.addCurve(to: to,
                   control1: CGPoint(x: from.x + dx * 0.33 + nx, y: from.y + dy * 0.33 + ny),
                   control2: CGPoint(x: from.x + dx * 0.66 - nx, y: from.y + dy * 0.66 - ny))
        return p
    }
}

struct LineScreen: View {
    let name: String
    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 4).fill(Color(Ink.sheet))
            RoundedRectangle(cornerRadius: 4).strokeBorder(Color(Ink.text), lineWidth: 1.2)
            Text(name).font(.serif(14)).lineLimit(1).minimumScaleFactor(0.5).padding(.horizontal, 8)
        }
        .contentShape(Rectangle())
    }
}

/// A laptop in one line: the lid, then the base below it.
struct LineMac: View {
    var body: some View {
        Canvas { ctx, size in
            let lid = CGRect(x: 14, y: 0, width: 80, height: 50)
            ctx.fill(Path(roundedRect: lid, cornerRadius: 3), with: .color(Color(Ink.sheet)))
            ctx.stroke(Path(roundedRect: lid, cornerRadius: 3), with: .color(Color(Ink.text)), lineWidth: 1.2)
            var base = Path()
            base.move(to: CGPoint(x: 0, y: 54)); base.addLine(to: CGPoint(x: 108, y: 54))
            base.addLine(to: CGPoint(x: 100, y: 60)); base.addLine(to: CGPoint(x: 8, y: 60)); base.closeSubpath()
            ctx.stroke(base, with: .color(Color(Ink.text)), lineWidth: 1.2)
        }
        .accessibilityHidden(true)
    }
}

/// While searching: what has been checked, and what is left to look at (RULES.md U13).
struct Checklist: View {
    let s: Status
    private let address = lanAddress()

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            item(ok: address != nil, address != nil ? "这台 Mac 已联网" : "这台 Mac 没有连上局域网",
                 address.map { "\($0)" } ?? "检查 Wi-Fi 或网线")
            item(ok: true, "Mac 上的 Wisp 在等待连接", "端口 24801")
            VStack(alignment: .leading, spacing: 4) {
                mark(ok: false)
                    .overlay(alignment: .leading) {
                        Text("还没有收到 \(s.paired) 的回应").font(.system(size: 14)).fixedSize().offset(x: 26)
                    }
                Text("那台电脑关机、睡眠或卡住了\n它连的是另一个网络\n代理软件的 TUN 模式挡住了局域网广播")
                    .font(.system(size: 12)).foregroundStyle(Color(Ink.soft))
                    .lineSpacing(4)
                    .padding(.leading, 26)
            }
            if let drop = s.drop {
                Text("上次断开 · \(drop.at.formatted(date: .omitted, time: .shortened)) · \(drop.reason)")
                    .font(.system(size: 12)).foregroundStyle(Color(Ink.faint))
                    .padding(.top, 2)
            }
        }
        .padding(.horizontal, 28)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
    }

    private func item(ok: Bool, _ title: String, _ note: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            mark(ok: ok)
            Text(title).font(.system(size: 14))
            Text(note).font(.mono(11)).foregroundStyle(Color(Ink.faint))
        }
    }

    private func mark(ok: Bool) -> some View {
        Text(ok ? "✓" : "!")
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(Color(ok ? Ink.celadon : Ink.ochre))
            .frame(width: 14)
    }
}

/// The open pairing code: six figures in the headline face over hairlines.
struct PairingCode: View {
    let s: Status

    var body: some View {
        VStack(spacing: 18) {
            HStack(spacing: 10) {
                ForEach(Array(s.code.enumerated()), id: \.offset) { i, ch in
                    Text(String(ch))
                        .font(.serif(34))
                        .frame(width: 34)
                        .padding(.bottom, 4)
                        .overlay(alignment: .bottom) { Rectangle().fill(Color(Ink.text)).frame(height: 1) }
                        .padding(.leading, i == 3 ? 12 : 0)
                }
            }
            .accessibilityElement()
            .accessibilityLabel(Text("配对码 " + s.code.map(String.init).joined(separator: " ")))
            Text("还剩 \(s.codeSecondsLeft / 60):\(String(format: "%02d", s.codeSecondsLeft % 60)) · 还可以输错 \(s.codeTriesLeft) 次")
                .font(.mono(12)).foregroundStyle(Color(Ink.soft))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// One hairline with a small ink bead on it.
struct Hairslider: View {
    @Binding var value: Double
    let range: ClosedRange<Double>
    let step: Double
    let editing: (Bool) -> Void
    @State private var dragging = false

    var body: some View {
        GeometryReader { g in
            let w = g.size.width
            let t = (value - range.lowerBound) / (range.upperBound - range.lowerBound)
            ZStack(alignment: .leading) {
                Rectangle().fill(Color(Ink.rule)).frame(height: 1)
                Rectangle().fill(Color(Ink.text)).frame(width: max(0, w * t), height: 1.5)
                Circle().fill(Color(Ink.text)).frame(width: dragging ? 12 : 10, height: dragging ? 12 : 10)
                    .offset(x: (w - 10) * t)
            }
            .frame(width: w, height: 20)
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0)
                .onChanged { d in
                    if !dragging { dragging = true; editing(true) }
                    let x = min(max(0, d.location.x / max(w, 1)), 1)
                    let raw = range.lowerBound + x * (range.upperBound - range.lowerBound)
                    value = min(max((raw / step).rounded() * step, range.lowerBound), range.upperBound)
                }
                .onEnded { _ in
                    dragging = false
                    editing(false)
                })
        }
        .frame(height: 20)
        .accessibilityElement()
        .accessibilityValue(Text(value.formatted(.number.precision(.fractionLength(step < 1 ? 1 : 0)))))
        .accessibilityAdjustableAction { dir in
            let next = dir == .increment ? value + step : value - step
            value = min(max(next, range.lowerBound), range.upperBound)
            editing(true)
            editing(false)
        }
    }
}

/// Paints the window itself paper, title bar strip included, in either appearance.
struct WindowPaper: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let v = NSView()
        DispatchQueue.main.async {
            v.window?.backgroundColor = Ink.paper
        }
        return v
    }
    func updateNSView(_ nsView: NSView, context: Context) {}
}
