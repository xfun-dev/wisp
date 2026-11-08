//! macOS side: owns the physical keyboard/trackpad. Two HID-level event taps
//! (RULES.md S1–S4): a listen-only `watch` tap sees mouse moves while the cursor is
//! on the Mac and can never hold up input; once the cursor is pushed past an edge,
//! the `grab` tap is switched on and swallows every event, forwarding it to the
//! client, until the cursor comes back. A watchdog thread switches `grab` off and
//! thaws the cursor if this thread stops answering while Windows has control.

use crate::control::{self, Live};
use crate::discovery;
use crate::keymap::{self, Change, SystemKey};
use crate::logic::capture::{self, Hop, Pinch, Rect, Switcher};
use crate::pasteboard_mac::{self as pasteboard, Job};
use crate::proto::{self, Msg};
use std::ffi::c_void;
use std::convert::Infallible;
use std::io;
use crate::pair;
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream, UdpSocket};
use std::ptr;
use std::sync::mpsc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFMachPortRef = *mut c_void;
type CGEventRef = *mut c_void;
type TapCallback = extern "C" fn(*mut c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;
type TimerCallback = extern "C" fn(*mut c_void, *mut c_void);

#[repr(C)]
struct TimerContext {
    version: isize,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    describe: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[repr(C)]
struct Opaque {
    _p: [u8; 0],
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, cb: TapCallback, refcon: *mut c_void) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetLocation(e: CGEventRef) -> CGPoint;
    fn CGEventGetIntegerValueField(e: CGEventRef, field: u32) -> i64;
    fn CGEventGetFlags(e: CGEventRef) -> u64;
    fn CGEventGetDoubleValueField(e: CGEventRef, field: u32) -> f64;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGWarpMouseCursorPosition(p: CGPoint) -> i32;
    fn CGAssociateMouseAndMouseCursorPosition(connected: u32) -> i32;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
    fn CGCursorIsVisible() -> u32;
    fn CGSetLocalEventsSuppressionInterval(seconds: f64) -> i32;
    fn _CGSDefaultConnection() -> i32;
    fn CGSSetConnectionProperty(cid: i32, target: i32, key: CFStringRef, value: CFTypeRef) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: Opaque;
    static kCFTypeDictionaryValueCallBacks: Opaque;
    fn CFMachPortCreateRunLoopSource(alloc: CFTypeRef, port: CFMachPortRef, order: isize) -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: CFStringRef);
    fn CFRunLoopRun();
    fn CFAbsoluteTimeGetCurrent() -> f64;
    fn CFRunLoopTimerCreate(
        alloc: CFTypeRef,
        fire: f64,
        interval: f64,
        flags: u64,
        order: isize,
        cb: TimerCallback,
        ctx: *mut TimerContext,
    ) -> *mut c_void;
    fn CFRunLoopAddTimer(rl: *mut c_void, timer: *mut c_void, mode: CFStringRef);
    fn CFRunLoopTimerSetNextFireDate(timer: *mut c_void, fire: f64);
    fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const u8, encoding: u32) -> CFStringRef;
    fn CFDictionaryCreate(
        alloc: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        n: isize,
        key_cb: *const Opaque,
        value_cb: *const Opaque,
    ) -> CFTypeRef;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    static kAXTrustedCheckOptionPrompt: CFStringRef;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
}

// CGEventType
const LEFT_DOWN: u32 = 1;
const LEFT_UP: u32 = 2;
const RIGHT_DOWN: u32 = 3;
const RIGHT_UP: u32 = 4;
const MOUSE_MOVED: u32 = 5;
const LEFT_DRAGGED: u32 = 6;
const RIGHT_DRAGGED: u32 = 7;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const SCROLL: u32 = 22;
const OTHER_DOWN: u32 = 25;
const OTHER_UP: u32 = 26;
const OTHER_DRAGGED: u32 = 27;
// trackpad gestures (undocumented NSEventTypeGesture / NSEventTypeMagnify)
const GESTURE: u32 = 29;
const MAGNIFY: u32 = 30;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER: u32 = 0xFFFF_FFFF;

