# Wisp 行为规则

这份文档写的是两端（以及以后新增的平台）**必须一致**的行为。各平台代码各写各的，但都要对齐这里。
改行为时：先改这里，再改代码；能写成测试的规则，在「保证」一栏注明测试名。

## 1. 布局与切屏

| # | 规则 | 保证 |
|---|---|---|
| L1 | 远端屏幕默认在本机屏幕**正上方**，可在设置里改成左、右（U2；没有“下方”，见 L9） | — |
| L2 | 光标在本机屏幕**顶边**并且继续**向上**推，同时有已认证的客户端在线，才切到远端 | `no_hop_without_upward_push_edge_or_client` |
| L3 | 进入远端时，落点在远端**底边**，横向按比例对应（本机最左对应远端最左，最右对应最右） | `edge_maps_proportionally_across_widths` |
| L4 | 在远端底边继续**向下**推就回到本机，回到本机顶边下方 2 点处，横向按比例对应 | `pushing_down_at_the_remote_bottom_returns_to_the_same_column` |
| L5 | **按住任何鼠标键时不切屏**（拖拽不会跨屏，避免某一端的按键卡在按下状态） | `dragging_never_leaves_the_remote_screen` |
| L6 | 远端光标被限制在远端屏幕范围内；位置没变化的移动不发送 | `remote_moves_clamp_scale_and_skip_repeats` |
| L7 | 连接断开或发送失败时，控制权立即回到本机，光标留在原处 | `reset_gives_the_cursor_back` |
| L8 | 不在顶边往上推的移动（占绝大多数），不查询屏幕、不加锁 | `no_hop_without_upward_push_edge_or_client` |
| L9 | 控制对方期间，Mac 指针隐藏并冻结在离开的边缘上。**不提供“下方”布局**：MacBook 底边是程序坞的自动隐藏触发条，指针冻结在那里会被重新显示，屏幕上出现第二个指针（笔记本下方本来也不会放显示器）。曾试过把指针挪到屏幕中央，结果 macOS 把这次挪动算进下一次位移，导致来回反复切屏 237 次，已撤销。已知风险：左/右布局在靠近触发角处离开可能触发触发角，尚未复现 | `settings_round_trip_and_reject_nonsense`（`bottom` 回退为 `top`） |

## 2. 键盘

| # | 规则 | 保证 |
|---|---|---|
| K1 | 线上传的是**物理位置**的 PC set-1 扫描码（扩展键为 0xE0xx），不是字符；输入法由接收端自己处理 | `modifiers_map_by_position` |
| K2 | 发送端如实映射：Command → Win 键位置，Option → Alt，Control → Ctrl | `modifiers_map_by_position` |
| K3 | 「Mac 习惯」由**接收端**决定：Windows 端把 Ctrl 和 Win 对调，所以 ⌘C 就是 Ctrl+C，⌃ 就是 Win 键 | `command_becomes_ctrl_and_back` |
| K4 | 修饰键分左右，按下和松开各发一次；Caps Lock 每次事件按「按下 + 松开」发送 | `flags_tell_press_from_release_per_side` |
| K5 | 映射表里不能有两个键对应同一个扫描码 | `no_two_keys_share_a_scancode` |
| K6 | fn 键不转发。按住 fn 时顶排就是 F1–F12，照常转发 | `flags_tell_press_from_release_per_side` |
| K7 | 顶排功能键（不按 fn）：F7–F12 的上一首、播放/暂停、下一首、静音、音量减/加（以及快进、快退）作为 Windows 的媒体键扫描码转发，Mac 自己的不动；调度中心 → Win+Tab，聚焦搜索/启动台 → Win+S，听写 → Win+H，按下时发一次组合键；**屏幕亮度、键盘背光、勿扰留给 Mac**，即使在控制 Windows 时也由 Mac 处理。媒体键是 NX_SYSDEFINED 事件，按键数据从整数字段读出（子类型 99、data1 149，实测），核心不加载 AppKit。其他认不出的键不转发并记日志 | `media_keys_go_to_windows_and_brightness_stays`、`top_row_shortcuts_become_windows_ones` |

## 3. 鼠标、滚动、手势

