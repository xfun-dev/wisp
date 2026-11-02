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
        Fonts.register()
    }

    var body: some Scene {
        Window("Wisp", id: "settings") {
            SettingsView().frame(width: 460)
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
    }
}

@MainActor
final class Core: ObservableObject {
    @Published var status: Status?
    @Published var problem: String?
    private let wisp = NSHomeDirectory() + "/Library/Application Support/Wisp/wisp"
    private let job = "gui/\(getuid())/fun.xfun.wisp"
    private var timer: Timer?

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
                problem = reply.isEmpty ? "Wisp 没有在运行" : reply
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

    func quit() {
        Task {
            _ = await ctl(["quit"])
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

// MARK: - Look: a clay desk (RULES.md B3)
//
// The window is one small scene: a wall, a wooden desk with this Mac and the other
// computer on it, and the desk's drawer front holding the controls. Everything is
// matte clay and wood; the only light is the teal wisp between the two machines.

extension NSColor {
    convenience init(hex: UInt32, alpha: CGFloat = 1) {
        self.init(srgbRed: CGFloat(hex >> 16 & 0xFF) / 255, green: CGFloat(hex >> 8 & 0xFF) / 255,
                  blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
    }
}

extension Color {
    init(hex: UInt32, opacity: Double = 1) { self.init(nsColor: NSColor(hex: hex, alpha: opacity)) }
}

enum Clay {
    static let wall: UInt32 = 0xEDE5D8
    static let wood: UInt32 = 0xB8936A
    static let woodBand: UInt32 = 0xA88A66
    static let tray: UInt32 = 0xA47E54
    static let groove: UInt32 = 0x8E6A42
    static let engrave: UInt32 = 0x6E4F2E
    static let clay: UInt32 = 0xF3E9DA
    static let clayShade: UInt32 = 0xD9C7AE
    static let clayDrop: UInt32 = 0x8D704F
    static let teal: UInt32 = 0x3FA69A
    static let tealGlow: UInt32 = 0x5FD6C6
    static let cream: UInt32 = 0xFFF7EC
    static let ink: UInt32 = 0x4A3725
    static let inkSoft: UInt32 = 0x8C7660
    static let amber: UInt32 = 0xD49A2A
    static let red: UInt32 = 0xC0503A
}

extension Font {
    /// The rounded face (Fredoka, SIL OFL) for numbers and Latin labels; Chinese stays in the system face.
    static func round(_ size: CGFloat) -> Font { .custom("Fredoka", size: size).weight(.semibold) }
}

enum Fonts {
    static func register() {
        guard let url = Bundle.main.url(forResource: "Fredoka", withExtension: "ttf") else { return }
        CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
    }

    static func round(_ size: CGFloat) -> NSFont {
        let d = NSFontDescriptor(fontAttributes: [
            .family: "Fredoka",
            .traits: [NSFontDescriptor.TraitKey.weight: NSFont.Weight.semibold],
        ])
        return NSFont(descriptor: d, size: size) ?? .systemFont(ofSize: size, weight: .semibold)
    }
}

/// Engraved lettering on the wood: dark, with a light edge underneath.
struct Engraved: ViewModifier {
    func body(content: Content) -> some View {
        content
            .foregroundStyle(Color(hex: Clay.engrave))
            .shadow(color: Color(hex: 0xFFEBCD, opacity: 0.35), radius: 0, x: 0, y: 1)
    }
}

extension View {
    func engraved() -> some View { modifier(Engraved()) }
}

// MARK: - State

/// What the window is showing, derived from the core's status line.
enum Mood: Equatable {
    case connected, searching, unpaired, paused, pairing

    init(_ s: Status) {
        if !s.code.isEmpty { self = .pairing }
        else if s.paired.isEmpty { self = .unpaired }
        else if s.paused { self = .paused }
        else if s.connected { self = .connected }
        else { self = .searching }
    }

    var dot: UInt32 {
        switch self {
        case .connected: Clay.teal
        case .searching, .pairing: Clay.amber
        case .paused: Clay.red
        case .unpaired: Clay.inkSoft
        }
    }
}

/// Where the other computer stands on the desk, per layout.
struct Stage {
    let monitor: CGPoint, scale: CGFloat, plantX: CGFloat
    /// the wisp from this Mac's screen to the other one, as a cubic
    let p0: CGPoint, c1: CGPoint, c2: CGPoint, p1: CGPoint

    static func of(_ edge: String) -> Stage {
        switch edge {
        case "left":
            Stage(monitor: CGPoint(x: 10, y: 150), scale: 0.85, plantX: 412,
                  p0: CGPoint(x: 172, y: 284), c1: CGPoint(x: 148, y: 282), c2: CGPoint(x: 172, y: 226), p1: CGPoint(x: 150, y: 196))
        case "right":
            Stage(monitor: CGPoint(x: 307, y: 150), scale: 0.85, plantX: 40,
                  p0: CGPoint(x: 288, y: 284), c1: CGPoint(x: 312, y: 282), c2: CGPoint(x: 288, y: 226), p1: CGPoint(x: 310, y: 196))
        default:
            Stage(monitor: CGPoint(x: 146, y: 94), scale: 1, plantX: 412,
                  p0: CGPoint(x: 230, y: 244), c1: CGPoint(x: 206, y: 232), c2: CGPoint(x: 254, y: 214), p1: CGPoint(x: 230, y: 200))
        }
    }

    var trail: CGPath {
        let p = CGMutablePath()
        p.move(to: p0)
        p.addCurve(to: p1, control1: c1, control2: c2)
        return p
    }
}

// MARK: - The desk scene

/// The wall, the desk and the two machines. Core Animation layers built once; a change
/// of layout or state moves or recolours them, and the loops (the wisp, the steam) run
/// in the window server, so an open window costs this process almost no CPU.
struct DeskScene: NSViewRepresentable {
    let mood: Mood
    let edge: String
    let name: String

    func makeNSView(context: Context) -> DeskLayers { DeskLayers() }

    func updateNSView(_ view: DeskLayers, context: Context) {
        view.show(mood: mood, edge: edge, name: name)
        let side = ["left": "左侧", "right": "右侧"][edge] ?? "上方"
        view.setAccessibilityLabel("书桌上：这台 Mac，另一台电脑的屏幕在它的\(side)")
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: DeskLayers, context: Context) -> CGSize? {
        CGSize(width: 460, height: 430)
    }
}

final class DeskLayers: NSView {
    private let monitor = CALayer()
    private let monitorScreen = CAGradientLayer()
    private let monitorName = CATextLayer()
    private let plant = CALayer()
    private let armEdge = CAShapeLayer()
    private let armFace = CAShapeLayer()
    private let joints = CAShapeLayer()
    private let glow = CAShapeLayer()
    private let trail = CAShapeLayer()
    private let spark = CALayer()
    private var shown: (Mood, String, String)?

    override var isFlipped: Bool { true }
    /// the window has no title bar; the scene helps move it
    override var mouseDownCanMoveWindow: Bool { true }

    override init(frame: NSRect) {
        super.init(frame: frame)
        wantsLayer = true
        setAccessibilityElement(true)
        setAccessibilityRole(.image)
        build()
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    // MARK: drawing helpers

    private func c(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor { NSColor(hex: hex, alpha: alpha).cgColor }

    @discardableResult
    private func shape(_ path: CGPath, fill: CGColor?, stroke: CGColor? = nil, width: CGFloat = 1, in parent: CALayer) -> CAShapeLayer {
        let l = CAShapeLayer()
        l.path = path
        l.fillColor = fill
        l.strokeColor = stroke
        l.lineWidth = width
        l.lineCap = .round
        l.lineJoin = .round
        parent.addSublayer(l)
        return l
    }

    @discardableResult
    private func gradient(_ path: CGPath, _ colors: [UInt32], diagonal: Bool = false, horizontal: Bool = false,
                          into g: CAGradientLayer = CAGradientLayer(), in parent: CALayer) -> CAGradientLayer {
        let box = path.boundingBoxOfPath
        g.frame = box
        g.colors = colors.map { c($0) }
        g.startPoint = .zero
        g.endPoint = diagonal ? CGPoint(x: 1, y: 1) : horizontal ? CGPoint(x: 1, y: 0) : CGPoint(x: 0, y: 1)
        let mask = CAShapeLayer()
        var t = CGAffineTransform(translationX: -box.minX, y: -box.minY)
        mask.path = path.copy(using: &t)
        g.mask = mask
        parent.addSublayer(g)
        return g
    }

    /// A soft contact shadow under something standing on the desk.
    private func contact(_ rect: CGRect, opacity: Float, radius: CGFloat = 6, in parent: CALayer) {
        let l = CALayer()
        l.shadowPath = CGPath(ellipseIn: rect, transform: nil)
        l.shadowColor = c(0x8A6640)
        l.shadowOpacity = opacity
        l.shadowRadius = radius
        l.shadowOffset = .zero
        parent.addSublayer(l)
    }

    private func poly(_ pts: [(CGFloat, CGFloat)]) -> CGPath {
        let p = CGMutablePath()
        p.addLines(between: pts.map { CGPoint(x: $0.0, y: $0.1) })
        p.closeSubpath()
        return p
    }

    private func rr(_ x: CGFloat, _ y: CGFloat, _ w: CGFloat, _ h: CGFloat, _ r: CGFloat) -> CGPath {
        CGPath(roundedRect: CGRect(x: x, y: y, width: w, height: h), cornerWidth: r, cornerHeight: r, transform: nil)
    }

    private func forever(_ a: CAAnimation) -> CAAnimation {
        a.repeatCount = .infinity
        a.isRemovedOnCompletion = false
        return a
    }

    // MARK: the scene, back to front

    private func build() {
        guard let root = layer else { return }
        root.backgroundColor = c(Clay.wall)
        CATransaction.begin()
        CATransaction.setDisableActions(true)

        // window light on the wall, the skirting, the desk
        shape(poly([(300, 0), (420, 0), (360, 262), (250, 262)]), fill: c(0xF5EEE3, 0.7), in: root)
        shape(CGPath(rect: CGRect(x: 0, y: 252, width: 460, height: 10), transform: nil), fill: c(0xE3D8C8), in: root)
        gradient(poly([(34, 262), (426, 262), (460, 372), (0, 372)]), [0xD9BC95, 0xCDAA7E], in: root)
        let edge = CGMutablePath()
        edge.move(to: CGPoint(x: 34, y: 262)); edge.addLine(to: CGPoint(x: 426, y: 262))
        shape(edge, fill: nil, stroke: c(0xE9D3B2), width: 2, in: root)
        let grain = CGMutablePath()
        grain.move(to: CGPoint(x: 60, y: 290)); grain.addQuadCurve(to: CGPoint(x: 400, y: 292), control: CGPoint(x: 200, y: 284))
        grain.move(to: CGPoint(x: 20, y: 330)); grain.addQuadCurve(to: CGPoint(x: 440, y: 334), control: CGPoint(x: 180, y: 322))
        grain.move(to: CGPoint(x: 110, y: 352)); grain.addQuadCurve(to: CGPoint(x: 452, y: 356), control: CGPoint(x: 260, y: 346))
        shape(grain, fill: nil, stroke: c(0xC49C6E, 0.45), width: 1.2, in: root)
        shape(CGPath(rect: CGRect(x: 0, y: 372, width: 460, height: 58), transform: nil), fill: c(0xC29A6C), in: root)
        shape(CGPath(rect: CGRect(x: 0, y: 372, width: 460, height: 3), transform: nil), fill: c(0xE3C79F), in: root)
        shape(CGPath(rect: CGRect(x: 0, y: 424, width: 460, height: 6), transform: nil), fill: c(Clay.woodBand), in: root)

        buildPlant(root)
        buildArm(root)
        buildMonitor(root)
        buildMac(root)
        buildMug(root)

        // the wisp
        glow.fillColor = nil
        glow.strokeColor = c(Clay.tealGlow, 0.35)
        glow.lineWidth = 10
        glow.lineCap = .round
        glow.shadowColor = c(Clay.tealGlow)
        glow.shadowRadius = 6
        glow.shadowOpacity = 0.8
        glow.shadowOffset = .zero
        root.addSublayer(glow)
        trail.fillColor = nil
        trail.strokeColor = c(Clay.teal)
        trail.lineWidth = 3.5
        trail.lineCap = .round
        trail.lineDashPattern = [0.01, 9]
        let march = CABasicAnimation(keyPath: "lineDashPhase")
        march.fromValue = 0
        march.toValue = -9.01
        march.duration = 0.6
        trail.add(forever(march), forKey: "march")
        root.addSublayer(trail)
        spark.bounds = CGRect(x: 0, y: 0, width: 6, height: 6)
        spark.cornerRadius = 3
        spark.backgroundColor = c(0xFFFFFF)
        spark.shadowColor = c(0xE9FFFB)
        spark.shadowRadius = 5
        spark.shadowOpacity = 1
        spark.shadowOffset = .zero
        root.addSublayer(spark)
        CATransaction.commit()
    }

    private func buildPlant(_ root: CALayer) {
        plant.anchorPoint = .zero
        plant.bounds = CGRect(x: -40, y: 0, width: 80, height: 320)
        plant.position = CGPoint(x: 412 - 40, y: 0)
        contact(CGRect(x: -26, y: 295, width: 52, height: 10), opacity: 0.35, in: plant)
        let pot = CGMutablePath()
        pot.move(to: CGPoint(x: -18, y: 262)); pot.addLine(to: CGPoint(x: 18, y: 262)); pot.addLine(to: CGPoint(x: 14, y: 298))
        pot.addQuadCurve(to: CGPoint(x: -14, y: 298), control: CGPoint(x: 0, y: 302)); pot.closeSubpath()
        gradient(pot, [0xD98463, 0xB9654A], horizontal: true, in: plant)
        shape(rr(-20, 258, 40, 8, 3), fill: c(0xE39572), in: plant)
        let leaf = { (pts: [CGFloat], color: UInt32) in
            let p = CGMutablePath()
            p.move(to: CGPoint(x: 0, y: 258))
            p.addCurve(to: CGPoint(x: pts[4], y: pts[5]), control1: CGPoint(x: pts[0], y: pts[1]), control2: CGPoint(x: pts[2], y: pts[3]))
            p.addCurve(to: CGPoint(x: 0, y: 258), control1: CGPoint(x: pts[6], y: pts[7]), control2: CGPoint(x: pts[8], y: pts[9]))
            self.shape(p, fill: self.c(color), in: self.plant)
        }
        leaf([-4, 236, -26, 226, -30, 206, -12, 210, -2, 228], 0x6FAE7E)
        leaf([4, 230, 22, 218, 30, 196, 34, 218, 14, 236], 0x5C9C6C)
        leaf([-2, 228, 6, 206, 2, 184, 14, 204, 10, 232], 0x7DBE8A)
        root.addSublayer(plant)
    }

    /// The monitor arm: clamped to the back edge of the desk and never moving from
    /// there; its elbow bends to reach the monitor wherever it stands.
    static let armBase = CGPoint(x: 330, y: 248)
    static let upperArm: CGFloat = 135, forearm: CGFloat = 125

    /// Where the elbow sits for the monitor's mount point: two-link reach, elbow up.
    static func elbow(to mount: CGPoint) -> CGPoint {
        let b = armBase
        let dx = mount.x - b.x, dy = mount.y - b.y
        let d = min(max(hypot(dx, dy), abs(upperArm - forearm) + 1), upperArm + forearm - 1)
        let a = (upperArm * upperArm - forearm * forearm + d * d) / (2 * d)
        let h = sqrt(max(0, upperArm * upperArm - a * a))
        let ux = dx / hypot(dx, dy), uy = dy / hypot(dx, dy)
        let px = b.x + a * ux, py = b.y + a * uy
        let e1 = CGPoint(x: px - h * uy, y: py + h * ux), e2 = CGPoint(x: px + h * uy, y: py - h * ux)
        return e1.y < e2.y ? e1 : e2
    }

    static func armPath(_ stage: Stage) -> CGPath {
        let mount = mount(stage)
        let p = CGMutablePath()
        p.move(to: armBase)
        p.addLine(to: elbow(to: mount))
        p.addLine(to: mount)
        return p
    }

    private func buildArm(_ root: CALayer) {
        let b = Self.armBase
        contact(CGRect(x: b.x - 16, y: b.y + 18, width: 32, height: 6), opacity: 0.35, radius: 3, in: root)
        gradient(rr(b.x - 11, b.y + 6, 22, 16, 3), [0xD2D6DC, 0xAEB4BC], in: root)
        shape(rr(b.x - 4, b.y - 2, 8, 12, 2), fill: c(0xC7CCD2), in: root)
        for (l, w, color) in [(armEdge, CGFloat(9), UInt32(0xAEB4BC)), (armFace, CGFloat(6), UInt32(0xE2E5E9))] {
            l.fillColor = nil
            l.strokeColor = c(color)
            l.lineWidth = w
            l.lineCap = .round
            l.lineJoin = .round
            root.addSublayer(l)
        }
        // the two joints are part of the same kind of path as the arm, so they move as one
        joints.fillColor = c(0xD2D6DC)
        joints.strokeColor = c(0xAEB4BC)
        joints.lineWidth = 1.3
        root.addSublayer(joints)
    }

    static func jointPath(_ stage: Stage) -> CGPath {
        let p = CGMutablePath()
        for q in [armBase, elbow(to: mount(stage))] {
            p.addEllipse(in: CGRect(x: q.x - 5.5, y: q.y - 5.5, width: 11, height: 11))
        }
        return p
    }

    static func mount(_ stage: Stage) -> CGPoint {
        CGPoint(x: stage.monitor.x + 84 * stage.scale, y: stage.monitor.y + 52 * stage.scale)
    }

    private func buildMonitor(_ root: CALayer) {
        monitor.anchorPoint = .zero
        monitor.bounds = CGRect(x: 0, y: 0, width: 168, height: 110)
        monitor.position = CGPoint(x: 146, y: 94)
        // held from behind by the desk arm (buildArm)
        let hang = CALayer()
        hang.shadowPath = CGPath(roundedRect: CGRect(x: 6, y: 10, width: 156, height: 96), cornerWidth: 10, cornerHeight: 10, transform: nil)
        hang.shadowColor = c(0x6B5236)
        hang.shadowOpacity = 0.28
        hang.shadowRadius = 10
        hang.shadowOffset = CGSize(width: 0, height: 6)
        monitor.addSublayer(hang)
        gradient(rr(0, 0, 168, 104, 10), [0xEEF0F3, 0xD2D6DC], in: monitor)
        shape(rr(5, 5, 158, 94, 6), fill: c(0x1F242B), in: monitor)
        gradient(rr(9, 9, 150, 86, 3), [0x8FE0D3, 0x2F8F84], into: monitorScreen, in: monitor)
        let hills = CGMutablePath()
        hills.move(to: CGPoint(x: 9, y: 72))
        hills.addQuadCurve(to: CGPoint(x: 90, y: 70), control: CGPoint(x: 50, y: 58))
        hills.addQuadCurve(to: CGPoint(x: 159, y: 64), control: CGPoint(x: 130, y: 82))
        hills.addLine(to: CGPoint(x: 159, y: 95)); hills.addLine(to: CGPoint(x: 9, y: 95)); hills.closeSubpath()
        shape(hills, fill: c(0xFFFFFF, 0.16), in: monitor)
        shape(CGPath(rect: CGRect(x: 9, y: 87, width: 150, height: 8), transform: nil), fill: c(0x000000, 0.22), in: monitor)
        shape(rr(78, 89, 12, 4, 1), fill: c(0xFFFFFF, 0.7), in: monitor)
        let shine = CGMutablePath()
        shine.move(to: CGPoint(x: 12, y: 12)); shine.addLine(to: CGPoint(x: 60, y: 12))
        shape(shine, fill: nil, stroke: c(0xFFFFFF, 0.25), width: 2, in: monitor)
        monitorName.frame = CGRect(x: 9, y: 34, width: 150, height: 24)
        monitorName.alignmentMode = .center
        monitorName.font = Fonts.round(18)
        monitorName.fontSize = 18
        monitorName.foregroundColor = c(0xFFFFFF, 0.95)
        monitorName.contentsScale = 2
        monitorName.truncationMode = .end
        monitor.addSublayer(monitorName)
        root.addSublayer(monitor)
    }

    private func buildMac(_ root: CALayer) {
        contact(CGRect(x: 140, y: 353, width: 180, height: 14), opacity: 0.4, in: root)
        gradient(rr(166, 234, 128, 96, 9), [0xEEF0F3, 0xD2D6DC], in: root)
        shape(rr(170, 238, 120, 88, 6), fill: c(0x1F242B), in: root)
        gradient(rr(174, 243, 112, 79, 3), [0xF2C9A0, 0xC98D7A], diagonal: true, in: root)
        shape(CGPath(rect: CGRect(x: 174, y: 243, width: 112, height: 5), transform: nil), fill: c(0xFFFFFF, 0.35), in: root)
        let wave = CGMutablePath()
        wave.move(to: CGPoint(x: 174, y: 300))
        wave.addQuadCurve(to: CGPoint(x: 240, y: 296), control: CGPoint(x: 210, y: 282))
        wave.addQuadCurve(to: CGPoint(x: 286, y: 290), control: CGPoint(x: 270, y: 306))
        wave.addLine(to: CGPoint(x: 286, y: 322)); wave.addLine(to: CGPoint(x: 174, y: 322)); wave.closeSubpath()
        shape(wave, fill: c(0xFFFFFF, 0.18), in: root)
        shape(CGPath(ellipseIn: CGRect(x: 228.8, y: 239.3, width: 2.4, height: 2.4), transform: nil), fill: c(0x55606C), in: root)

        // the base: as wide as the lid at the hinge, a little wider at the front
        let (top, bot): (CGFloat, CGFloat) = (330, 354)
        let (tl, tr, bl, br): (CGFloat, CGFloat, CGFloat, CGFloat) = (166, 294, 152, 308)
        func edge(_ y: CGFloat) -> (CGFloat, CGFloat) {
            let t = (y - top) / (bot - top)
            return (tl + (bl - tl) * t, tr + (br - tr) * t)
        }
        gradient(poly([(tl, top), (tr, top), (br, bot), (bl, bot)]), [0xE2E5E9, 0xC7CCD2], in: root)
        shape(poly([(bl, bot), (br, bot), (br - 2.5, bot + 4.5), (bl + 2.5, bot + 4.5)]), fill: c(0xAEB4BC), in: root)
        let m: CGFloat = 8.5
        let (w0, w1): (CGFloat, CGFloat) = (331.4, 343.6)
        let (l0, r0) = edge(w0), (l1, r1) = edge(w1)
        shape(poly([(l0 + m - 1, w0), (r0 - m + 1, w0), (r1 - m + 1, w1), (l1 + m - 1, w1)]), fill: c(0xB3B9C1), in: root)
        let rows: [(CGFloat, CGFloat, [CGFloat])] = [
            (332.1, 2.2, Array(repeating: 1, count: 13)),
            (334.9, 2.4, [1.4] + Array(repeating: 1, count: 11) + [1.4]),
            (337.9, 2.5, [1.8] + Array(repeating: 1, count: 10) + [1.8]),
            (341.0, 2.5, [1.2, 1, 1.2, 6.2, 1.2, 1, 1.2]),
        ]
        let keys = CGMutablePath(), tops = CGMutablePath()
        for (y, h, units) in rows {
            let (la, ra) = edge(y), (lb, rb) = edge(y + h)
            let l = (la + lb) / 2 + m, r = (ra + rb) / 2 - m
            let gap: CGFloat = 0.8
            let unit = (r - l - gap * CGFloat(units.count - 1)) / units.reduce(0, +)
            var x = l
            for u in units {
                let w = unit * u
                keys.addRoundedRect(in: CGRect(x: x, y: y, width: w, height: h), cornerWidth: 0.6, cornerHeight: 0.6)
                tops.addRect(CGRect(x: x + 0.3, y: y, width: w - 0.6, height: 0.55))
                x += w + gap
            }
        }
        shape(keys, fill: c(0x2E333A), in: root)
        shape(tops, fill: c(0xFFFFFF, 0.14), in: root)
        shape(poly([(210, 345.2), (250, 345.2), (252, 352.4), (208, 352.4)]), fill: c(0xD5D9DE), stroke: c(0xB9BEC5), width: 0.6, in: root)
    }

    private func buildMug(_ root: CALayer) {
        let steam = CAShapeLayer()
        let s = CGMutablePath()
        s.move(to: CGPoint(x: 92, y: 318)); s.addCurve(to: CGPoint(x: 92, y: 292), control1: CGPoint(x: 86, y: 308), control2: CGPoint(x: 98, y: 302))
        s.move(to: CGPoint(x: 102, y: 316)); s.addCurve(to: CGPoint(x: 102, y: 290), control1: CGPoint(x: 96, y: 306), control2: CGPoint(x: 108, y: 300))
        steam.path = s
        steam.fillColor = nil
        steam.strokeColor = c(0xFFFFFF, 0.7)
        steam.lineWidth = 2.5
        steam.lineCap = .round
        let rise = CABasicAnimation(keyPath: "transform.translation.y")
        rise.fromValue = 0
        rise.toValue = -5
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 0.9
        fade.toValue = 0.35
        let both = CAAnimationGroup()
        both.animations = [rise, fade]
        both.duration = 1.5
        both.autoreverses = true
        both.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        steam.add(forever(both), forKey: "steam")
        root.addSublayer(steam)
        contact(CGRect(x: 75, y: 351, width: 44, height: 10), opacity: 0.35, in: root)
        let cup = CGMutablePath()
        cup.move(to: CGPoint(x: 80, y: 324)); cup.addLine(to: CGPoint(x: 114, y: 324)); cup.addLine(to: CGPoint(x: 111, y: 354))
        cup.addQuadCurve(to: CGPoint(x: 83, y: 354), control: CGPoint(x: 97, y: 358)); cup.closeSubpath()
        shape(cup, fill: c(Clay.clay), in: root)
        shape(CGPath(ellipseIn: CGRect(x: 80, y: 320, width: 34, height: 8), transform: nil), fill: c(0x7A5236), in: root)
        let handle = CGMutablePath()
        handle.move(to: CGPoint(x: 113, y: 330)); handle.addCurve(to: CGPoint(x: 111, y: 346), control1: CGPoint(x: 124, y: 330), control2: CGPoint(x: 124, y: 346))
        shape(handle, fill: nil, stroke: c(Clay.clay), width: 4, in: root)
    }

    // MARK: state

    func show(mood: Mood, edge: String, name: String) {
        let first = shown == nil
        if let shown, shown == (mood, edge, name) { return }
        let movedEdge = shown?.1 != edge
        shown = (mood, edge, name)
        let stage = Stage.of(edge)

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        monitorName.string = name.isEmpty ? "?" : name
        let screen: [UInt32] = switch mood {
        case .connected: [0x8FE0D3, 0x2F8F84]
        case .paused: [0xC9BFB4, 0x8E8276]
        default: [0xF5D79A, 0xC9923A]
        }
        monitorScreen.colors = screen.map { c($0) }
        CATransaction.commit()

        // the other computer: faint until paired, gone while the code is up
        let opacity: Float = switch mood {
        case .pairing: 0
        case .unpaired: 0.3
        default: 1
        }
        fade(monitor, to: opacity)

        let target = CATransform3DMakeScale(stage.scale, stage.scale, 1)
        if first || !movedEdge {
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            monitor.position = stage.monitor
            monitor.transform = target
            plant.position = CGPoint(x: stage.plantX - 40, y: 0)
            armEdge.path = Self.armPath(stage)
            armFace.path = armEdge.path
            joints.path = Self.jointPath(stage)
            CATransaction.commit()
        } else {
            // one curve for all of it, with no overshoot: a path cannot overshoot, so a
            // springy screen would come loose from the arm holding it
            for (l, path) in [(armEdge, Self.armPath(stage)), (armFace, Self.armPath(stage)), (joints, Self.jointPath(stage))] {
                glide(l, "path", from: l.presentation()?.path ?? l.path as Any, to: path)
                l.path = path
            }
            glide(monitor, "position", from: monitor.presentation()?.position ?? monitor.position, to: stage.monitor)
            glide(monitor, "transform", from: monitor.presentation()?.transform ?? monitor.transform, to: target)
            glide(plant, "position", from: plant.presentation()?.position ?? plant.position, to: CGPoint(x: stage.plantX - 40, y: 0))
            monitor.position = stage.monitor
            monitor.transform = target
            plant.position = CGPoint(x: stage.plantX - 40, y: 0)
        }

        // the wisp only while connected
        let live = mood == .connected
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        if movedEdge && !first {
            for l in [glow, trail] {
                glide(l, "path", from: l.presentation()?.path ?? l.path as Any, to: stage.trail)
            }
        }
        glow.path = stage.trail
        trail.path = stage.trail
        CATransaction.commit()
        fade(glow, to: live ? 1 : 0)
        fade(trail, to: live ? 1 : 0)
        fade(spark, to: live ? 1 : 0)
        let ride = CAKeyframeAnimation(keyPath: "position")
        ride.path = stage.trail
        ride.calculationMode = .paced
        ride.duration = 1.8
        spark.add(forever(ride), forKey: "ride")
    }

    private func fade(_ l: CALayer, to opacity: Float) {
        guard l.opacity != opacity else { return }
        let a = CABasicAnimation(keyPath: "opacity")
        a.fromValue = l.presentation()?.opacity ?? l.opacity
        a.toValue = opacity
        a.duration = 0.35
        l.add(a, forKey: "fade")
        l.opacity = opacity
    }

    private func glide(_ l: CALayer, _ key: String, from: Any, to: Any) {
        let a = CABasicAnimation(keyPath: key)
        a.fromValue = from
        a.toValue = to
        a.duration = 0.6
        a.timingFunction = CAMediaTimingFunction(controlPoints: 0.22, 1, 0.36, 1)
        l.add(a, forKey: key)
    }
}

// MARK: - On the wall

/// The sign on the wall: what is going on, or the pairing code as six clay keycaps.
struct WallSign: View {
    let s: Status?
    let problem: String?
    @State private var dropped = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Circle()
                    .fill(Color(hex: mood?.dot ?? Clay.inkSoft))
                    .frame(width: 10, height: 10)
                    .background(Circle().fill(Color(hex: mood?.dot ?? Clay.inkSoft, opacity: 0.2)).frame(width: 18, height: 18))
                Text(title).font(.system(size: 18, weight: .semibold)).foregroundStyle(Color(hex: Clay.ink))
            }
            if let s, mood == .pairing {
                code(s)
            } else {
                Text(detail)
                    .font(.system(size: 12))
                    .foregroundStyle(problem == nil ? Color(hex: Clay.inkSoft) : Color(hex: Clay.red))
                    .padding(.leading, 20)
                    .frame(maxWidth: 260, alignment: .leading)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .animation(.easeOut(duration: 0.3), value: mood)
    }

    private var mood: Mood? { s.map(Mood.init) }

    private var title: String {
        guard let s, let mood else { return problem == nil ? "正在连接 Wisp…" : "Wisp 没有在运行" }
        return switch mood {
        case .connected: "已连接 \(s.paired)"
        case .searching: "等待 \(s.paired) 连接"
        case .unpaired: "还没有配对"
        case .paused: "已暂停切屏"
        case .pairing: "配对码"
        }
    }

    private var detail: String {
        if let problem { return problem }
        guard let s, let mood else { return "" }
        return switch mood {
        case .connected: "\(s.peer) · \(s.screen)"
        case .searching: "确认那台电脑上的 Wisp 在运行，并且在同一个局域网"
        case .unpaired: "点下面的“配对新电脑”，再到 Windows 上的 Wisp 里输入配对码"
        case .paused: "光标不会切到 \(s.paired)；Wisp 重启后自动恢复"
        case .pairing: ""
        }
    }

    private func code(_ s: Status) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 7) {
                ForEach(Array(s.code.enumerated()), id: \.offset) { i, ch in
                    Text(String(ch))
                        .font(.round(24))
                        .foregroundStyle(Color(hex: Clay.ink))
                        .frame(width: 38, height: 44)
                        .background(
                            RoundedRectangle(cornerRadius: 10, style: .continuous)
                                .fill(Color(hex: Clay.clay).shadow(.inner(color: Color(hex: Clay.clayShade), radius: 0, y: -5)))
                                .shadow(color: Color(hex: Clay.clayDrop), radius: 0, y: 5)
                                .shadow(color: Color(hex: 0x462D14, opacity: 0.25), radius: 7, y: 9)
                        )
                        .padding(.leading, i == 3 ? 10 : 0)
                        .offset(y: dropped ? 0 : -22)
                        .opacity(dropped ? 1 : 0)
                        .animation(.spring(response: 0.45, dampingFraction: 0.55).delay(Double(i) * 0.07), value: dropped)
                }
            }
            .accessibilityElement()
            .accessibilityLabel(Text("配对码 " + s.code.map(String.init).joined(separator: " ")))
            Text("在 Windows 上的 Wisp 里输入 · 还剩 \(s.codeSecondsLeft / 60):\(String(format: "%02d", s.codeSecondsLeft % 60)) · 还可以输错 \(s.codeTriesLeft) 次")
                .font(.system(size: 12))
                .monospacedDigit()
                .foregroundStyle(Color(hex: Clay.inkSoft))
        }
        .padding(.top, 6)
        .onAppear { dropped = true }
        .onChange(of: s.code) { _, _ in
            dropped = false
            DispatchQueue.main.async { dropped = true }
        }
    }
}

// MARK: - The drawer front

/// Where the other screen stands, picked in a tray carved into the drawer: a tiny Mac
/// in the middle and three places around it; the tiny monitor springs to the one picked.
struct LayoutTray: View {
    let edge: String
    let pick: (String) -> Void
    @Namespace private var space

    private let slots: [(id: String, word: String, x: CGFloat, y: CGFloat)] = [
        ("top", "上方", 158, 6), ("left", "左侧", 30, 40), ("right", "右侧", 286, 40),
    ]

    var body: some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(Color(hex: Clay.tray).shadow(.inner(color: Color(hex: 0x3C230A, opacity: 0.45), radius: 5, y: 5)))
                .shadow(color: Color(hex: 0xFFEBCD, opacity: 0.25), radius: 0, y: 1)
            MiniMac().offset(x: 206 - 38, y: 62)
            ForEach(slots, id: \.id) { slot in
                Slot(word: slot.word, on: edge == slot.id, space: space) { pick(slot.id) }
                    .offset(x: slot.x, y: slot.y)
            }
        }
        .frame(height: 124)
        .animation(.spring(response: 0.45, dampingFraction: 0.62), value: edge)
    }

