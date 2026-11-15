//! The words people see, in five languages (RULES.md U14): English, Simplified and
//! Traditional Chinese, Japanese and Korean. The language follows the system, read
//! once; anything else gets English. `WISP_LANG` (en, zh-Hans, zh-Hant, ja, ko)
//! overrides it, for testing. Logs and developer-only messages are English and not
//! here. Every text is a constant compiled in: nothing is loaded at run time.

use std::fmt::Display;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lang {
    En,
    ZhHans,
    ZhHant,
    Ja,
    Ko,
}

/// A language tag such as "zh-Hans-CN", "zh-TW", "ja-JP" or "en-US".
pub fn pick(tag: &str) -> Lang {
    let t = tag.to_ascii_lowercase().replace('_', "-");
    if t.starts_with("zh") {
        let traditional = t.contains("hant") || t.ends_with("-tw") || t.ends_with("-hk") || t.ends_with("-mo");
        if traditional { Lang::ZhHant } else { Lang::ZhHans }
    } else if t.starts_with("ja") {
        Lang::Ja
    } else if t.starts_with("ko") {
        Lang::Ko
    } else {
        Lang::En
    }
}

/// A Windows LANGID (GetUserDefaultUILanguage).
#[cfg(any(test, windows))]
pub fn pick_langid(id: u16) -> Lang {
    match (id & 0x3FF, id >> 10) {
        // Chinese: Taiwan, Hong Kong, Macao are traditional; China, Singapore simplified
        (0x04, 0x01 | 0x03 | 0x05) => Lang::ZhHant,
        (0x04, _) => Lang::ZhHans,
        (0x11, _) => Lang::Ja,
        (0x12, _) => Lang::Ko,
        _ => Lang::En,
    }
}

pub fn lang() -> Lang {
    static LANG: OnceLock<Lang> = OnceLock::new();
    *LANG.get_or_init(|| {
        if cfg!(test) {
            // the tests read the Chinese words
            return Lang::ZhHans;
        }
        match std::env::var("WISP_LANG") {
            Ok(tag) if !tag.is_empty() => pick(&tag),
            _ => system(),
        }
    })
}

#[cfg(target_os = "macos")]
fn system() -> Lang {
    use std::ffi::c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFLocaleCopyPreferredLanguages() -> *const c_void;
        fn CFArrayGetCount(a: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(a: *const c_void, i: isize) -> *const c_void;
        fn CFStringGetCString(s: *const c_void, buf: *mut u8, size: isize, encoding: u32) -> bool;
        fn CFRelease(cf: *const c_void);
    }
    unsafe {
        let list = CFLocaleCopyPreferredLanguages();
        if list.is_null() {
            return Lang::En;
        }
        let mut buf = [0u8; 64];
        let found = CFArrayGetCount(list) > 0
            && CFStringGetCString(CFArrayGetValueAtIndex(list, 0), buf.as_mut_ptr(), buf.len() as isize, 0x0800_0100);
        CFRelease(list);
        if !found {
            return Lang::En;
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        pick(std::str::from_utf8(&buf[..end]).unwrap_or(""))
    }
}

#[cfg(windows)]
fn system() -> Lang {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    pick_langid(unsafe { GetUserDefaultUILanguage() })
}

#[cfg(not(any(windows, target_os = "macos")))]
fn system() -> Lang {
    Lang::En
}

/// The text in this process's language.
pub fn tr(t: T) -> &'static str {
    tr_in(lang(), t)
}

pub fn tr_in(lang: Lang, t: T) -> &'static str {
    table(t)[lang as usize]
}

/// The text with {0}, {1}, … filled in.
pub fn trf(t: T, args: &[&dyn Display]) -> String {
    fill(tr(t), args)
}

fn fill(s: &str, args: &[&dyn Display]) -> String {
    let mut out = s.to_string();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), &a.to_string());
    }
    out
}