| # | 规则 | 保证 |
|---|---|---|
| M1 | 位置一律发**绝对坐标**（远端像素），由发送端计算；接收端换算到 0..65535 | `absolute_covers_the_full_range` |
| M2 | 按键编号：1 左键、2 右键、3 中键、4/5 侧键；其他编号忽略 | `release_all_lets_go_of_everything_once` |
| M3 | 滚动用 Windows 单位（120 = 一格）。触控板按像素 × 系数（默认 3），鼠标滚轮按行 × 120；横向取反 | `wheel_scales_trackpad_points_and_mouse_lines` |
| M4 | 双指缩放：放大倍率累计到 0.15 发一档 `Zoom`，手势结束时清掉余量；接收端用 Ctrl + 滚轮实现，用户自己按着 Ctrl 时不重复按 | `pinch_emits_whole_steps_and_forgets_between_gestures` |

## 3b. 剪贴板（双向、1 MB、不同步敏感内容、不加开关）

| # | 规则 | 保证 |
|---|---|---|
| CB1 | **双向**：光标进入远端时，把本机剪贴板带过去；光标回到本机时，把远端剪贴板带回来。只在切屏这一刻同步，平时不监听 | — |
| CB2 | 只同步**纯文本**（UTF-8），单次上限 **1 MB**；超出上限这次不同步，日志记一笔 | `clipboard_sends_only_new_text_within_the_limit`、`roundtrip_over_tcp` |
| CB2b | 换行在线上统一为 `\n`；Windows 端读取时把 `\r\n` 换成 `\n`，写入时换回 `\r\n` | `line_breaks_travel_as_lf_and_land_as_crlf_on_windows` |
| CB3 | 密码管理器标记为敏感的内容**不同步**：Mac 上带 `org.nspasteboard.ConcealedType`，Windows 上带 `ExcludeClipboardContentFromMonitorProcessing` 格式 | — |
| CB4 | 内容没变不重发；从对方收到的内容不会再发回去（不回声） | `clipboard_never_echoes_what_it_received` |
| CB5 | 不提供开关（没人用的开关不加） | — |
| CB6 | 读写剪贴板不在输入事件回调线程里做，不拖慢键鼠；而且**不在核心进程里做**：每次读写都启动一个临时的 `wisp pasteboard read/write` 子进程，用完即退。实测剪贴板框架一旦用过会让进程固定多占约 1 MB（核心 3 MB → 4.1 MB，超出预算），所以放进子进程。超过 1 MB 的内容由子进程直接回报“太大”，不传给核心 | `helper_output_round_trips` |

## 4. 不卡键

| # | 规则 | 保证 |
|---|---|---|
| H1 | 接收端记录所有按下的键和鼠标键；光标离开（`Leave`）或连接断开时，**全部松开**，按键按后按下的先松开 | `release_all_lets_go_of_everything_once` |
| H2 | 自动连发的重复按下只记一次，松开只发一次 | `release_all_lets_go_of_everything_once` |
| H3 | 控制权回到 Mac 时，**隐藏 Windows 的指针**，切回 Windows 或连接断开时再显示。做法：在指针下放一个 64×64、几乎全透明、置顶、不抢焦点的小窗口，它声明“这里不显示指针”。不改系统的指针方案，所以 Wisp 崩溃或退出时窗口随之消失，指针立刻恢复；本地鼠标一移出这块区域也会显示 | e2e `H3` |
| H4 | Windows 有控制权时，Mac 指针必须一直冻结并隐藏。macOS 会自己撤销这两件事（2026-11-01 实验证实：息屏再亮屏后指针被重新显示，出现两个能动的指针）：每个事件都检查 Mac 指针是否在动（切屏后 150 ms 内的移动算正常漂移，作为新起点），每 250 ms 检查一次是否可见，发现就重新冻结并隐藏，不把指针挪回原位（L9）。隐藏/显示按屏幕上的实际状态来做（可见才隐藏、不可见才显示），因为系统会重置隐藏计数 | e2e `H4`（会让 Mac 息屏几秒） |

## 4b. 输入安全

起因：部署新设置窗口时，Mac 的触控板、键盘全部失灵，重启后仍然如此，进入安全模式才恢复。设置窗口单独运行时（有自动恢复脚本保护）已排除；当时的结构是“先开会吞事件的拦截，再做初始化”，并且光标在 Mac 上时拦截也处在可吞事件的模式，所以后台任何卡顿都会卡住整台 Mac 的输入。