    private struct Slot: View {
        let word: String
        let on: Bool
        let space: Namespace.ID
        let pick: () -> Void
        @State private var hover = false

        var body: some View {
            Button(action: pick) {
                VStack(spacing: 4) {
                    ZStack {
                        RoundedRectangle(cornerRadius: 7, style: .continuous)
                            .strokeBorder(Color(hex: 0xFFF0D7, opacity: hover ? 0.85 : 0.45), style: StrokeStyle(lineWidth: 2, dash: [5, 4]))
                            .offset(y: hover && !on ? -2 : 0)
                        if on {
                            ClayMonitor().matchedGeometryEffect(id: "monitor", in: space)
                        }
                    }
                    .frame(width: 52, height: 34)
                    Text(word).font(.system(size: 11, weight: .semibold)).foregroundStyle(Color(hex: 0xF6EBDD, opacity: 0.85))
                }
                .frame(width: 96, height: 58)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .onHover { h in withAnimation(.easeOut(duration: 0.15)) { hover = h } }
            .accessibilityLabel(Text("另一台屏幕在\(word)"))
            .accessibilityAddTraits(on ? .isSelected : [])
        }
    }
}

struct ClayMonitor: View {
    var body: some View {
        RoundedRectangle(cornerRadius: 7, style: .continuous)
            .fill(Color(hex: 0xE3E6EA))
            .overlay(RoundedRectangle(cornerRadius: 4, style: .continuous).fill(Color(hex: Clay.teal)).padding(4))
            .shadow(color: Color(hex: 0xA9AEB5), radius: 0, y: 5)
            .shadow(color: Color(hex: 0x321C08, opacity: 0.35), radius: 5, y: 8)
            .frame(width: 52, height: 34)
    }
}

struct MiniMac: View {
    var body: some View {
        ZStack(alignment: .top) {
            Ellipse().fill(Color(hex: 0x7E5D38, opacity: 0.55)).frame(width: 96, height: 12).blur(radius: 2).offset(y: 37)
            RoundedRectangle(cornerRadius: 6, style: .continuous).fill(Color(hex: 0xE3E6EA)).frame(width: 56, height: 36)
                .overlay(
                    RoundedRectangle(cornerRadius: 3, style: .continuous).fill(Color(hex: 0x36404D)).padding(4)
                        .overlay(Text("Mac").font(.round(9)).foregroundStyle(.white.opacity(0.8)))
                )
            Path { p in
                p.move(to: CGPoint(x: 0, y: 0)); p.addLine(to: CGPoint(x: 76, y: 0))
                p.addLine(to: CGPoint(x: 72, y: 8)); p.addLine(to: CGPoint(x: 4, y: 8)); p.closeSubpath()
            }
            .fill(Color(hex: 0xD3D7DC))
            .frame(width: 76, height: 8)
            .offset(y: 36)
        }
        .frame(width: 76, height: 46)
        .accessibilityHidden(true)
    }
}

/// A groove carved into the wood with a clay bead riding in it.
struct GrooveSlider: View {
    @Binding var value: Double
    let range: ClosedRange<Double>
    let step: Double
    let editing: (Bool) -> Void
    @State private var dragging = false