// CGEventField
const BUTTON_NUMBER: u32 = 3;
const DELTA_X: u32 = 4;
const DELTA_Y: u32 = 5;
const KEYCODE: u32 = 9;
const AUTOREPEAT: u32 = 8;
// NX_SYSDEFINED (the top row's media keys): its subtype and data1, readable as plain
// integer fields, so the core needs no AppKit (measured: fields 99 and 149)
const SYS_DEFINED: u32 = 14;
const SYS_SUBTYPE: u32 = 99;
const SYS_DATA1: u32 = 149;
const SCROLL_LINES_Y: u32 = 11;
const SCROLL_LINES_X: u32 = 12;
const SCROLL_CONTINUOUS: u32 = 88;
const SCROLL_POINTS_Y: u32 = 96;
const SCROLL_POINTS_X: u32 = 97;
// undocumented gesture fields
const GESTURE_HID_TYPE: u32 = 110;
const GESTURE_ZOOM_VALUE: u32 = 113;
const GESTURE_PHASE: u32 = 132;

const HID_TYPE_ZOOM: i64 = 8;
const PHASE_ENDED: i64 = 8;
const PHASE_CANCELLED: i64 = 16;

#[derive(Clone)]
struct Link {
    id: u64,
    peer: String,
    tx: proto::Queue,
    w: f64,
    h: f64,
    /// the connection itself, so unpairing can cut it
    closer: Arc<TcpStream>,
}

// kCGHIDEventTap, kCGHeadInsertEventTap; kCGEventTapOptionDefault / ListenOnly
const HID: u32 = 0;
const HEAD: u32 = 0;
const ACTIVE: u32 = 0;
const LISTEN_ONLY: u32 = 1;

/// How often the main thread proves it is alive, and how long the watchdog lets it
/// stay silent while Windows has control before taking input back (RULES.md S3).
/// Both sleep while the cursor is on the Mac, where nothing can hold up input.
const BEAT: Duration = Duration::from_millis(500);
const STALL_LIMIT: Duration = Duration::from_millis(1500);

/// Shared with the watchdog thread, which must not touch `Tap`.
static GRAB: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
/// the heartbeat timer on the main run loop
static TIMER: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static WATCHDOG: std::sync::OnceLock<std::thread::Thread> = std::sync::OnceLock::new();
static DISPLAY: AtomicU32 = AtomicU32::new(0);
/// milliseconds since START at the main thread's last beat
static LAST_BEAT: AtomicU64 = AtomicU64::new(0);
/// true exactly while Windows has control (the grab tap is on)
static REMOTE: AtomicBool = AtomicBool::new(false);
/// set by the watchdog after it took input back; the main thread finishes the job
static RESCUED: AtomicBool = AtomicBool::new(false);
/// a test-only stall of the main thread, in milliseconds (`wisp ctl debug-stall`)
pub static STALL_MS: AtomicU64 = AtomicU64::new(0);
static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn since_start() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Starts the heartbeat now (CF timers may be rescheduled from any thread).
fn wake_beat() {
    let timer = TIMER.load(Ordering::SeqCst);
    if !timer.is_null() {
        unsafe { CFRunLoopTimerSetNextFireDate(timer, CFAbsoluteTimeGetCurrent()) };
    }
}

/// Puts the heartbeat to sleep until `wake_beat`.
fn sleep_beat() {
    let timer = TIMER.load(Ordering::SeqCst);
    if !timer.is_null() {
        unsafe { CFRunLoopTimerSetNextFireDate(timer, CFAbsoluteTimeGetCurrent() + 1e9) };
    }
}

struct Tap {
    link: Arc<Mutex<Option<Link>>>,
    /// listen-only, mouse moves only: edge detection while the cursor is on the Mac
    watch: CFMachPortRef,
    /// swallows everything; enabled only while Windows has control
    grab: CFMachPortRef,
    display: u32,
    live: Arc<Live>,
    switcher: Switcher,
    /// the link the cursor is on; Some exactly while `switcher` is remote
    active: Option<Link>,
    /// when the cursor went over (ms since start), and when to next ask whether macOS
    /// has shown the Mac cursor again behind our back (RULES.md H4)
    entered_at: u64,
    next_visibility_check: u64,
    /// where the Mac cursor was frozen on entering the remote, and whether it has
    /// already been reported as moving since (it must not move while remote)
    frozen: CGPoint,
    thaw_reported: bool,
    pinch: Pinch,
    clipboard: mpsc::Sender<Job>,
}

pub struct Options {
    pub port: u16,
    pub live: Arc<Live>,
}