| # | 规则 | 保证 |
|---|---|---|
| S1 | 后台**先完成全部初始化**（端口、电脑名、令牌、线程），最后才创建拦截并进入事件循环；启动每一步写一行带毫秒的日志（`启动 +Nms：…`） | 代码顺序；日志 |
| S2 | 光标在 Mac 上时只有**旁听式**拦截（listen-only，只看鼠标移动），它按系统设计不能阻塞输入；会吞事件的拦截创建后立刻关闭，**只在 Windows 有控制权时打开**，控制权回到 Mac 时立刻关闭 | e2e `S2` |
| S3 | **看门狗**：独立线程每 0.5 秒检查一次主线程心跳（心跳和看门狗只在 Windows 有控制权时运行，光标在 Mac 上时都休眠，省电）；Windows 有控制权时主线程超过 1.5 秒没响应，就关掉吞事件的拦截、恢复并显示 Mac 指针、写日志；主线程恢复后再通知 Windows 松开按键 | e2e `S3` |
| S4 | 只供测试的命令 `wisp ctl debug-stall 毫秒`（最长 10 秒）让主线程停住，用来验证 S2/S3；不出现在任何界面里 | `control_commands_parse_and_reject_unknown` |

注：S2/S3 的 e2e 用合成事件（CGEventPost）验证，走的是同一层 HID 拦截，但不等于真实触控板；真实设备要人工试。看门狗和主线程谁把 REMOTE 从 true 改成 false，谁负责恢复指针（隐藏/显示是计数的，只能做一次）。

## 5. 连接与安全

| # | 规则 | 保证 |
|---|---|---|
| C1 | 默认端口 24801（不与 Deskflow 的 24800 冲突），TCP 传输、UDP 发现共用这个端口号；Windows 主动连接 Mac | — |
| C2 | 握手交换随机盐，每个方向各自派生会话密钥；ChaCha20-Poly1305 加密，nonce 严格递增（重放、乱序直接断开） | `roundtrip_over_tcp`、`wrong_password_is_rejected` |
| C3 | 对方的第一帧解密成功（证明知道密码）之前，不给它开线程、不发心跳、不登记为当前连接 | — |
| C4 | 空闲 2 秒发一次心跳；6 秒收不到任何东西就判定断开；心跳跟连接绑定，不跟内部队列绑定 | `roundtrip_over_tcp` |
| C5 | 改变任何消息的格式或含义，都必须**升 `proto::VERSION`**（当前 4），并且两端一起部署 | — |
| C6 | 只接受合法长度的帧（最长 = 1 MB 剪贴板 + 头尾），超长帧读到长度前缀就拒绝；键鼠这类小消息收发都不分配堆内存 | `oversized_frames_are_rejected_before_reading` |
| C7 | 密钥只来自配对（P3、P7），不出现在命令行或开机启动命令里 | — |
| C8 | 找 Mac 的顺序：`--server` 手动指定 → 上次连上的地址 → 局域网广播查询，等 1 秒。广播发往 255.255.255.255 **以及本机每个私有局域网地址所在 /24 的网段广播**——代理/VPN 的 TUN 网卡（如 Clash 的 198.18.x）常常占着默认路由并吞掉 255.255.255.255，网段广播走局域网路由不受影响；169.254 和公网地址跳过 | `broadcasts_reach_every_private_lan_but_skip_tunnels_and_link_local` |
| C9 | 发现查询和回答都带由密码派生的认证标签（截断 SHA-256），回答还要对上查询里的随机数；不知道密码的设备收不到回答，也伪造不了回答。等待回答期间每 300 ms 重发一次查询：Wi‑Fi 上的广播会丢（Mac 无线网卡休眠时尤其），只发一次曾让配对偶尔失败 | `discovery_answers_only_the_password_holder`、`discovery_rejects_forged_or_replayed_answers`、`a_server_with_another_password_stays_silent`、`a_lost_query_is_sent_again` |
| C10 | 客户端只发出站流量（广播查询 + 等待单播回复），Windows 防火墙不需要入站规则 | `finds_the_server_and_its_tcp_port` |
| C11 | 同一个连接失败原因只记一次日志，直到成功连上为止（Mac 不在时不刷日志） | — |

## 5b. 设置与界面