    var body: some View {
        GeometryReader { g in
            let w = g.size.width
            let t = (value - range.lowerBound) / (range.upperBound - range.lowerBound)
            ZStack(alignment: .topLeading) {
                Capsule()
                    .fill(Color(hex: Clay.groove).shadow(.inner(color: Color(hex: 0x321C08, opacity: 0.55), radius: 3, y: 3)))
                    .shadow(color: Color(hex: 0xFFEBCD, opacity: 0.3), radius: 0, y: 1)
                    .frame(height: 12)
                    .offset(y: 9)
                Capsule()
                    .fill(Color(hex: Clay.tealGlow))
                    .shadow(color: Color(hex: Clay.tealGlow, opacity: 0.8), radius: 4)
                    .frame(width: max(0, (w - 6) * t), height: 6)
                    .offset(x: 3, y: 12)
                ForEach(0..<5) { i in
                    Capsule().fill(Color(hex: Clay.groove, opacity: 0.7))
                        .frame(width: 2, height: 5)
                        .offset(x: 14 + (w - 28) * CGFloat(i) / 4, y: 26)
                }
                Capsule()
                    .fill(Color(hex: Clay.clay).shadow(.inner(color: Color(hex: Clay.clayShade), radius: 0, y: -4)))
                    .overlay(Capsule().fill(.white.opacity(0.7)).frame(height: 2).padding(.horizontal, 7).offset(y: -9))
                    .shadow(color: Color(hex: Clay.clayDrop), radius: 0, y: 4)
                    .shadow(color: Color(hex: 0x321C08, opacity: 0.35), radius: 5, y: 7)
                    .frame(width: 30, height: 26)
                    .scaleEffect(dragging ? 1.08 : 1)
                    .offset(x: (w - 30) * t, y: 2)
                    .animation(dragging ? nil : .spring(response: 0.3, dampingFraction: 0.6), value: value)
                    .animation(.spring(response: 0.25, dampingFraction: 0.6), value: dragging)
            }
            .frame(width: w, height: 34, alignment: .topLeading)
            .contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0)
                .onChanged { d in
                    if !dragging { dragging = true; editing(true) }
                    let x = min(max(0, (d.location.x - 15) / max(w - 30, 1)), 1)
                    let raw = range.lowerBound + x * (range.upperBound - range.lowerBound)
                    value = min(max((raw / step).rounded() * step, range.lowerBound), range.upperBound)
                }
                .onEnded { _ in
                    dragging = false
                    editing(false)
                })
        }
        .frame(height: 34)
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

/// A clay knob in a carved channel.
struct ClaySwitch: View {
    let on: Bool
    let flip: () -> Void