pub fn run(opts: Options) -> io::Result<()> {
    let step = |what: &str| log!("wisp: 启动 +{}ms：{what}", since_start());
    step("开始");
    let link: Arc<Mutex<Option<Link>>> = Arc::new(Mutex::new(None));
    let clipboard = pasteboard::spawn();

    // everything that might block comes first, while no tap exists yet (RULES.md S1)
    let listener = TcpListener::bind(("0.0.0.0", opts.port))?;
    log!("wisp: 等待客户端连接，端口 {}", opts.port);
    let name = computer_name();
    log!("wisp: 本机名称：{name}");
    match opts.live.paired() {
        Some(p) => log!("wisp: 已与 {} 配对", p.name),
        None => log!("wisp: 还没有配对：在 Wisp 设置里点“配对新电脑”"),
    }
    match UdpSocket::bind(("0.0.0.0", opts.port)) {
        Ok(socket) => {
            let (live, name, port) = (opts.live.clone(), name.clone(), opts.port);
            std::thread::spawn(move || {
                discovery::serve(socket, |q| {
                    let paired = live.paired();
                    let pairing = live.pairing_code().map(|_| name.as_str());
                    discovery::respond(q, paired.as_ref().map(|p| &p.key), pairing, port)
                })
            });
        }
        Err(e) => log!("wisp: 局域网自动发现不可用（客户端需用 --server 指定地址）：{e}"),
    }
    match (TcpListener::bind((Ipv4Addr::LOCALHOST, control::PORT)), control::ensure_token(&crate::data_dir())) {
        (Ok(local), Ok(token)) => {
            let (describe_link, drop) = (link.clone(), link.clone());
            let hooks = control::Link {
                describe: Box::new(move || match &*describe_link.lock().unwrap() {
                    Some(l) => format!("state=connected\tpeer={}\tscreen={}x{}", l.peer, l.w, l.h),
                    None => "state=waiting".to_string(),
                }),
                drop: Box::new(move || drop_link(&drop)),
                stall: Box::new(|ms| {
                    STALL_MS.store(ms, Ordering::Relaxed);
                    wake_beat();
                }),
            };
            let live = opts.live.clone();
            std::thread::spawn(move || control::serve(local, token, live, hooks));
        }
        (Err(e), _) | (_, Err(e)) => log!("wisp: 设置端口不可用（设置窗口无法连接）：{e}"),
    }
    #[cfg(feature = "bridge")]
    {
        // the developer bridge hands its messages to whichever PC is connected; the
        // queue is cloned out of the lock so a full queue never holds it (RULES.md X3)
        let link = link.clone();
        crate::bridge_mac::set_sender(Box::new(move |msg| {
            let tx = link.lock().unwrap().as_ref().map(|l| l.tx.clone());
            tx.is_some_and(|tx| tx.send(msg).is_ok())
        }));
    }
    std::thread::spawn({
        let (link, clipboard, live) = (link.clone(), clipboard.clone(), opts.live.clone());
        move || accept_loop(listener, link, clipboard, name, live)
    });
    unsafe {
        // let a background process hide the cursor while it is on the client
        let key = CFStringCreateWithCString(ptr::null(), b"SetsCursorInBackground\0".as_ptr(), 0x0800_0100);
        let cid = _CGSDefaultConnection();
        CGSSetConnectionProperty(cid, cid, key, kCFBooleanTrue);
    }
    step("网络与设置端口就绪");

    let display = unsafe { CGMainDisplayID() };
    let tap = Box::into_raw(Box::new(Tap {
        clipboard,
        link,
        watch: ptr::null_mut(),
        grab: ptr::null_mut(),
        display,
        live: opts.live.clone(),
        switcher: Switcher::default(),
        active: None,
        entered_at: 0,
        next_visibility_check: 0,
        frozen: CGPoint { x: 0.0, y: 0.0 },
        thaw_reported: false,
        pinch: Pinch::default(),
    }));

    let grab_mask = [
        LEFT_DOWN, LEFT_UP, RIGHT_DOWN, RIGHT_UP, MOUSE_MOVED, LEFT_DRAGGED, RIGHT_DRAGGED, KEY_DOWN, KEY_UP,
        FLAGS_CHANGED, SCROLL, OTHER_DOWN, OTHER_UP, OTHER_DRAGGED, GESTURE, MAGNIFY, SYS_DEFINED,
    ]
    .iter()
    .fold(0u64, |m, t| m | 1 << t);
    // Creating the active tap checks the permission live; AXIsProcessTrusted() stays
    // stale for a process that was already running when the user granted access.
    // A new tap starts enabled, so it is switched off on the spot (RULES.md S2).
    let create_grab = || unsafe {
        let port = CGEventTapCreate(HID, HEAD, ACTIVE, grab_mask, on_grab, tap as *mut c_void);
        if !port.is_null() {
            CGEventTapEnable(port, false);
        }
        port
    };
    let mut grab = create_grab();
    if grab.is_null() {
        prompt_accessibility();
        log!("wisp: 需要「辅助功能」权限，请在 系统设置 → 隐私与安全性 → 辅助功能 中打开 wisp，授权后会自动继续");
        // keep waiting instead of exiting, so launchd does not respawn us and re-prompt every few seconds
        while grab.is_null() {
            std::thread::sleep(Duration::from_secs(2));
            grab = create_grab();
        }
        log!("wisp: 已获得辅助功能权限");
    }
    let watch = unsafe { CGEventTapCreate(HID, HEAD, LISTEN_ONLY, 1 << MOUSE_MOVED, on_watch, tap as *mut c_void) };
    if watch.is_null() {
        return Err(io::Error::other("无法创建旁听拦截"));
    }
    unsafe {
        (*tap).grab = grab;
        (*tap).watch = watch;
    }
    GRAB.store(grab, Ordering::SeqCst);
    DISPLAY.store(display, Ordering::SeqCst);
    step("拦截已创建（控制权拦截关闭中）");

    unsafe {
        let rl = CFRunLoopGetCurrent();
        for port in [grab, watch] {
            CFRunLoopAddSource(rl, CFMachPortCreateRunLoopSource(ptr::null(), port, 0), kCFRunLoopCommonModes);
        }
        let mut ctx = TimerContext {
            version: 0,
            info: tap as *mut c_void,
            retain: ptr::null(),
            release: ptr::null(),
            describe: ptr::null(),
        };
        let secs = BEAT.as_secs_f64();
        // asleep until the cursor first goes over to Windows
        let timer = CFRunLoopTimerCreate(ptr::null(), CFAbsoluteTimeGetCurrent() + 1e9, secs, 0, 0, on_beat, &mut ctx);
        CFRunLoopAddTimer(rl, timer, kCFRunLoopCommonModes);
        TIMER.store(timer, Ordering::SeqCst);
    }
    LAST_BEAT.store(since_start(), Ordering::SeqCst);
    let _ = WATCHDOG.set(std::thread::spawn(watchdog).thread().clone());
    unsafe { CGEventTapEnable(watch, true) };
    step("进入事件循环");
    unsafe { CFRunLoopRun() };
    Ok(())
}

