#!/usr/bin/env python3
"""End-to-end check of the installed wisp, Mac -> Windows, as a black box.

Posts synthetic trackpad/keyboard events on the Mac (they pass through wisp's event
tap like real input) and reads back what Windows really did through the ComfyUI
bridge. Each check names the RULES.md rule it covers. Both clipboards and the Mac
cursor are restored afterwards.

It moves the Windows cursor, holds keys and replaces clipboards while it runs:
only run it when nobody is using either machine, and keep hands off the trackpad
and keyboard until it finishes, or the results are meaningless. Needs Accessibility permission
for the terminal that runs it (to post events).
    usage: scripts/e2e.py
"""
import base64
import ctypes
import os
import re
import shlex
import subprocess
import sys
import time
from ctypes import c_bool, c_double, c_int64, c_uint16, c_uint32, c_uint64, c_void_p

HERE = os.path.dirname(os.path.abspath(__file__))
def _local_env():
    """KEY="value" lines from scripts/local.env (machine-specific, not committed), then the environment."""
    found = {}
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "local.env")
    if os.path.exists(path):
        for line in open(path, encoding="utf-8"):
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                found[k.strip()] = v.strip().strip('"')
    found.update({k: v for k, v in os.environ.items() if k.startswith("WISP_")})
    return found


_ENV = _local_env()
if not _ENV.get("WISP_WIN_BRIDGE") or not _ENV.get("WISP_WIN_DIR"):
    sys.exit("e2e: set WISP_WIN_BRIDGE and WISP_WIN_DIR in scripts/local.env (see README)")
COMFY = shlex.split(_ENV["WISP_WIN_BRIDGE"])
WIN_DIR = _ENV["WISP_WIN_DIR"]
WIN_HELPER = WIN_DIR.replace("/", "\\") + "\\e2e_win.ps1"
MAC_LOG = os.path.expanduser("~/Library/Logs/wisp.log")
WISP = os.path.expanduser("~/Library/Application Support/Wisp/wisp")
SPEED = 1.0  # the installed server runs with the default --speed

# --- CoreGraphics -----------------------------------------------------------
CG = ctypes.cdll.LoadLibrary("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics")
CF = ctypes.cdll.LoadLibrary("/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation")


class CGPoint(ctypes.Structure):
    _fields_ = [("x", c_double), ("y", c_double)]


class CGRect(ctypes.Structure):
    _fields_ = [("x", c_double), ("y", c_double), ("w", c_double), ("h", c_double)]


CG.CGEventCreate.restype = c_void_p
CG.CGEventCreate.argtypes = [c_void_p]
CG.CGEventGetLocation.restype = CGPoint
CG.CGEventGetLocation.argtypes = [c_void_p]
CG.CGEventCreateMouseEvent.restype = c_void_p
CG.CGEventCreateMouseEvent.argtypes = [c_void_p, c_uint32, CGPoint, c_uint32]
CG.CGEventCreateKeyboardEvent.restype = c_void_p
CG.CGEventCreateKeyboardEvent.argtypes = [c_void_p, c_uint16, c_bool]
CG.CGEventSetIntegerValueField.argtypes = [c_void_p, c_uint32, c_int64]
CG.CGEventSetType.argtypes = [c_void_p, c_uint32]
CG.CGEventSetFlags.argtypes = [c_void_p, c_uint64]
CG.CGEventPost.argtypes = [c_uint32, c_void_p]
CG.CGWarpMouseCursorPosition.argtypes = [CGPoint]
CG.CGMainDisplayID.restype = c_uint32
CG.CGDisplayBounds.restype = CGRect
CG.CGDisplayBounds.argtypes = [c_uint32]
CF.CFRelease.argtypes = [c_void_p]
CG.CGCursorIsVisible.restype = c_bool

HID_TAP = 0
MOUSE_MOVED = 5
FLAGS_CHANGED = 12
DELTA_X, DELTA_Y = 4, 5


def post(ev):
    CG.CGEventPost(HID_TAP, ev)
    CF.CFRelease(ev)
    time.sleep(0.05)


def cursor():
    ev = CG.CGEventCreate(None)
    p = CG.CGEventGetLocation(ev)
    CF.CFRelease(ev)
    return p.x, p.y