| # | 规则 | 保证 |
|---|---|---|
| U1 | **Mac 默认不常驻图标**：界面是单独的进程，从启动台按需打开，关掉就退出（实测最小的 Mac 菜单栏图标就要 10 MB，核心才 2.8 MB）；菜单栏图标是可选项，见 U12。**Windows 有托盘图标**（否则关掉状态窗口后什么都看不到；实测托盘图标只多约 0.19 MB），见 U10；状态窗口仍按需打开 | — |
| U2 | 第一版设置只有四项：**连接状态、屏幕方向（上/左/右）、鼠标速度和滚动速度、暂停切屏**；配对、开机自启开关等以后真需要再加 | — |
| U3 | 设置属于控制方（Mac），存在 Mac 数据目录的 `config` 文件里，修改后立即生效，不用重启 | `settings_round_trip_and_reject_nonsense` |
| U4 | **暂停不保存**：Wisp 或电脑重启后自动恢复切屏；暂停期间光标不会切到对方，已经在对方屏幕上的不受影响 | — |
| U5 | Windows 窗口只显示连接状态（是否连上、连的哪台 Mac、连不上的原因），从开始菜单“Wisp”打开（`wisp.exe status`），每秒刷新，关掉就退出。**打开时如果后台没在运行，先把后台启动起来**（否则从托盘退出后，开始菜单只能看到“没有在运行”，启动不回来） | `status_window_words_for_each_state` |
| U9 | Mac 连上后发送自己的“电脑名称”（系统设置里的名字，最多 128 字节，按字符截断），Windows 状态窗口显示“已连接 <名字>”，下方托盘显示连接时长和 Mac 地址 | `names_are_cut_on_a_character_boundary`、`status_window_words_for_each_state` |
| U10 | Windows 托盘图标由后台进程画出（不带图片文件）：**绿点**=已连接、**橙点**=寻找中；悬停显示“Wisp：<状态窗口第一行>”；单击打开状态窗口（已开着就切到前面）；右键菜单只有“打开状态窗口”“退出 Wisp”（退出时连同打开着的状态窗口一起关掉；只有后台意外崩溃时状态窗口才会留着显示“没有在运行”；下次登录或从开始菜单打开时再启动）；资源管理器重启后自动重新加上。与指针隐藏共用同一个界面线程 | `tray_dot_is_round_coloured_and_premultiplied` |
| U11 | **主动退出两端对齐**：Mac 在设置窗口底部点“退出 Wisp”（或 `wisp ctl quit`），后台以正常方式退出并保持退出，设置窗口和菜单栏图标（U12）一起关掉；菜单栏图标里的“退出 Wisp”同样关掉后台和设置窗口；Windows 从托盘退出（U10）。再从启动台/开始菜单打开 Wisp 时，后台没在运行就先启动它。意外崩溃仍自动重启（LaunchAgent 的 KeepAlive 只在非正常退出时生效），下次登录照常自启 | `control_commands_parse_and_reject_unknown` |
| U12 | **Mac 菜单栏图标可选，默认关闭**：设置窗口“菜单栏图标”开关打开后，单独的小进程 `WispMenu`（只用 AppKit，`~/Library/Application Support/Wisp/WispMenu`，LaunchAgent `fun.xfun.wisp.menu`）立即出现并随登录启动；关掉即移除。图标是“一缕”标志的单色模板图（随菜单栏显示为白或黑，与系统图标一致），暂停或没在运行时变淡；菜单：状态、暂停切屏（或“启动 Wisp”）、打开 Wisp 设置…、退出 Wisp。它每 2 秒直接经控制端口（U6）问一次状态，不另起进程。实测约 10 MB、CPU 0.1%，开关旁注明多占内存 | 实测：菜单内容、暂停切换生效 |
| U8 | Windows 后台进程每次状态变化时整份重写 `%LOCALAPPDATA%\Wisp\status`（先写临时文件再改名），并在整个生命周期持有命名互斥量 `Local\WispClient`；窗口靠互斥量判断后台是否在运行，所以后台崩溃后旧文件不会被当成“已连接” | `status_file_round_trips`、`status_window_words_for_each_state` |
| U6 | 界面通过本机控制端口 `127.0.0.1:24802` 跟核心通信，只接受本机连接，用本机令牌 `ctl-token` 认证（P7）；协议是一行一条的文本命令（`status` / `set k=v` / `pause` / `resume` / `quit` / `pair` / `pair-cancel` / `unpair`），状态行用制表符分隔字段（名字里有空格），命令行工具 `wisp ctl` 也走这里 | `control_commands_parse_and_reject_unknown` |
| U7 | 数值有安全范围：速度 0.2–5，滚动 0.5–10；超出范围或格式不对的设置被拒绝，不会写进配置 | `settings_round_trip_and_reject_nonsense` |

## 5c. 配对（2 分钟/错 3 次、只配一台、重新配对一次、可解除）