/// Runs on its own thread: if the main thread has been silent too long while Windows
/// has control, switch the grab tap off and give the Mac its cursor back (RULES.md S3).
/// Every call here is safe off the main thread; `Tap` itself is left to the main thread.
fn watchdog() {
    loop {
        if !REMOTE.load(Ordering::SeqCst) {
            // woken by check_edge; a spurious wake-up just loops back here
            std::thread::park();
            continue;
        }
        std::thread::sleep(BEAT);
        let silent = since_start().saturating_sub(LAST_BEAT.load(Ordering::SeqCst));
        if silent < STALL_LIMIT.as_millis() as u64 {
            continue;
        }
        // only one side may thaw the cursor (hide/show is counted): win REMOTE or leave it
        if REMOTE.compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            continue;
        }
        RESCUED.store(true, Ordering::SeqCst);
        unsafe {
            CGEventTapEnable(GRAB.load(Ordering::SeqCst), false);
            CGAssociateMouseAndMouseCursorPosition(1);
            show_cursor(DISPLAY.load(Ordering::SeqCst));
        }
        log!("wisp: 异常：主线程 {silent}ms 没有响应，看门狗已关闭拦截并把键鼠还给 Mac");
        // the main thread may have put the heartbeat to sleep between REMOTE and RESCUED:
        // make sure it beats again to finish the hand-back
        wake_beat();
    }
}

/// The main thread's heartbeat; also where it notices a rescue and a test stall.
extern "C" fn on_beat(_timer: *mut c_void, info: *mut c_void) {
    let tap = unsafe { &mut *(info as *mut Tap) };
    LAST_BEAT.store(since_start(), Ordering::SeqCst);
    if RESCUED.swap(false, Ordering::SeqCst) {
        tap.release(None, false);
    }
    let stall = STALL_MS.swap(0, Ordering::Relaxed);
    if stall > 0 {
        log!("wisp: 测试：主线程暂停 {stall}ms");
        std::thread::sleep(Duration::from_millis(stall));
    }
    if !REMOTE.load(Ordering::SeqCst) && !RESCUED.load(Ordering::SeqCst) && STALL_MS.load(Ordering::Relaxed) == 0 {
        sleep_beat();
    }
}

fn prompt_accessibility() {
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let opts = CFDictionaryCreate(
            ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        );
        AXIsProcessTrustedWithOptions(opts);
    }
}

