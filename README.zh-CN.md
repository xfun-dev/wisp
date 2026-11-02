<div align="center">

[English](README.md) · **中文**

# 一缕 Wisp

**Mac 的触控板和键盘，直接用在旁边的 Windows 电脑上。**

把光标推过 Mac 屏幕的边缘，它就出现在 Windows 上；再推回来，就回到 Mac。<br>
打字、快捷键、剪贴板都跟着过去。不用第二套键鼠，也不用切换器。

<img src="docs/demo-zh.gif" alt="光标从 Mac 推到上方的 Windows 屏幕，在记事本里打字、复制，回到 Mac 粘贴" width="720">

</div>

## 它能做什么

| | |
|---|---|
| 🖱️ **推过去就到** | 光标越过 Mac 屏幕边缘，直接进入 Windows。另一台屏幕可以放在 Mac 的上方、左侧或右侧。 |
| ⌨️ **键盘照常用** | 按物理位置映射按键。Mac 的 ⌘C、⌘V 在 Windows 上自动变成 Ctrl+C、Ctrl+V，双指缩放变成 Ctrl + 滚轮。 |
| 📋 **剪贴板互通** | 在一边复制，到另一边直接粘贴（纯文本，最大 1 MB）。密码管理器标记为敏感的内容不会同步。 |
| 🪶 **极省内存** | 常驻后台在 Mac 上约 **3 MB**，Windows 上约 **2 MB**。Rust 编写，没有运行时，不是 Electron。 |
| 🔒 **配对后全程加密** | 用 6 位配对码配对（SPAKE2，配对码无法被离线猜出），之后所有数据都用 ChaCha20-Poly1305 加密。局域网里的陌生设备找不到你。 |
| 🧯 **不会卡住你的电脑** | 切换时自动松开按住的键，不会卡键；后台万一卡住超过 1.5 秒，看门狗会把键鼠自动还给 Mac。 |

## 界面

<table>
<tr>
<td align="center" width="50%"><img src="docs/mac-settings.png" alt="Mac 设置窗口：墙与木桌上的 Mac 和另一台屏幕，下方是选择屏幕方向和调速度的木抽屉" width="360"><br><sub><b>Mac 设置窗口</b>：选另一台屏幕在哪边，调鼠标和滚动速度</sub></td>
<td align="center" width="50%"><img src="docs/windows-status.png" alt="Windows 状态窗口：同一张桌子，下方托盘显示连接时长和 Mac 地址" width="360"><br><sub><b>Windows 状态窗口</b>：连接状态，配对时在这里输入配对码</sub></td>
</tr>
</table>

## 三步用起来

1. **Mac 上**：安装后打开 Wisp，在"系统设置 → 隐私与安全性 → 辅助功能"里允许 `wisp`。
2. **Windows 上**：安装后从开始菜单打开 Wisp。
3. **配对**：在 Mac 的 Wisp 里点"配对新电脑"，把出现的 6 位数字填进 Windows 的 Wisp。完成，以后两台电脑开机就会自动连上。

## 支持的平台

| | |
|---|---|
| Mac 控制 Windows（macOS 14+，Windows 10/11） | ✅ |
| Windows 控制 Mac | 计划中 |
| Linux | 计划中 |

## 安装

目前需要从源码构建。Wisp 没有经过 Apple 公证，也没有 Windows 代码签名，第一次打开时系统可能会拦截，需要手动允许（见下文）。

**准备**：macOS 上安装 [Rust](https://rustup.rs)、Xcode 命令行工具，以及交叉编译 Windows 版用的 mingw-w64：

```sh
brew install mingw-w64
rustup target add x86_64-pc-windows-gnu
scripts/verify.sh        # 跑测试，零警告编译 Mac 版、Windows 版和设置窗口
```

**Mac**

1. 在"钥匙串访问 → 证书助理 → 创建证书"里建一个**代码签名**类型的自签名证书，名字叫 `Wisp Dev`（也可以用 `WISP_SIGN_ID` 指定别的名字）。签名固定不变，更新后"辅助功能"权限才不会失效。
2. 运行 `scripts/deploy.sh mac`：安装后台（开机自启）和设置窗口 `/Applications/Wisp.app`。
3. 如果设置窗口被拦下，到"系统设置 → 隐私与安全性"里点"仍要打开"。

**Windows**

把 `target/x86_64-pc-windows-gnu/release/wisp.exe` 复制到那台电脑的 `%LOCALAPPDATA%\Wisp\wisp.new.exe`，连同 `scripts\install.ps1` 一起，然后运行：

```powershell
powershell -ExecutionPolicy Bypass -File install.ps1
```

它会把 Wisp 设成登录时自启，并在开始菜单里加上"Wisp"。如果 Windows Defender 或 SmartScreen 拦截，选择允许，或者把 Wisp 文件夹加入排除项。

## 参与开发

- **[RULES.md](RULES.md)** 是项目的规格书：每条行为规则都有编号，并写明由哪个测试保证。想改行为，先改这里。
- `src/logic/` 是不依赖平台的纯逻辑，在任何机器上都能测试；`server_mac.rs`、`client_win.rs` 等只负责把系统事件翻译进来、把结果执行出去。
- `scripts/verify.sh --live` 和 `scripts/e2e.py` 会检查正在运行的两台电脑：内存预算、连接状态，以及端到端的键鼠和剪贴板。它们通过一个"桥"命令来操作 Windows，任何支持 `put <本地文件> <远程路径>` 和 `exec "<命令>" --timeout N` 的命令都可以。在 `scripts/local.env`（不提交）里配置：

  ```sh
  WISP_WIN_BRIDGE="your-bridge-command"
  WISP_WIN_DIR="C:/Users/<you>/AppData/Local/Wisp"
  ```

## 许可

[MIT](LICENSE)。设置窗口打包的 Fredoka 字体使用 SIL Open Font License，见 [ui/mac/fonts/Fredoka-OFL.txt](ui/mac/fonts/Fredoka-OFL.txt)。