    var body: some View {
        Button(action: flip) {
            ZStack(alignment: on ? .trailing : .leading) {
                Capsule().fill(Color(hex: on ? Clay.red : Clay.clayDrop).shadow(.inner(color: Color(hex: 0x321C08, opacity: 0.5), radius: 3, y: 3)))
                Circle()
                    .fill(Color(hex: Clay.clay).shadow(.inner(color: Color(hex: Clay.clayShade), radius: 0, y: -3)))
                    .shadow(color: Color(hex: Clay.clayDrop), radius: 0, y: 3)
                    .frame(width: 26, height: 26)
                    .padding(3)
            }
            .frame(width: 64, height: 32)
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .animation(.spring(response: 0.35, dampingFraction: 0.6), value: on)
        .accessibilityValue(Text(on ? "开" : "关"))
    }
}

/// Plain engraved words that act as buttons.
struct CarvedLink: View {
    let title: String
    let act: () -> Void
    @State private var hover = false

    var body: some View {
        Button(action: act) {
            Text(title).font(.system(size: 12, weight: .semibold))
                .foregroundStyle(Color(hex: hover ? 0x3E2A14 : Clay.engrave))
                .shadow(color: Color(hex: 0xFFEBCD, opacity: 0.35), radius: 0, x: 0, y: 1)
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
    }
}

/// Paints the window itself the wall colour, title bar strip included.
struct WindowWall: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let v = NSView()
        DispatchQueue.main.async {
            v.window?.backgroundColor = NSColor(hex: Clay.wall)
            v.window?.appearance = NSAppearance(named: .aqua)
        }
        return v
    }
    func updateNSView(_ nsView: NSView, context: Context) {}
}

// MARK: - Window

struct SettingsView: View {
    @StateObject private var core = Core()
    @State private var speed = 1.0
    @State private var scroll = 3.0
    @State private var dragging = false
    @State private var confirmUnpair = false
    @State private var menuIcon = Installer.menuEnabled