/// Listen-only: whatever happens here, the event has already gone on its way.
extern "C" fn on_watch(_proxy: *mut c_void, ty: u32, ev: CGEventRef, refcon: *mut c_void) -> CGEventRef {
    let tap = unsafe { &mut *(refcon as *mut Tap) };
    if ty == TAP_DISABLED_BY_TIMEOUT || ty == TAP_DISABLED_BY_USER {
        unsafe { CGEventTapEnable(tap.watch, true) };
        return ev;
    }
    // paused: the cursor never crosses over (RULES.md U4)
    if ty == MOUSE_MOVED && tap.active.is_none() && !tap.live.paused() {
        tap.check_edge(ev);
    }
    ev
}

extern "C" fn on_grab(_proxy: *mut c_void, ty: u32, ev: CGEventRef, refcon: *mut c_void) -> CGEventRef {
    let tap = unsafe { &mut *(refcon as *mut Tap) };
    if ty == TAP_DISABLED_BY_TIMEOUT || ty == TAP_DISABLED_BY_USER {
        if REMOTE.load(Ordering::SeqCst) {
            unsafe { CGEventTapEnable(tap.grab, true) };
            // the watchdog may have stepped in between the check and the enable
            if !REMOTE.load(Ordering::SeqCst) {
                unsafe { CGEventTapEnable(tap.grab, false) };
            }
        }
        return ev;
    }
    // stragglers after control came back, or after a rescue: the Mac keeps them
    if tap.active.is_none() || !REMOTE.load(Ordering::SeqCst) {
        return ev;
    }
    // brightness, the keyboard backlight, Do Not Disturb: the Mac's even now (RULES.md K7)
    if stays_on_mac(ty, ev) {
        return ev;
    }
    tap.handle(ty, ev);
    ptr::null_mut()
}

/// Whether a key event is one the Mac keeps while Windows has control.
fn stays_on_mac(ty: u32, ev: CGEventRef) -> bool {
    match ty {
        // not a media key (another subtype): left alone, as before the tap saw these
        SYS_DEFINED => keymap::aux_button(field(ev, SYS_SUBTYPE), field(ev, SYS_DATA1)).is_none_or(|(key, _)| keymap::aux_key(key) == SystemKey::Mac),
        KEY_DOWN | KEY_UP => keymap::special_key(field(ev, KEYCODE) as u16) == Some(SystemKey::Mac),
        _ => false,
    }
}

/// Moves within this long after the hop are still the Mac's own (RULES.md S2).
const SETTLE_MS: u64 = 150;
const VISIBILITY_EVERY_MS: u64 = 250;

/// Hiding is counted, and macOS resets the count on its own (display wake), so
/// hide and show go by what is on screen: at most one hide outstanding, ever.
fn hide_cursor(display: u32) {
    unsafe {
        if CGCursorIsVisible() != 0 {
            CGDisplayHideCursor(display);
        }
    }
}

/// Shows until it is on screen: if macOS showed the cursor without clearing our
/// hide count and we hid it again, one show is not enough. Bounded, never a hang.
fn show_cursor(display: u32) {
    for _ in 0..8 {
        if unsafe { CGCursorIsVisible() } != 0 {
            return;
        }
        unsafe { CGDisplayShowCursor(display) };
    }
}

fn field(ev: CGEventRef, f: u32) -> i64 {
    unsafe { CGEventGetIntegerValueField(ev, f) }
}

fn display_bounds(display: u32) -> Rect {
    let b = unsafe { CGDisplayBounds(display) };
    Rect { x: b.x, y: b.y, w: b.w, h: b.h }
}

impl Tap {
    /// An event while Windows has control; it is swallowed either way.
    fn handle(&mut self, ty: u32, ev: CGEventRef) {
        self.keep_frozen(ty, ev);
        let id = self.active.as_ref().map_or(0, |a| a.id);
        if current(&mut self.link.lock().unwrap(), id).is_none() {
            log!("wisp: 客户端已断开，控制权回到 Mac");
            self.go_local(None);
            return;
        }
        if self.forward(ty, ev).is_err() {
            self.go_local(None);
        }
    }