| # | 规则 | 保证 |
|---|---|---|
| P1 | Mac 生成 **6 位配对码**，有效 **2 分钟**，**输错 3 次作废**；用过一次即作废；同一时间只有一个配对码 | `a_code_lasts_two_minutes_and_three_wrong_guesses`、`three_wrong_codes_void_the_window` |
| P2 | Windows 广播“谁在等待配对”，只有开着配对码的 Mac 回答（只给名字和端口）；其他时候 Mac 不回答这种查询 | `a_mac_waiting_to_pair_says_its_name_and_only_then` |
| P3 | 用 **SPAKE2** 把配对码变成共享秘密（抓包也无法离线猜码）；双方各自证明算出了同一个秘密，并把两台电脑的名字绑进证明，**确认后才保存**；保存的是由秘密派生的 32 字节密钥，秘密和配对码都不传输 | `the_right_code_gives_both_sides_the_same_key` |
| P4 | **一台 Mac 只配一台 Windows**：新的配对成功后替换旧的，并断开旧连接；配对失败不影响现有配对 | — |
| P5 | Mac 可以**解除配对**：删掉密钥文件、断开连接；之后不接受连接、不回答发现查询，直到重新配对 | `pairing_commands_open_a_code_and_unpair_forgets` |
| P6 | 未配对时：Mac 不接受普通连接、不回答带密钥的发现查询；Windows 状态显示“还没有配对”，每 2 秒检查一次配对文件，配对后自动连上 | — |
| P7 | 密钥存在各自数据目录的 `pair` 文件（名字 + 密钥，先写临时文件再改名；Mac 上权限 600）。设置窗口和 `wisp ctl` 用 Mac 本机单独的随机令牌 `ctl-token`，与配对无关，所以未配对时也能发起配对。旧的共享密码文件 `key` 不再使用 | `pair_file_round_trips_and_rejects_broken_keys`、`the_local_token_is_made_once_and_kept` |
| P8 | **Windows 状态窗口**在没连上时（未配对或找不到 Mac）显示配对码输入框：未配对提示“输入 Mac 上显示的 6 位配对码”，找不到 Mac 时提示“要重新配对？…”；只能输数字、最多 6 位，回车或点“配对”；配对在后台线程进行，窗口不卡，结果用绿/红字显示；连上后输入框自动收起。**Mac 设置窗口**：未配对显示“配对新电脑”；配对中大字显示配对码、倒计时、剩余次数和“取消配对”；已配对显示“已与 X 配对”和“解除配对”（先确认），“配对新电脑”旁注明会替换现有配对 | `status_window_words_for_each_state` |

## 5d. 品牌（「一缕」）

| # | 规则 | 保证 |
|---|---|---|
| B1 | Logo 是一笔画出的 W，结尾卷成一缕烟；母版在 `brand/glyph.svg`、`brand/app-icon.svg`。**所有图标都由 `scripts/brand.swift` 从同一份路径数据渲染**：Mac 应用图标 `ui/mac/AppIcon.icns`、Windows 图标 `ui/win/wisp.ico`（编进 exe）、托盘位图 `src/assets/tray-glyph.bin`（16–48 px，托盘版去掉卷尾、放大 1.2 倍）。改 Logo 只改那一处路径，再重新运行脚本 | `tray_glyph_comes_in_every_tray_size_and_tints_premultiplied` |
| B2 | 颜色：墨 `#0C1018`、青光 `#2FD5BE`（已连接）、琥珀 `#F2A33A`（寻找中/需要注意）、雾 `#F6F7F9`；状态色靠亮度也能区分，不只靠色相 | — |
| B4 | Mac 设置窗口是「桌面模型」：上半是墙与木桌上的黏土 Mac 和另一台屏幕，屏幕由夹在桌沿、底座不动的两节机械臂托着，换布局时关节弯折、屏幕弹过去；已连接时两机之间是流动的青色光点。下半是木抽屉面板：刻进木板的托盘选位置（迷你 Mac 与“上方”在同一竖线）、凹槽滑轨配黏土珠子调速度、刻字链接。固定暖木配色，不随系统深色模式变；数字与拉丁字母用 Fredoka（SIL OFL，打包在应用里，许可在 `ui/mac/fonts/`），中文用系统字体。动画全部交给 Core Animation，窗口开着几乎不占 CPU | 截图核对三种布局 |
| B5 | Windows 状态窗口是同一张桌子：这台电脑的屏幕挂在机械臂上、Mac 在下；下半木抽屉里**始终是同一块凹槽托盘**（位置大小不变，切换状态只换里面的字）：已连接时显示连接时长与 Mac 地址，配对时是 6 个配对码凹槽（数字落成黏土键帽），没在运行时写明原因；底部一行左边是这个状态的操作（凸起的黏土拉手或刻字链接），右边“退出 Wisp”。**能点的才凸起，只看的都内嵌**。GDI+ 画进缓存位图，只重画会动的部分（20 fps，最小化时停）；标题栏用 DWM 染成墙色 | 截图核对；`a_click_survives_the_capture_change_sent_while_releasing` |