def move(x, y, dx, dy):
    """A trackpad move reported at (x, y) with raw deltas (dx, dy)."""
    ev = CG.CGEventCreateMouseEvent(None, MOUSE_MOVED, CGPoint(x, y), 0)
    CG.CGEventSetIntegerValueField(ev, DELTA_X, dx)
    CG.CGEventSetIntegerValueField(ev, DELTA_Y, dy)
    post(ev)


def modifier(keycode, device_bit, family_bit, down):
    ev = CG.CGEventCreateKeyboardEvent(None, keycode, down)
    CG.CGEventSetType(ev, FLAGS_CHANGED)
    CG.CGEventSetFlags(ev, (device_bit | family_bit) if down else 0)
    post(ev)


LEFT_SHIFT = (0x38, 0x02, 0x20000)
LEFT_COMMAND = (0x37, 0x08, 0x100000)
VK_LSHIFT, VK_LCONTROL = 0xA0, 0xA2

# --- Windows observer ---------------------------------------------------------


def win(cmd, arg=""):
    out = subprocess.run(
        COMFY + ["exec", f"powershell -NoProfile -ExecutionPolicy Bypass -File {WIN_HELPER} -Cmd {cmd} -Arg {arg or 'x'}",
                 "--timeout", "30"],
        capture_output=True, text=True, timeout=90,
    ).stdout.strip().splitlines()
    return out[-1].strip() if out else ""


def win_cursor():
    x, y = win("cursor").split(",")
    return int(x), int(y)


def b64(text):
    return base64.b64encode(text.encode()).decode()


def win_clip():
    return base64.b64decode(win("clipget")).decode()


def mac_clip():
    return subprocess.run(["pbpaste"], capture_output=True, text=True).stdout


def set_mac_clip(text):
    subprocess.run(["pbcopy"], input=text, text=True, check=True)


def ctl(command):
    return subprocess.run([WISP, "ctl", *command.split()], capture_output=True, text=True).stdout.strip()


# --- checks ---------------------------------------------------------------------

results = []
# another clipboard syncer (e.g. UU Remote) copies text across on its own within a few seconds
OTHER_SYNC_WAIT = 3


def check(rule, what, ok, detail=""):
    results.append(ok)
    print(f"  {'PASS' if ok else 'FAIL'}  {rule:<5} {what}" + (f"  ({detail})" if detail else ""))


def skip(rule, what, why):
    print(f"  SKIP  {rule:<5} {what}  ({why})")


def near(a, b, tol=1):
    return all(abs(p - q) <= tol for p, q in zip(a, b))


def remote_size():
    with open(MAC_LOG, encoding="utf-8") as f:
        sizes = re.findall(r"屏幕 (\d+)×(\d+)", f.read())
    if not sizes:
        sys.exit("e2e: no client in the Mac log; is wisp connected? (scripts/verify.sh --live)")
    return tuple(map(int, sizes[-1]))


def display_wake(b, px):
    """RULES.md H4: macOS shows the Mac pointer again when the display wakes; the next
    event while Windows has control must freeze and hide it again."""
    CG.CGWarpMouseCursorPosition(CGPoint(px, b.y + 50))
    move(px, b.y, 0, -5)
    subprocess.run(["pmset", "displaysleepnow"], check=True)
    time.sleep(4)
    subprocess.run(["caffeinate", "-u", "-t", "2"], check=True)
    time.sleep(3)
    check("H4", "(setup) waking the display shows the Mac pointer again", CG.CGCursorIsVisible())
    move(px, b.y, 10, 0)
    time.sleep(0.3)
    check("H4", "after the display wakes on Windows, the Mac pointer is hidden again", not CG.CGCursorIsVisible())
    move(px, b.y, 0, 5000)
    time.sleep(0.5)
    check("H4", "back on the Mac the pointer shows", CG.CGCursorIsVisible())