    /// While Windows has control the Mac cursor must stay frozen and hidden. macOS
    /// undoes both on its own, e.g. when the display wakes (RULES.md H4): put them
    /// back as soon as an event shows it. The cursor is not warped back (RULES.md L9).
    fn keep_frozen(&mut self, ty: u32, ev: CGEventRef) {
        // after a rescue the Mac owns the cursor again, even if this thread was mid-event
        if !REMOTE.load(Ordering::SeqCst) {
            return;
        }
        let p = unsafe { CGEventGetLocation(ev) };
        let now = since_start();
        if now < self.entered_at + SETTLE_MS {
            // moves already in flight at the hop still land on the Mac: the new baseline
            self.frozen = p;
        } else if (p.x - self.frozen.x).abs() > 20.0 || (p.y - self.frozen.y).abs() > 20.0 {
            if !self.thaw_reported {
                self.thaw_reported = true;
                log!(
                    "wisp: 异常：控制 Windows 期间 Mac 光标在移动（冻结于 {:.0},{:.0}，现在 {:.0},{:.0}，事件类型 {ty}），重新冻结",
                    self.frozen.x, self.frozen.y, p.x, p.y
                );
            }
            unsafe { CGAssociateMouseAndMouseCursorPosition(0) };
            self.frozen = p;
        }
        if now >= self.next_visibility_check {
            self.next_visibility_check = now + VISIBILITY_EVERY_MS;
            if unsafe { CGCursorIsVisible() } != 0 {
                log!("wisp: Mac 指针被系统重新显示（例如亮屏后），重新冻结并隐藏");
                unsafe { CGAssociateMouseAndMouseCursorPosition(0) };
                hide_cursor(self.display);
                self.frozen = p;
            }
        }
    }

    fn check_edge(&mut self, ev: CGEventRef) {
        let p = unsafe { CGEventGetLocation(ev) };
        let (display, link) = (self.display, &self.link);
        let mut target = None;
        let hop = self.switcher.local_move(
            self.live.settings().edge,
            (p.x, p.y),
            (field(ev, DELTA_X) as f64, field(ev, DELTA_Y) as f64),
            || display_bounds(display),
            || {
                target = link.lock().unwrap().clone();
                target.as_ref().map(|l| (l.w, l.h))
            },
        );
        let (Hop::Enter { x, y }, Some(link)) = (hop, target) else { return };
        // never block the event tap: a full queue means the link is stuck, same as gone
        if link.tx.try_send(Msg::Enter).is_err() || link.tx.try_send(Msg::MouseAbs { x, y }).is_err() {
            self.switcher.reset();
            return;
        }
        unsafe {
            CGAssociateMouseAndMouseCursorPosition(0);
            hide_cursor(self.display);
            // a fresh beat, so a backlog right after a long stall is not mistaken for one
            LAST_BEAT.store(since_start(), Ordering::SeqCst);
            REMOTE.store(true, Ordering::SeqCst);
            CGEventTapEnable(self.grab, true);
        }
        wake_beat();
        if let Some(w) = WATCHDOG.get() {
            w.unpark();
        }
        let _ = self.clipboard.send(Job::Push(link.tx.clone()));
        self.frozen = p;
        self.thaw_reported = false;
        self.entered_at = since_start();
        self.next_visibility_check = self.entered_at + VISIBILITY_EVERY_MS;
        log!("wisp: 光标进入 Windows（方向 {}，从 Mac {:.0},{:.0}）", self.live.settings().edge.name(), p.x, p.y);
        self.active = Some(link);
    }

    /// Hands the cursor back to the Mac, optionally placing it at `at`.
    fn go_local(&mut self, at: Option<CGPoint>) {
        self.release(at, true);
    }

    /// Ends remote control. `thaw` is false after a rescue, when the watchdog has
    /// already switched the grab tap off and shown the cursor (hide/show is counted).
    fn release(&mut self, at: Option<CGPoint>, thaw: bool) {
        self.switcher.reset();
        let Some(link) = self.active.take() else { return };
        match at {
            Some(p) => log!("wisp: 光标回到 Mac（{:.0},{:.0}）", p.x, p.y),
            None if thaw => log!("wisp: 控制权收回 Mac（连接断开或发送失败）"),
            None => log!("wisp: 控制权收回 Mac（看门狗恢复后）"),
        }
        let _ = link.tx.try_send(Msg::Leave);
        let ours = thaw && REMOTE.swap(false, Ordering::SeqCst);
        // all of this is safe to repeat after the watchdog did it: the grab tap off,
        // the cursor attached and shown (by what is on screen, RULES.md H4)
        unsafe {
            CGEventTapEnable(self.grab, false);
            CGAssociateMouseAndMouseCursorPosition(1);
            if let (true, Some(p)) = (ours, at) {
                CGSetLocalEventsSuppressionInterval(0.0);
                CGWarpMouseCursorPosition(p);
            }
        }
        show_cursor(self.display);
    }