## 5e. 安装与发布

| # | 规则 | 保证 |
|---|---|---|
| I1 | Windows 只发一个 `Wisp-<版本>.exe`，它自己就是安装程序：双击（或 `wisp install`）把自己复制到 `%LOCALAPPDATA%\Wisp\wisp.exe`，写登录自启（Run 键）、开始菜单“Wisp”（打开状态窗口）、“设置 → 应用”里的卸载项，启动后台并打开状态窗口；已在运行的旧版本先退出再替换。`--quiet` 不弹窗口（`scripts/deploy.sh` 用） | 实机安装 |
| I2 | `wisp uninstall`（“设置 → 应用”里的卸载）退出后台、删除自启、开始菜单项、卸载项和整个 Wisp 文件夹 | 实机卸载 |
| I3 | 安装器启动的进程不继承安装器的输入输出，等待安装器的脚本不会被常驻后台拖住；创建快捷方式用 IShellLink，ole32.dll 按需加载（R6） | — |
| I4 | Mac 只发一个 DMG，里面的 `Wisp.app` 自带后台（`Contents/Helpers/wisp`），两者都是 Apple 芯片 + Intel 的通用二进制。每次打开 Wisp，它比较自带的后台和已安装的（`~/Library/Application Support/Wisp/wisp`）以及 LaunchAgent：不同就停掉旧的、换上新的、重新注册；相同就什么都不做。`Wisp --install` 只做这一步然后退出（`scripts/deploy.sh` 用）。签名身份不变（D4），所以更新后“辅助功能”权限仍然有效 | 实测：首次安装、覆盖更新、无变化时不重启 |
| I6 | Mac 卸载：设置窗口菜单“Wisp → 卸载 Wisp…”确认后，停止后台，删除 LaunchAgent、后台程序、`~/.config/wisp`（配对与设置）和日志，把 Wisp.app 移到废纸篓；`Wisp --uninstall` 不询问直接执行。系统设置“辅助功能”里的 wisp 条目会留下（macOS 按路径记录，普通权限删不掉），重新安装后授权沿用 | 实测：卸载后无进程、无残留文件 |
| I5 | `scripts/release.sh` 跑完验证后在 `dist/` 生成 `Wisp-<版本>.dmg` 和 `Wisp-<版本>.exe` | — |

## 5f. 开发者通道（仅开发版）

| # | 规则 | 保证 |
|---|---|---|
| X1 | 开发者通道让已配对的 Mac 在 Windows 上执行命令、收发文件，供部署和测试使用，取代外部远程工具。**只在 `bridge` 编译开关打开时编入**。普通版不带；Releases 另外提供开发者版（文件名带 `-dev`），由用户按需下载，两端都装开发者版才能使用。`scripts/deploy.sh` 给自己的机器装开发者版。`scripts/release.sh` 一次打两套，并按字节检查：普通版里有通道代码、或开发者版里没有，都拒绝打包 | release.sh 的检查 |
| X2 | 开启方式：Mac 设置窗口里点两下花盆、一下茶杯（3 秒内），再点一次同样的顺序关闭；或 `wisp win on/off`。只有后台带通道时（状态里有 `bridge=` 字段）这个手势才有反应 | 实测 |
| X3 | 开了就一直开着，直到用户关闭，不限时（用户 2026-11-11）。开关记在 Mac 数据目录的 `bridge` 文件里，Mac 后台重启、断线重连后保持原样；Windows 每次新连接先当作关闭，等 Mac 告知。Windows 只在开启期间执行请求，否则拒绝。命令在独立线程里执行，不耽误键鼠。文件按 12 KB 一块传，每块等对方确认再发下一块，任何时候只有一块在途：大块或不限速会让两端内存上涨（实测 768 KB 块让 Mac 后台到 5 MB；16 KB 块让 Windows 每次多 260 KB 且持续增长；12 KB 只多 56 KB 且稳定）。连接一断，所有等待中的请求立即失败 | `every_bridge_message_round_trips`、`the_bridge_stays_as_the_mac_last_set_it`、`a_piece_fits_well_inside_a_frame` |
| X4 | 命令行：`wisp win put <本地> <远程>`、`get <远程> <本地>`、`exec "<命令>" [--timeout 秒]`（经 cmd.exe，输出与错误合并，最多 64 KB，退出码原样返回，超时为 -2）。`scripts/local.env` 的 `WISP_WIN_BRIDGE` 指向它 | 实测 |
| X5 | 通道开着时一定看得见：Mac 茶杯的热气变青色、墙上写“开发者通道已开启”；Windows 状态窗口同样显示 | `an_open_bridge_is_always_on_show` |

