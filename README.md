<div align="center">

**English** · [中文](README.zh-CN.md)

# Wisp

**Use your Mac's trackpad and keyboard on the Windows PC next to it.**

Push the cursor past the edge of your Mac's screen and it appears on Windows; push it back and you're on the Mac again.<br>
Typing, shortcuts and the clipboard come along. No second keyboard, no switch box.

<img src="docs/demo-en.gif" alt="The cursor moves from the Mac up onto the Windows screen above it, types and copies in Notepad, then pastes back on the Mac" width="720">

</div>

## What it does

| | |
|---|---|
| 🖱️ **Just push it over** | The cursor crosses the edge of the Mac's screen straight onto Windows. The other screen can sit above, left of or right of the Mac. |
| ⌨️ **Your keyboard, as is** | Keys map by physical position. ⌘C and ⌘V on the Mac become Ctrl+C and Ctrl+V on Windows; pinch-to-zoom becomes Ctrl + scroll. |
| 📋 **One clipboard** | Copy on one side, paste on the other (plain text, up to 1 MB). Anything a password manager marks as sensitive is never synced. |
| 🪶 **Featherweight** | About **3 MB** of memory on the Mac and **2 MB** on Windows. Written in Rust: no runtime, no Electron. |
| 🔒 **Encrypted once paired** | Pair with a 6-digit code (SPAKE2, so the code can't be guessed offline); everything after that is ChaCha20-Poly1305 encrypted. Strangers on your network can't find you. |
| 🧯 **Never locks you out** | Keys held while crossing are released, so nothing gets stuck. If the background process ever stalls for 1.5 s, a watchdog hands the keyboard and mouse back to the Mac. |

## The windows

<table>
<tr>
<td align="center" width="50%"><img src="docs/mac-settings.png" alt="Mac settings window: the Mac and the other screen on a wooden desk, with a drawer below for the screen side and speeds" width="360"><br><sub><b>Mac settings</b>: where the other screen is, mouse and scroll speed</sub></td>
<td align="center" width="50%"><img src="docs/windows-status.png" alt="Windows status window: the same desk, with the connection time and the Mac's address in the tray below" width="360"><br><sub><b>Windows status</b>: the connection, and where you type the pairing code</sub></td>
</tr>
</table>

The interface is in Chinese for now; English is on the way.

## Up and running in three steps

1. **On the Mac**: install and open Wisp, then allow `wisp` under System Settings → Privacy & Security → Accessibility.
2. **On Windows**: install, then open Wisp from the Start menu.
3. **Pair**: in Wisp on the Mac, click "配对新电脑" (pair a new computer) and type the 6 digits it shows into Wisp on Windows. That's it: from now on the two find each other on their own.

## Platforms

| | |
|---|---|
| Mac controls Windows (macOS 14+, Windows 10/11) | ✅ |
| Windows controls Mac | Planned |
| Linux | Planned |

## Install

For now Wisp is built from source. It is not notarized by Apple and the Windows exe is not code-signed, so the first launch may be blocked until you allow it (see below).

**You'll need** [Rust](https://rustup.rs), the Xcode command line tools, and mingw-w64 to cross-compile for Windows:

```sh
brew install mingw-w64
rustup target add x86_64-pc-windows-gnu
scripts/verify.sh        # tests, plus warning-free builds of the Mac, Windows and settings-window binaries
```

**Mac**

1. In Keychain Access → Certificate Assistant → Create a Certificate, make a self-signed **Code Signing** certificate named `Wisp Dev` (or set `WISP_SIGN_ID` to another name). A stable signature keeps the Accessibility permission valid across updates.
2. Run `scripts/deploy.sh mac`: it installs the background process (starts at login) and the settings window `/Applications/Wisp.app`.
3. If the settings window is blocked, click "Open Anyway" under System Settings → Privacy & Security.

**Windows**

Copy `target/x86_64-pc-windows-gnu/release/wisp.exe` to `%LOCALAPPDATA%\Wisp\wisp.new.exe` on the PC, together with `scripts\install.ps1`, and run:

```powershell
powershell -ExecutionPolicy Bypass -File install.ps1
```

It makes Wisp start at login and adds "Wisp" to the Start menu. If Windows Defender or SmartScreen objects, allow it or add the Wisp folder to the exclusions.

## Contributing

- **[RULES.md](RULES.md)** is the spec (in Chinese): every behaviour has a numbered rule and the test that guarantees it. To change behaviour, change it there first.
- `src/logic/` is platform-independent logic, testable on any machine; `server_mac.rs`, `client_win.rs` and friends only translate OS events in and carry results out.
- `scripts/verify.sh --live` and `scripts/e2e.py` check the two running machines: memory budgets, the connection, and keyboard, mouse and clipboard end to end. They reach Windows through a "bridge" — any command with `put <local file> <remote path>` and `exec "<command>" --timeout N`. Configure it in `scripts/local.env` (not committed):

  ```sh
  WISP_WIN_BRIDGE="your-bridge-command"
  WISP_WIN_DIR="C:/Users/<you>/AppData/Local/Wisp"
  ```

## License

[MIT](LICENSE). The Fredoka font bundled with the settings window is under the SIL Open Font License, see [ui/mac/fonts/Fredoka-OFL.txt](ui/mac/fonts/Fredoka-OFL.txt).