    fn send(&self, msg: Msg) -> Result<(), ()> {
        self.active.as_ref().unwrap().tx.try_send(msg).map_err(|_| ())
    }

    /// Taps a key combination: pressed in order, released in reverse (RULES.md K7).
    fn chord(&self, keys: &[u16]) -> Result<(), ()> {
        for &scan in keys {
            self.send(Msg::Key { scan, down: true })?;
        }
        for &scan in keys.iter().rev() {
            self.send(Msg::Key { scan, down: false })?;
        }
        Ok(())
    }

    fn forward(&mut self, ty: u32, ev: CGEventRef) -> Result<(), ()> {
        match ty {
            MOUSE_MOVED | LEFT_DRAGGED | RIGHT_DRAGGED | OTHER_DRAGGED => {
                let display = self.display;
                let (dx, dy) = (field(ev, DELTA_X) as f64, field(ev, DELTA_Y) as f64);
                let speed = self.live.settings().speed;
                match self.switcher.remote_move(speed, (dx, dy), ty != MOUSE_MOVED, || display_bounds(display)) {
                    Hop::Move { x, y } => self.send(Msg::MouseAbs { x, y }),
                    Hop::Leave { x, y } => {
                        self.go_local(Some(CGPoint { x, y }));
                        Ok(())
                    }
                    _ => Ok(()),
                }
            }
            LEFT_DOWN | LEFT_UP => self.send(Msg::Button { btn: 1, down: ty == LEFT_DOWN }),
            RIGHT_DOWN | RIGHT_UP => self.send(Msg::Button { btn: 2, down: ty == RIGHT_DOWN }),
            OTHER_DOWN | OTHER_UP => {
                let btn = match field(ev, BUTTON_NUMBER) {
                    2 => 3,
                    3 => 4,
                    4 => 5,
                    _ => return Ok(()),
                };
                self.send(Msg::Button { btn, down: ty == OTHER_DOWN })
            }
            GESTURE | MAGNIFY => {
                let phase = field(ev, GESTURE_PHASE);
                if phase == PHASE_ENDED || phase == PHASE_CANCELLED {
                    self.pinch.end();
                }
                if field(ev, GESTURE_HID_TYPE) != HID_TYPE_ZOOM {
                    return Ok(());
                }
                match self.pinch.add(unsafe { CGEventGetDoubleValueField(ev, GESTURE_ZOOM_VALUE) }) {
                    0 => Ok(()),
                    steps => self.send(Msg::Zoom { steps }),
                }
            }
            SCROLL => {
                let f = |k| field(ev, k) as f64;
                let points = (f(SCROLL_POINTS_X), f(SCROLL_POINTS_Y));
                let lines = (f(SCROLL_LINES_X), f(SCROLL_LINES_Y));
                match capture::wheel(field(ev, SCROLL_CONTINUOUS) != 0, points, lines, self.live.settings().scroll) {
                    Some((dx, dy)) => self.send(Msg::Wheel { dx, dy }),
                    None => Ok(()),
                }
            }
            KEY_DOWN | KEY_UP => {
                let kc = field(ev, KEYCODE) as u16;
                if let Some(scan) = keymap::mac_to_scan(kc) {
                    return self.send(Msg::Key { scan, down: ty == KEY_DOWN });
                }
                match keymap::special_key(kc) {
                    // once per press, not per autorepeat
                    Some(SystemKey::Chord(keys)) if ty == KEY_DOWN && field(ev, AUTOREPEAT) == 0 => self.chord(keys),
                    Some(_) => Ok(()),
                    None => {
                        if ty == KEY_DOWN && field(ev, AUTOREPEAT) == 0 {
                            log!("wisp: 没有对应的按键，未转发（keycode {kc:#x}）");
                        }
                        Ok(())
                    }
                }
            }
            SYS_DEFINED => match keymap::aux_button(field(ev, SYS_SUBTYPE), field(ev, SYS_DATA1)) {
                Some((key, down)) => match keymap::aux_key(key) {
                    SystemKey::Key(scan) => self.send(Msg::Key { scan, down }),
                    SystemKey::Chord(keys) if down => self.chord(keys),
                    _ => Ok(()),
                },
                None => Ok(()),
            },
            FLAGS_CHANGED => {
                let flags = unsafe { CGEventGetFlags(ev) };
                let Some((scan, change)) = keymap::flags_changed(field(ev, KEYCODE) as u16, flags) else { return Ok(()) };
                match change {
                    Change::Press => self.send(Msg::Key { scan, down: true }),
                    Change::Release => self.send(Msg::Key { scan, down: false }),
                    Change::Tap => {
                        self.send(Msg::Key { scan, down: true })?;
                        self.send(Msg::Key { scan, down: false })
                    }
                }
            }
            _ => Ok(()),
        }
    }
}