## 5g. 锁屏和权限弹窗（Windows 系统服务，可选）

| # | 规则 | 保证 |
|---|---|---|
| W1 | 托盘菜单「锁屏和权限弹窗也能控制」，**默认关**。勾选或取消都由用户身份的 `wisp.exe service-switch on/off` 请求一次管理员授权（UAC）。开机自启和普通客户端属于这个用户，由未提权的一方处理；提权进程不读写用户文件夹（批准的管理员可能是另一个账号，文件夹也可能被做成链接）。失败则保持原样 | 实测 |
| W2 | 开启后 WispService（LocalSystem，自动启动）在控制台会话以 SYSTEM 身份运行 `wisp.exe client --system`，客户端每 300 ms 及每次注入失败时切到当前输入桌面，锁屏、登录界面、UAC 弹窗里都能用 Mac 键鼠。**SYSTEM 只读写、只运行管理员才能改的位置，不碰用户文件夹里的任何文件**：程序在 `Program Files\Wisp`（位置取自 HKLM 注册表，不看环境变量），状态、日志在其中的 `state`（用户只读，无法抢先创建），配对密钥在 `state\key`（只有 SYSTEM 和管理员能读，否则本机其他账号可冒充 Mac）。配对由未提权的一方作为参数交给提权进程写入；服务开着时在状态窗口重新配对，需要再授权一次才同步给 SYSTEM 客户端 | 实测 |
| W3 | SYSTEM 客户端放行来自用户进程的托盘消息（`ChangeWindowMessageFilterEx`），运行标记（互斥量）允许所有人等待；状态窗口读两处状态文件里较新的那份，所以托盘、状态窗口、安装程序看到的状态与普通模式一致 | 实测 |
| W4 | 从 SYSTEM 托盘打开的状态窗口、发起的服务开关，都以登录用户身份启动；拿不到用户环境时拒绝，不带 SYSTEM 的环境启动 | 实测 |
| W5 | 开着服务时更新需要一次 UAC：安装程序先更新用户自己的副本（不写 Run 键、不让服务的客户端退出），再提权重装服务的文件。开发版另有一条路：已配对的 Mac 在通道开启时 `wisp win put <exe> wisp:update`，SYSTEM 客户端把它完整写进 `state` 后才改名为 `wisp.next.exe`，服务停下客户端换上新版；换不上则放回旧版（旧版只在新版到位后才删除），且不反复重试。重装时文件复制完才把服务指向新位置，复制失败旧服务照常运行；结果写入 `service.log` | `an_update_replaces_the_client`、`an_update_that_cannot_move_in_leaves_the_old_client` |
| W6 | 用户退出时 SYSTEM 客户端以退出码 `0x5157` 结束，服务不再拉起；状态窗口「启动 Wisp」、开始菜单等启动普通客户端时，先向服务发唤醒控制码，服务在运行就由它启动，否则照常运行普通客户端，任何时候都不会一个客户端都没有。服务每次启动（开机）以及有用户登录时都会重新启动客户端；SYSTEM 客户端启动时若发现已有普通客户端，让它退出；客户端崩溃或换了控制台会话，服务重新启动它 | `the_service_keeps_one_client_in_the_console_session`、`a_quit_keeps_the_client_stopped_until_started_again` |
| W7 | 开发者通道在服务模式下**以登录用户身份**执行命令、读写文件；没有用户登录时（开机登录界面）拒绝。命令的输出管道一次只建一个，不会被同时运行的命令继承。用到的 wtsapi32、userenv 每次调用完即卸载：常驻不卸载时客户端多占约 280 KB | 实测 |
| W8 | 卸载时若服务在，先请求管理员授权移除服务及 `Program Files\Wisp`，授权被拒则什么都不删 | 实测 |