def main():
    subprocess.run(COMFY + ["put", os.path.join(HERE, "e2e_win.ps1"), WIN_DIR + "/e2e_win.ps1"],
                   capture_output=True, check=True)
    W, H = remote_size()
    b = CG.CGDisplayBounds(CG.CGMainDisplayID())
    home = cursor()
    saved_mac, saved_win = mac_clip(), win_clip()
    token = f"wisp e2e {int(time.time())}"
    mac_text, win_text = f"{token} Mac→Win\n第二行 ✓", f"{token} Win→Mac\r\n第二行 ✓"

    try:
        # start local: a big downward move gets the cursor back if it was on Windows
        move(home[0], home[1], 0, 5000)
        print(f"e2e: Mac display {b.w:.0f}×{b.h:.0f}, Windows {W}×{H}")

        set_mac_clip(mac_text)
        time.sleep(OTHER_SYNC_WAIT)
        mac_synced_elsewhere = win_clip().replace("\r\n", "\n") == mac_text
        px = b.x + b.w / 2
        CG.CGWarpMouseCursorPosition(CGPoint(px, b.y + 50))
        move(px, b.y, 0, -5)  # push up past the top edge
        expect = (round((px - b.x) / b.w * (W - 1)), H - 1)
        got = win_cursor()
        check("L2/L3", "pushing up at the top enters Windows at its bottom edge", near(got, expect), f"{got} vs {expect}")
        check("H3", "the Windows pointer shows while Windows has control", win("shown") == "shown")

        move(px, b.y, 100, -100)
        expect2 = (round(expect[0] + 100 * SPEED), round(expect[1] - 100 * SPEED))
        got = win_cursor()
        check("L6/M1", "moves land on the same pixel on Windows", near(got, expect2), f"{got} vs {expect2}")

        time.sleep(0.5)
        got = win_clip()
        if mac_synced_elsewhere:
            skip("CB1", "Mac clipboard arrives on Windows (CRLF there)", "another program synced it before wisp could")
        else:
            check("CB1", "Mac clipboard arrives on Windows (CRLF there)", got == mac_text.replace("\n", "\r\n"), repr(got[:40]))

        modifier(*LEFT_SHIFT, True)
        check("K4", "Shift down reaches Windows", win("key", VK_LSHIFT) == "down")
        modifier(*LEFT_SHIFT, False)
        check("K4", "Shift up reaches Windows", win("key", VK_LSHIFT) == "up")

        modifier(*LEFT_COMMAND, True)
        check("K3", "Command acts as Ctrl on Windows", win("key", VK_LCONTROL) == "down")

        win("clipset", b64(win_text))
        time.sleep(OTHER_SYNC_WAIT)
        win_synced_elsewhere = mac_clip() == win_text.replace("\r\n", "\n")
        # hop back with Command still held: Windows must let go of Ctrl
        move(px, b.y, 0, 5000)
        time.sleep(0.5)
        check("H1", "leaving releases keys still held on Windows", win("key", VK_LCONTROL) == "up")
        check("H3", "the Windows pointer hides once control is back on the Mac", win("shown") == "hidden")
        modifier(*LEFT_COMMAND, False)

        x, y = cursor()
        # wisp tracks the remote position unrounded; the Mac reports whole points
        remote_x = (px - b.x) / b.w * (W - 1) + 100 * SPEED
        back_x = b.x + remote_x / W * b.w
        check("L4", "pushing down returns to the Mac top, same column", abs(x - back_x) <= 1 and abs(y - (b.y + 2)) <= 1,
              f"({x:.1f},{y:.1f}) vs ({back_x:.1f},{b.y + 2})")
        got = mac_clip()
        if win_synced_elsewhere:
            skip("CB1/CB2b", "Windows clipboard comes back to the Mac (LF here)", "another program synced it before wisp could")
        else:
            check("CB1/CB2b", "Windows clipboard comes back to the Mac (LF here)", got == win_text.replace("\r\n", "\n"), repr(got[:40]))

        # every edge (RULES.md L1/U2): push out, land on the opposite side, push back
        mid = (b.x + b.w / 2, b.y + b.h / 2)
        fx, fy = (mid[0] - b.x) / b.w, (mid[1] - b.y) / b.h
        edges = {
            # name: (where to stand, push toward the edge, expected Windows pixel, push back, expected Mac point)
            "left": ((b.x, mid[1]), (-5, 0), (W - 1, round(fy * (H - 1))), (5, 0), (b.x + 2, None)),
            "right": ((b.x + b.w - 1, mid[1]), (5, 0), (0, round(fy * (H - 1))), (-5, 0), (b.x + b.w - 3, None)),
        }
        for name, (at, push, want_win, out, want_mac) in edges.items():
            ctl(f"set edge={name}")
            CG.CGWarpMouseCursorPosition(CGPoint(*mid))
            move(at[0], at[1], *push)
            got = win_cursor()
            check("U2", f"edge={name}: enters Windows on the opposite side", near(got, want_win), f"{got} vs {want_win}")
            move(at[0], at[1], *out)
            x, y = cursor()
            ok = abs(x - want_mac[0]) <= 1.5 and abs(y - mid[1]) <= 1.5
            check("U2", f"edge={name}: pushing back returns beside the same spot", ok, f"({x:.1f},{y:.1f})")

        # pause (RULES.md U4): pushing at the edge stays on the Mac
        ctl("set edge=top")
        ctl("pause")
        before = win_cursor()
        CG.CGWarpMouseCursorPosition(CGPoint(px, b.y + 50))
        move(px, b.y, 0, -5)
        move(px, b.y, 0, -5)
        check("U4", "paused: the cursor stays on the Mac", win_cursor() == before and cursor()[1] <= b.y + 1,
              f"windows {win_cursor()} mac {cursor()}")
        ctl("resume")

        # input safety (RULES.md S2/S3/S4): a stuck event thread never takes the Mac's input
        with open(MAC_LOG, encoding="utf-8") as f:
            stalls = f.read().count("测试：主线程暂停")
        ctl("debug-stall 3000")
        time.sleep(0.8)  # the stall starts at the next heartbeat
        with open(MAC_LOG, encoding="utf-8") as f:
            check("S4", "(setup) the event thread really is stuck", f.read().count("测试：主线程暂停") > stalls)
        CG.CGWarpMouseCursorPosition(CGPoint(*mid))
        move(mid[0] + 40, mid[1] + 40, 40, 40)
        check("S2", "stuck while on the Mac: the Mac still moves", near(cursor(), (mid[0] + 40, mid[1] + 40), 1.5),
              f"mac {cursor()}")
        time.sleep(2.5)

        CG.CGWarpMouseCursorPosition(CGPoint(px, b.y + 50))
        move(px, b.y, 0, -5)
        check("S3", "(setup) entered Windows", win("shown") == "shown")
        with open(MAC_LOG, encoding="utf-8") as f:
            log = f.read()
            seen, seen_back = log.count("看门狗已关闭拦截"), log.count("看门狗恢复后")
        ctl("debug-stall 6000")
        rescued = False
        deadline = time.time() + 5  # stall starts within 0.5 s, the watchdog acts within 2 s of it
        while time.time() < deadline and not rescued:
            time.sleep(0.25)
            with open(MAC_LOG, encoding="utf-8") as f:
                rescued = f.read().count("看门狗已关闭拦截") > seen
        check("S3", "the watchdog steps in while the event thread is stuck", rescued)
        CG.CGWarpMouseCursorPosition(CGPoint(*mid))
        move(mid[0] + 40, mid[1] + 40, 40, 40)
        check("S3", "stuck while on Windows: the watchdog gives the Mac its input back",
              near(cursor(), (mid[0] + 40, mid[1] + 40), 1.5), f"mac {cursor()}")
        handed_back = False
        deadline = time.time() + 7  # the stall ends; the next beat finishes the hand-back
        while time.time() < deadline and not handed_back:
            time.sleep(0.25)
            with open(MAC_LOG, encoding="utf-8") as f:
                handed_back = f.read().count("看门狗恢复后") > seen_back
        check("S3", "once unstuck the event thread finishes the hand-back", handed_back)
        check("S3", "after the stall Windows is told to let go", win("shown") == "hidden")

        # display sleep while Windows has control (RULES.md H4); with an immediate screen
        # lock the input would go to the login window instead, so it is skipped then
        lock = subprocess.run(["sysadminctl", "-screenLock", "status"], capture_output=True, text=True).stderr
        delay = re.search(r"screenLock delay is (\d+) seconds", lock)
        if not ("screenLock is off" in lock or (delay and int(delay.group(1)) >= 30)):
            skip("H4", "display wake while on Windows", f"the screen might lock: {lock.strip()[-60:]!r}")
        else:
            display_wake(b, px)
    finally:
        ctl("resume")
        ctl("set edge=top")
        modifier(*LEFT_COMMAND, False)
        modifier(*LEFT_SHIFT, False)
        set_mac_clip(saved_mac)
        if saved_win:  # an empty clipboard cannot be passed as an argument; leave the test text
            win("clipset", b64(saved_win))
        CG.CGWarpMouseCursorPosition(CGPoint(*home))

    failed = results.count(False)
    print(f"e2e: {len(results) - failed}/{len(results)} passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