    var body: some View {
        let s = core.status
        VStack(spacing: 0) {
            ZStack(alignment: .topLeading) {
                DeskScene(mood: s.map(Mood.init) ?? .searching, edge: s?.edge ?? "top", name: s?.paired ?? "")
                WallSign(s: s, problem: core.problem)
                    .padding(.leading, 24)
                    .padding(.top, 16)
            }
            .frame(width: 460, height: 430)
            drawer(s)
                .frame(width: 460, height: 370)
        }
        .background(Color(hex: Clay.wall))
        .background(WindowWall())
        .overlay(alignment: .top) {
            Text("wisp").font(.round(13)).tracking(1).foregroundStyle(Color(hex: 0x9A8167))
                .offset(y: -22)
                .accessibilityHidden(true)
        }
        .environment(\.colorScheme, .light)
        .onChange(of: core.status) { _, s in
            guard let s, !dragging else { return }
            speed = s.speed
            scroll = s.scroll
        }
    }

    private func drawer(_ s: Status?) -> some View {
        ZStack(alignment: .topLeading) {
            Color(hex: Clay.wood)
            Color(hex: Clay.woodBand).frame(height: 10)
            VStack(alignment: .leading, spacing: 0) {
                Text("屏幕在哪一边").font(.system(size: 12, weight: .semibold)).tracking(2).engraved()
                    .padding(.top, 22)
                LayoutTray(edge: s?.edge ?? "top") { core.send(["set", "edge=\($0)"]) }
                    .padding(.top, 10)
                VStack(spacing: 18) {
                    slider("鼠标速度", value: $speed, range: 0.2...5, step: 0.1, key: "speed")
                    slider("滚动速度", value: $scroll, range: 0.5...10, step: 0.5, key: "scroll")
                }
                .padding(.top, 22)
                Spacer(minLength: 0)
                HStack(spacing: 14) {
                    Text("菜单栏图标").font(.system(size: 12, weight: .semibold)).tracking(2).engraved()
                        .frame(width: 70, alignment: .leading)
                    ClaySwitch(on: menuIcon) {
                        menuIcon.toggle()
                        let on = menuIcon
                        Task.detached { Installer.setMenu(on) }
                    }
                    .accessibilityLabel(Text("在菜单栏显示图标"))
                    Text("约多占 10 MB 内存").font(.system(size: 11)).foregroundStyle(Color(hex: Clay.engrave, opacity: 0.75))
                    Spacer()
                }
                .padding(.bottom, 12)
                HStack(spacing: 14) {
                    Text("暂停切屏").font(.system(size: 12, weight: .semibold)).tracking(2).engraved()
                        .frame(width: 70, alignment: .leading)
                    ClaySwitch(on: s?.paused ?? false) { core.send([(s?.paused ?? false) ? "resume" : "pause"]) }
                        .accessibilityLabel(Text("暂停切屏"))
                    Spacer()
                    footer(s)
                }
                .padding(.bottom, 24)
            }
            .padding(.horizontal, 24)
            .disabled(s == nil)
            .opacity(s == nil ? 0.55 : 1)
        }
    }