## 6. 资源预算（产品卖点，视同功能）

| # | 规则 |
|---|---|
| R1 | 常驻核心进程内存上限：**Mac ≤ 4 MB**（phys_footprint），**Windows ≤ 2 MB**（独占内存）。每次改动后实测，超了要么优化，要么说明理由。开发者版通过开发者通道测量时，读数包含测量请求本身（一个线程、一个管道、输出缓冲，实测约 70–130 KB），这时 Windows 允许多 128 KB；普通版预算不变 |
| R2 | 热路径（每个输入事件都会走的代码）不做堆分配、不做系统查询，除非确有必要 |
| R3 | 新增依赖前先评估它对体积和内存的影响；能用标准库就用标准库 |
| R4 | 界面是**单独的进程，按需启动**，关掉就退出，不计入常驻内存 |
| R5 | 线程之间传消息用**预先分配的有界队列**（512 条），不用无界队列（实测会让 Mac 多占 0.2–0.4 MB）；事件回调往队列放消息绝不等待，队列满 = 连接卡死，按断开处理把控制权交回 Mac。控制端口单线程依次处理请求、256 字节缓冲、2 秒超时，不为每个请求开线程。（2026-11-01 排查：Mac 运行一段时间后从 2.8 MB 涨到 4.5 MB；堆上活跃始终约 0.5 MB，增长来自若干处反复申请/释放留下的内存页，各自会封顶；主动归还空闲页无效） |
| R6 | 后台和状态窗口是同一个 wisp.exe：只有窗口用得到的系统库（gdiplus.dll、dwmapi.dll）**不静态链接**，打开窗口时从 System32 动态加载，否则后台启动就会多占约 130 KB | `verify.sh --live` 的 Windows 内存预算 |

## 7. 开发约定

| # | 规则 |
|---|---|
| D1 | 所有代码自己写，**不复制 Deskflow 等 GPL 代码**（可以参考原理） |
| D2 | 判断逻辑放在 `src/logic`（不依赖平台，在 Mac 上就能测试）；平台文件只负责翻译事件和执行结果 |
| D3 | 用 `scripts/deploy.sh` 部署：先跑 `scripts/verify.sh`，再编译、签名、两端安装，最后跑 `verify.sh --live` 检查连接和内存预算 |
| D5 | 提交说明写清改了什么、怎么验证、怎么回滚 |
| D6 | 发布前跑 `scripts/e2e.py`（黑盒：Mac 发合成输入，读 Windows 真实的光标/按键/剪贴板，对照规则编号）。它会实际操作两台电脑约 30 秒：**运行期间不碰触控板和键盘**，否则结果无效。别的剪贴板同步软件（如微信输入法“跨设备复制粘贴”）开着时，剪贴板项会 SKIP |
| D4 | Mac 端用固定的签名证书（`scripts/deploy.sh` 里的 `SIGN_ID`，默认「Wisp Dev」，自己在钥匙串里建一个自签名代码签名证书即可）和标识 `fun.xfun.wisp` 签名；改了它们就得重新授予“辅助功能”权限 |

## 平台对照

| 功能 | macOS（发送） | Windows（接收） | 说明 |
|---|---|---|---|
| 切屏（上下） | ✅ | ✅ | |
| 鼠标、按键、滚动 | ✅ | ✅ | |
| 双指缩放 | ✅ | ✅ Ctrl+滚轮 | |
| 媒体键（音量、播放）、调度中心/搜索/听写 | ✅ | ✅ | 亮度、背光、勿扰留给 Mac（K7） |
| 剪贴板（纯文本，双向） | ✅ | ✅ | 切屏时同步，≤1 MB，不同步敏感内容 |
| 自动发现 | ✅ 回答查询 | ✅ 广播查询 | 也可用 `--server` 手动指定 |
| 配对 | ✅ 设置窗口 | ✅ 状态窗口 | 也可用命令 `wisp ctl pair` / `wisp.exe pair <码>` |
| 反向（Windows 控制 Mac） | — | — | 以后再做 |
| 锁屏、登录界面、UAC 弹窗里的输入 | — | ✅ 可选 | 托盘里开启，装成系统服务（W1–W8） |
| 开机自启 | ✅ LaunchAgent | ✅ 注册表 Run 键 | Windows 的 Wisp 文件夹需加入 Defender 排除项（签名之前） |