macro_rules! texts {
    ($($name:ident => [$en:expr, $hans:expr, $hant:expr, $ja:expr, $ko:expr],)*) => {
        /// Every text; each platform uses its own share of them.
        #[derive(Clone, Copy, Debug, PartialEq)]
        #[allow(dead_code)]
        pub enum T { $($name),* }

        #[cfg(test)]
        const ALL: &[T] = &[$(T::$name),*];

        /// English, Simplified Chinese, Traditional Chinese, Japanese, Korean
        fn table(t: T) -> [&'static str; 5] {
            match t { $(T::$name => [$en, $hans, $hant, $ja, $ko]),* }
        }
    };
}

texts! {
    // settings, from the Mac's core to its window
    SetRange => ["{0} must be between {1} and {2}", "{0}要在 {1} 到 {2} 之间", "{0}須介於 {1} 到 {2} 之間", "{0}は {1}〜{2} の範囲で指定してください", "{0}은(는) {1}에서 {2} 사이여야 합니다"],
    SetEdge => ["The side must be top, left or right", "方向只能是 top、left、right", "方向只能是 top、left、right", "方向は top、left、right のいずれかです", "방향은 top, left, right 중 하나여야 합니다"],
    MouseSpeed => ["Mouse speed", "鼠标速度", "滑鼠速度", "マウスの速度", "마우스 속도"],
    Position => ["Screen position", "屏幕位置", "螢幕位置", "画面の位置", "화면 위치"],
    ScrollSpeed => ["Scroll speed", "滚动速度", "捲動速度", "スクロールの速度", "스크롤 속도"],
    SetUnknown => ["No such setting: {0}", "没有这个设置：{0}", "沒有這個設定：{0}", "そのような設定はありません：{0}", "그런 설정이 없습니다: {0}"],
    SaveFailed => ["Couldn't save the settings: {0}", "保存设置失败：{0}", "儲存設定失敗：{0}", "設定を保存できませんでした：{0}", "설정을 저장하지 못했습니다: {0}"],
    CodeFailed => ["Couldn't make a pairing code", "无法生成配对码", "無法產生配對碼", "ペアリングコードを作成できませんでした", "페어링 코드를 만들 수 없습니다"],
    UnpairFailed => ["Couldn't unpair: {0}", "解除配对失败：{0}", "解除配對失敗：{0}", "ペアリングを解除できませんでした：{0}", "페어링을 해제하지 못했습니다: {0}"],
    BridgeFailed => ["Developer bridge: {0}", "开发者通道：{0}", "開發者通道：{0}", "開発者チャネル：{0}", "개발자 채널: {0}"],
    AuthFailed => ["Authentication failed", "认证失败", "驗證失敗", "認証に失敗しました", "인증에 실패했습니다"],

    // pairing and finding the Mac
    NoMac => ["Can't find the Mac on the local network", "局域网里没有找到 Mac", "在區域網路中找不到 Mac", "ローカルネットワークで Mac が見つかりません", "로컬 네트워크에서 Mac을 찾을 수 없습니다"],
    NoMacPairing => ["No Mac on the local network is waiting to pair: first click “Pair New Computer” in Wisp on the Mac", "局域网里没有正在等待配对的 Mac：请先在 Mac 的 Wisp 里点“配对新电脑”", "區域網路中沒有正在等待配對的 Mac：請先在 Mac 的 Wisp 裡點「配對新電腦」", "ペアリングを待っている Mac がローカルネットワークにありません。先に Mac の Wisp で「新しいコンピュータとペアリング」をクリックしてください", "페어링을 기다리는 Mac이 로컬 네트워크에 없습니다. 먼저 Mac의 Wisp에서 ‘새 컴퓨터와 페어링’을 누르세요"],
    CodeExpired => ["The code on the Mac has expired; make a new one on the Mac", "Mac 上的配对码已失效，请在 Mac 上重新生成", "Mac 上的配對碼已失效，請在 Mac 上重新產生", "Mac のペアリングコードは無効になりました。Mac で新しいコードを作成してください", "Mac의 페어링 코드가 만료되었습니다. Mac에서 새로 만드세요"],
    PairBadMessage => ["The pairing message was garbled; try again", "配对消息有误，请重试", "配對訊息有誤，請重試", "ペアリングのメッセージが正しくありません。もう一度お試しください", "페어링 메시지가 올바르지 않습니다. 다시 시도하세요"],
    PairNoProof => ["The other side couldn't prove it knows the code; pairing stopped", "对方没能证明自己知道配对码，已停止配对", "對方未能證明自己知道配對碼，已停止配對", "相手がコードを知っていることを証明できなかったため、ペアリングを中止しました", "상대가 코드를 안다는 것을 증명하지 못해 페어링을 중단했습니다"],
    CodeWrongDead => ["Wrong code, and it no longer works; make a new one on the Mac", "配对码不对，已失效，请在 Mac 上重新生成", "配對碼不對，已失效，請在 Mac 上重新產生", "コードが違います。このコードは無効になりました。Mac で新しいコードを作成してください", "코드가 틀렸고 더 이상 쓸 수 없습니다. Mac에서 새로 만드세요"],
    CodeWrongTries => ["Wrong code; {0} tries left", "配对码不对，还可以再试 {0} 次", "配對碼不對，還可以再試 {0} 次", "コードが違います。あと {0} 回試せます", "코드가 틀렸습니다. {0}번 더 시도할 수 있습니다"],
    CodeSix => ["The pairing code is 6 digits", "配对码是 6 位数字", "配對碼是 6 位數字", "ペアリングコードは 6 桁の数字です", "페어링 코드는 6자리 숫자입니다"],
    NotPaired => ["Not paired yet: click “Pair New Computer” in Wisp on the Mac, then enter the code here", "还没有配对：在 Mac 的 Wisp 里点“配对新电脑”，再在这里输入配对码", "尚未配對：在 Mac 的 Wisp 裡點「配對新電腦」，再在這裡輸入配對碼", "まだペアリングしていません。Mac の Wisp で「新しいコンピュータとペアリング」をクリックし、ここにコードを入力してください", "아직 페어링하지 않았습니다. Mac의 Wisp에서 ‘새 컴퓨터와 페어링’을 누른 뒤 여기에 코드를 입력하세요"],
    PairedNeedsAdmin => ["Paired, but the lock screen and UAC prompts need administrator approval to use the new pairing ({0})", "已配对，但锁屏和权限弹窗下要用新配对需要管理员授权（{0}）", "已配對，但要在鎖定畫面和權限提示中使用新配對，需要管理員授權（{0}）", "ペアリングしました。ロック画面と権限の確認画面で新しいペアリングを使うには管理者の許可が必要です（{0}）", "페어링했습니다. 잠금 화면과 권한 확인 창에서 새 페어링을 쓰려면 관리자 승인이 필요합니다({0})"],
    CantResolve => ["Couldn't resolve the server address", "无法解析服务器地址", "無法解析伺服器位址", "サーバーのアドレスを解決できません", "서버 주소를 확인할 수 없습니다"],
    HandshakeDropped => ["The connection closed right after the handshake", "连接在握手后立即断开", "連線在交握後立即中斷", "ハンドシェイク直後に接続が切れました", "핸드셰이크 직후 연결이 끊어졌습니다"],

    // installing and the lock-screen service (Windows)
    StepFailed => ["{0} failed (0x{1})", "{0}失败（0x{1}）", "{0}失敗（0x{1}）", "{0}に失敗しました（0x{1}）", "{0} 실패(0x{1})"],
    CreateShortcut => ["Creating the shortcut", "创建快捷方式", "建立捷徑", "ショートカットの作成", "바로 가기 만들기"],
    SaveShortcut => ["Saving the shortcut", "保存快捷方式", "儲存捷徑", "ショートカットの保存", "바로 가기 저장"],
    ShortcutNote => ["Wisp: connection status and pairing", "Wisp：连接状态与配对", "Wisp：連線狀態與配對", "Wisp：接続状態とペアリング", "Wisp: 연결 상태와 페어링"],
    CopyFailed => ["Copy failed", "复制失败", "複製失敗", "コピーに失敗しました", "복사에 실패했습니다"],
    UpdateNeedsAdmin => ["With “Control the lock screen and UAC prompts” on, updating needs administrator approval ({0})", "开着「锁屏和权限弹窗也能控制」时，更新需要管理员授权（{0}）", "開啟「鎖定畫面和權限提示也能控制」時，更新需要管理員授權（{0}）", "「ロック画面と権限の確認画面も操作」がオンのときは、更新に管理者の許可が必要です（{0}）", "‘잠금 화면과 권한 확인 창도 제어’가 켜져 있으면 업데이트에 관리자 승인이 필요합니다({0})"],
    InstallFailed => ["Wisp couldn't be installed: {0}", "Wisp 安装失败：{0}", "Wisp 安裝失敗：{0}", "Wisp をインストールできませんでした：{0}", "Wisp를 설치하지 못했습니다: {0}"],
    UninstallFailed => ["Wisp wasn't uninstalled: {0}", "Wisp 没有卸载：{0}", "Wisp 未解除安裝：{0}", "Wisp をアンインストールできませんでした：{0}", "Wisp를 제거하지 못했습니다: {0}"],
    Uninstalled => ["Wisp has been uninstalled.", "Wisp 已卸载。", "Wisp 已解除安裝。", "Wisp をアンインストールしました。", "Wisp를 제거했습니다."],
    BadPairInfo => ["The pairing data isn't valid", "配对信息无效", "配對資訊無效", "ペアリング情報が無効です", "페어링 정보가 올바르지 않습니다"],
    NeedAdmin => ["Administrator rights are needed", "需要管理员权限", "需要系統管理員權限", "管理者権限が必要です", "관리자 권한이 필요합니다"],
    NoAdmin => ["Administrator approval wasn't given", "没有得到管理员授权", "未取得管理員授權", "管理者の許可が得られませんでした", "관리자 승인을 받지 못했습니다"],
    SetupUnfinished => ["Setup didn't finish", "设置没有完成", "設定未完成", "設定が完了しませんでした", "설정이 완료되지 않았습니다"],
    NoUser => ["Nobody is signed in", "没有用户登录", "沒有使用者登入", "サインインしているユーザーがいません", "로그인한 사용자가 없습니다"],
    NoUserEnv => ["Couldn't get the user's environment", "无法取得用户的环境", "無法取得使用者的環境", "ユーザーの環境を取得できません", "사용자 환경을 가져올 수 없습니다"],

    // the tray and the status window (Windows)
    LockSwitch => ["Control the lock screen and UAC prompts", "锁屏和权限弹窗也能控制", "鎖定畫面和權限提示也能控制", "ロック画面と権限の確認画面も操作", "잠금 화면과 권한 확인 창도 제어"],
    OpenWindow => ["Open Status Window", "打开状态窗口", "開啟狀態視窗", "状態ウインドウを開く", "상태 창 열기"],
    Quit => ["Quit Wisp", "退出 Wisp", "結束 Wisp", "Wisp を終了", "Wisp 종료"],
    ConnectedFor => ["Connected · {0}", "已连接 · {0}", "已連線 · {0}", "接続中 · {0}", "연결됨 · {0}"],
    Connected => ["Connected", "已连接", "已連線", "接続中", "연결됨"],
    AnotherMac => ["Another Mac", "换一台 Mac", "換一台 Mac", "別の Mac", "다른 Mac"],
    Replaces => ["Once paired, it replaces the current Mac", "配对成功后，会替换现在这台 Mac", "配對成功後，會取代現在這台 Mac", "ペアリングすると、今の Mac と置き換わります", "페어링하면 지금 Mac을 대신합니다"],
    Unpaired => ["Not paired", "还没有配对", "尚未配對", "未ペアリング", "페어링 안 됨"],
    PairThisPc => ["Pair This PC", "配对这台电脑", "配對這台電腦", "この PC をペアリング", "이 PC 페어링"],
    PairHowTo => ["In Wisp on the Mac, click “Pair New Computer”, then type the digits it shows", "在 Mac 的 Wisp 里点“配对新电脑”，再输入它显示的数字", "在 Mac 的 Wisp 裡點「配對新電腦」，再輸入它顯示的數字", "Mac の Wisp で「新しいコンピュータとペアリング」をクリックし、表示された数字を入力します", "Mac의 Wisp에서 ‘새 컴퓨터와 페어링’을 누르고 표시된 숫자를 입력하세요"],
    Searching => ["Searching", "正在寻找", "正在尋找", "検索中", "찾는 중"],
    SearchingMac => ["Looking for the Mac", "正在寻找 Mac", "正在尋找 Mac", "Mac を検索中", "Mac을 찾는 중"],
    Stopped => ["Not running", "没有在运行", "未執行", "停止中", "실행 중 아님"],
    StartWisp => ["Start Wisp", "启动 Wisp", "啟動 Wisp", "Wisp を起動", "Wisp 시작"],
    PairingNow => ["Pairing…", "配对中…", "配對中…", "ペアリング中…", "페어링 중…"],
    Pair => ["Pair", "配对", "配對", "ペアリング", "페어링"],
    Cancel => ["Cancel", "取消", "取消", "キャンセル", "취소"],
    PairAnother => ["Pair with Another Mac", "换一台 Mac 配对", "換一台 Mac 配對", "別の Mac とペアリング", "다른 Mac과 페어링"],
    BackendStopped => ["Wisp's background process isn't running, so the Mac's keyboard and mouse can't reach this PC", "Wisp 的后台没有在运行，Mac 的键鼠不会过来", "Wisp 的背景程式沒有在執行，Mac 的鍵鼠不會過來", "Wisp のバックグラウンドが動いていないため、Mac のキーボードとマウスはこの PC で使えません", "Wisp 백그라운드가 실행 중이 아니어서 Mac의 키보드와 마우스를 이 PC에서 쓸 수 없습니다"],
    WaitingAdmin => ["Waiting for administrator approval…", "等待管理员同意…", "等待管理員同意…", "管理者の許可を待っています…", "관리자 승인을 기다리는 중…"],
    AdminOnce => ["Switching it needs administrator approval once", "开关时需要管理员同意一次", "開關時需要管理員同意一次", "切り替えるときに管理者の許可が 1 回必要です", "켜고 끌 때 관리자 승인이 한 번 필요합니다"],
    ThisPc => ["This PC", "这台电脑", "這台電腦", "この PC", "이 PC"],
    SetOnMac => ["The screen's position is set on the Mac", "屏幕位置在 Mac 上设置", "螢幕位置在 Mac 上設定", "画面の位置は Mac で設定します", "화면 위치는 Mac에서 설정합니다"],
    EnterNewCode => ["Enter the code shown on the new Mac", "输入新 Mac 上显示的配对码", "輸入新 Mac 上顯示的配對碼", "新しい Mac に表示されたコードを入力", "새 Mac에 표시된 코드를 입력하세요"],
    Checking => ["Checking the code with the Mac", "正在和 Mac 核对配对码", "正在和 Mac 核對配對碼", "Mac とコードを照合しています", "Mac과 코드를 확인하는 중"],
    TypeDigits => ["Type the digits, then press Enter", "用键盘输入数字，回车确认", "用鍵盤輸入數字，按 Enter 確認", "数字を入力して Enter キーで確定", "숫자를 입력하고 Enter를 누르세요"],
    PairedConnecting => ["Paired with {0}; connecting…", "已与 {0} 配对，正在连接…", "已與 {0} 配對，正在連線…", "{0} とペアリングしました。接続しています…", "{0}와(과) 페어링했습니다. 연결하는 중…"],
    BridgeOn => ["Developer bridge is on", "开发者通道已开启", "開發者通道已開啟", "開発者チャネルがオンです", "개발자 채널이 켜져 있습니다"],
    UnderMinute => ["under a minute", "不到 1 分钟", "不到 1 分鐘", "1 分未満", "1분 미만"],
    Minutes => ["{0} min", "{0} 分钟", "{0} 分鐘", "{0} 分", "{0}분"],
    HoursMinutes => ["{0} h {1} min", "{0} 小时 {1} 分钟", "{0} 小時 {1} 分鐘", "{0} 時間 {1} 分", "{0}시간 {1}분"],
    WispStopped => ["Wisp isn't running", "Wisp 没有在运行", "Wisp 未執行", "Wisp は動作していません", "Wisp가 실행 중이 아닙니다"],
    StartsAtSignIn => ["It starts on its own when you sign in to Windows", "登录 Windows 时它会自动启动", "登入 Windows 時它會自動啟動", "Windows にサインインすると自動で起動します", "Windows에 로그인하면 자동으로 시작됩니다"],
    ConnectedTo => ["Connected to {0}", "已连接 {0}", "已連線 {0}", "{0} に接続中", "{0}에 연결됨"],
    CursorComes => ["The Mac's cursor comes here when it crosses the screen edge", "Mac 的光标越过屏幕边缘，就会来到这里", "Mac 的游標越過螢幕邊緣，就會來到這裡", "Mac のカーソルが画面の端を越えるとここに来ます", "Mac의 커서가 화면 가장자리를 넘으면 여기로 옵니다"],
    NotPairedYet => ["Not paired yet", "还没有配对", "尚未配對", "まだペアリングしていません", "아직 페어링하지 않았습니다"],
    ClickPairOnMac => ["In Wisp on the Mac, click “Pair New Computer”", "在 Mac 的 Wisp 里点“配对新电脑”", "在 Mac 的 Wisp 裡點「配對新電腦」", "Mac の Wisp で「新しいコンピュータとペアリング」をクリック", "Mac의 Wisp에서 ‘새 컴퓨터와 페어링’을 누르세요"],
    EnterMacCode => ["Enter the code shown on the Mac", "输入 Mac 上显示的配对码", "輸入 Mac 上顯示的配對碼", "Mac に表示されたコードを入力", "Mac에 표시된 코드를 입력하세요"],
    SearchingLan => ["Looking for the Mac on the local network", "正在局域网里寻找 Mac", "正在區域網路中尋找 Mac", "ローカルネットワークで Mac を探しています", "로컬 네트워크에서 Mac을 찾는 중"],
    SearchingNow => ["Looking for the Mac…", "正在寻找 Mac…", "正在尋找 Mac…", "Mac を検索中…", "Mac을 찾는 중…"],
    RepairPrompt => ["Pair again? Enter the code shown on the Mac", "要重新配对？输入 Mac 上的配对码", "要重新配對？輸入 Mac 上的配對碼", "ペアリングし直しますか？Mac に表示されたコードを入力", "다시 페어링할까요? Mac에 표시된 코드를 입력하세요"],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_language_picks_one_of_five_and_english_otherwise() {
        assert_eq!(pick("zh-Hans-CN"), Lang::ZhHans);
        assert_eq!(pick("zh-Hant-TW"), Lang::ZhHant);
        assert_eq!(pick("zh-HK"), Lang::ZhHant);
        assert_eq!(pick("zh_CN"), Lang::ZhHans);
        assert_eq!(pick("ja-JP"), Lang::Ja);
        assert_eq!(pick("ko-KR"), Lang::Ko);
        assert_eq!(pick("en-GB"), Lang::En);
        assert_eq!(pick("fr-FR"), Lang::En, "not one of the five: English");
        assert_eq!(pick_langid(0x0804), Lang::ZhHans);
        assert_eq!(pick_langid(0x0404), Lang::ZhHant);
        assert_eq!(pick_langid(0x0C04), Lang::ZhHant);
        assert_eq!(pick_langid(0x0411), Lang::Ja);
        assert_eq!(pick_langid(0x0412), Lang::Ko);
        assert_eq!(pick_langid(0x0409), Lang::En);
        assert_eq!(pick_langid(0x0407), Lang::En);
    }

    #[test]
    fn every_text_has_all_five_languages_with_the_same_blanks() {
        let blanks = |s: &str| (0..4).filter(|i| s.contains(&format!("{{{i}}}"))).collect::<Vec<_>>();
        for &t in ALL {
            let all = table(t);
            for s in all {
                assert!(!s.trim().is_empty(), "{t:?} has an empty translation");
                assert_eq!(blanks(s), blanks(all[0]), "{t:?}: {s}");
            }
        }
        assert_eq!(fill(tr_in(Lang::En, T::CodeWrongTries), &[&2]), "Wrong code; 2 tries left");
        assert_eq!(fill(tr_in(Lang::Ja, T::HoursMinutes), &[&3, &5]), "3 時間 5 分");
    }
}