    /// RULES.md P1, P4, P5, P8: start a code, cancel it, unpair; and quit (U11).
    @ViewBuilder private func footer(_ s: Status?) -> some View {
        if let s, !s.code.isEmpty {
            CarvedLink(title: "取消配对码") { core.send(["pair-cancel"]) }
        } else if let s, s.paired.isEmpty {
            CarvedLink(title: "配对新电脑") { core.send(["pair"]) }
        } else if let s {
            CarvedLink(title: "换一台电脑") { core.send(["pair"]) }
            CarvedLink(title: "解除配对") { confirmUnpair = true }
                .confirmationDialog("解除与 \(s.paired) 的配对？", isPresented: $confirmUnpair) {
                    Button("解除配对", role: .destructive) { core.send(["unpair"]) }
                } message: {
                    Text("这台 Mac 会断开与它的连接，以后要再用就得重新配对。")
                }
        }
        CarvedLink(title: "退出") { core.quit() }
    }

    private func slider(_ title: String, value: Binding<Double>, range: ClosedRange<Double>, step: Double, key: String) -> some View {
        HStack(spacing: 14) {
            Text(title).font(.system(size: 12, weight: .semibold)).tracking(2).engraved()
                .frame(width: 70, alignment: .leading)
            GrooveSlider(value: value, range: range, step: step) { editing in
                dragging = editing
                if !editing { core.send(["set", "\(key)=\(value.wrappedValue)"]) }
            }
            .accessibilityLabel(Text(title))
            Text(value.wrappedValue.formatted(.number.precision(key == "speed" ? .fractionLength(1) : .fractionLength(0...1))))
                .font(.round(15))
                .foregroundStyle(Color(hex: Clay.cream))
                .frame(width: 46, height: 28)
                .background(RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(Color(hex: Clay.tray).shadow(.inner(color: Color(hex: 0x321C08, opacity: 0.45), radius: 2, y: 2))))
        }
    }
}