/// The registered link, if it still belongs to connection `id`.
fn current(link: &mut Option<Link>, id: u64) -> Option<&mut Link> {
    link.as_mut().filter(|l| l.id == id)
}

/// The Computer Name from System Settings (e.g. "小林的 MacBook Pro"), read once;
/// `scutil` keeps SystemConfiguration out of this process.
fn computer_name() -> String {
    std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|n| proto::short_name(&n))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Mac".to_string())
}

/// Cuts the current connection, e.g. after unpairing or pairing with another machine.
fn drop_link(link: &Mutex<Option<Link>>) {
    if let Some(l) = link.lock().unwrap().take() {
        let _ = l.closer.shutdown(Shutdown::Both);
    }
}

/// The first four bytes a client sent, without consuming them.
fn peek_magic(stream: &TcpStream) -> io::Result<[u8; 4]> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut head = [0u8; 4];
    loop {
        match stream.peek(&mut head)? {
            0 => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            4 => return Ok(head),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn accept_loop(
    listener: TcpListener,
    link: Arc<Mutex<Option<Link>>>,
    clipboard: mpsc::Sender<Job>,
    name: String,
    live: Arc<Live>,
) {
    for (id, stream) in (1u64..).zip(listener.incoming().flatten()) {
        let (link, clipboard, name, live) = (link.clone(), clipboard.clone(), name.clone(), live.clone());
        std::thread::spawn(move || {
            let peer = stream.peer_addr().map_or_else(|_| "未知地址".to_string(), |a| a.to_string());
            if peek_magic(&stream).is_ok_and(|m| &m == pair::MAGIC) {
                match pair::host(stream, &live.pairing, &name, control::now()) {
                    Ok(p) => {
                        log!("wisp: 已与 {} 配对（{peer}）", p.name);
                        if let Err(e) = live.set_paired(Some(p)) {
                            log!("wisp: 保存配对失败：{e}");
                        }
                        drop_link(&link);
                    }
                    Err(e) => log!("wisp: {peer} 配对失败：{e}"),
                }
                return;
            }
            let Some(paired) = live.paired() else {
                log!("wisp: 还没有配对，不接受 {peer} 的连接");
                return;
            };
            let Err(e) = serve_client(stream, &peer, &link, &clipboard, &name, paired.key, id);
            log!("wisp: {peer} 断开：{e}");
            let mut l = link.lock().unwrap();
            if current(&mut l, id).is_some() {
                *l = None;
            }
            // even when a newer link already took its place: requests sent over this one
            // will never be answered, and must not wait out their timeout (RULES.md X3)
            #[cfg(feature = "bridge")]
            crate::bridge_mac::on_disconnect();
        });
    }
}
fn serve_client(
    stream: TcpStream,
    peer: &str,
    link: &Mutex<Option<Link>>,
    clipboard: &mpsc::Sender<Job>,
    name: &str,
    psk: [u8; 32],
    id: u64,
) -> io::Result<Infallible> {
    let closer = Arc::new(stream.try_clone()?);
    let (tx, mut rx) = proto::handshake(stream, &psk, true)?;
    // the first frame decrypting proves the client knows the password;
    // only then does it get a writer thread and a place as the active link
    let Msg::Info { w, h } = rx.recv()? else {
        return Err(io::Error::other("expected screen info"));
    };
    log!("wisp: {peer} 已连接，屏幕 {w}×{h}");
    let tx = tx.spawn();
    let _ = tx.send(Msg::Name(name.to_string()));
    *link.lock().unwrap() = Some(Link { id, peer: peer.to_string(), tx, w: w as f64, h: h as f64, closer });
    #[cfg(feature = "bridge")]
    crate::bridge_mac::on_connect();

    Err(rx.run(|msg| match msg {
        Msg::Info { w, h } => {
            if let Some(l) = current(&mut link.lock().unwrap(), id) {
                l.w = w as f64;
                l.h = h as f64;
            }
        }
        Msg::Clipboard(text) => {
            let _ = clipboard.send(Job::Pull(text));
        }
        #[cfg(feature = "bridge")]
        Msg::Bridge(bytes) => crate::bridge_mac::on_message(&bytes),
        _ => {}
    }))
}
