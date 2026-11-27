//! hint 的 Tauri 壳：窗口、托盘、全局热键、命令注册。
//!
//! 业务链路在 [`session`]（采集 → STT → 会话存储 → LLM 流式）与
//! [`commands`]（前端 invoke 入口）里。

mod commands;
mod doctor;
mod import;
mod logging;
mod session;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use hint_core::session::LifecycleQueue;
use hint_core::settings::Settings;
use hint_store::Store;
use hint_stt::ModelManager;
use tauri::{
    Manager, RunEvent, WindowEvent,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// 忽略锁中毒：任何线程 panic 都不该让命令层跟着挂。
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 运行中的会话：命令层只读取是否存在，结束时由 `session` 模块整体取走。
pub(crate) struct RunningSession {
    pub(crate) shared: Arc<session::SessionShared>,
    pub(crate) handle: hint_audio::AudioHandle,
    pub(crate) jobs: Option<std::sync::mpsc::SyncSender<session::SttJob>>,
    pub(crate) dispatcher: Option<std::thread::JoinHandle<()>>,
}

pub(crate) struct AppState {
    pub(crate) settings_path: PathBuf,
    /// 日志目录，`open_log_dir` 命令用。
    pub(crate) log_dir: PathBuf,
    /// 首次关闭主窗口的托盘提示只发一次。
    pub(crate) close_tip_shown: AtomicBool,
    /// 启动时会话数据库是否打不开、降级到了内存库。
    ///
    /// 与 `settings_corrupted` 同一类问题：`Store::open` 失败时那条
    /// `tracing::error!` 发生在 `logging::init` 之前，写不出去；用户看到的是
    /// 「会话复盘里一条记录都没有」——历史面试记录已经在磁盘上坏掉了，而他不知道。
    /// 内存库只在本次运行有效，重启后又是一个空库。
    pub(crate) store_degraded: bool,
    /// 启动时配置文件是否损坏过（损坏的文件已被备份成 `settings.json.bad`）。
    ///
    /// 为什么记在这里：`Settings::load` 里的 `tracing::warn!` 发生在
    /// `logging::init` 之前（配置在 `AppState::new` 阶段就要读，而日志在
    /// `.setup()` 里才初始化），那条警告**写不出去**。真机实测全日志里搜不到
    /// 「配置文件损坏」，而用户的简历、项目资料、接口配置已经全部回到默认值——
    /// 只会发现"我填的东西没了"，不知道被备份了、也不知道 `.bad` 在哪。
    pub(crate) settings_corrupted: bool,
    /// 浮层几何防抖的「代数」：拖动时每来一个事件 +1，睡醒后代数变了就跳过。
    ///
    /// 用 `Arc<AtomicU64>` 是因为要移进保存线程里比对；`AppState` 本身不能
    /// 被线程捕获（它由 Tauri 托管）。
    pub(crate) overlay_bounds_gen: Arc<AtomicU64>,
    /// 会话数据库；`hint_store::Store` 不是 `Sync`，统一用 `Mutex` 保护。
    pub(crate) store: Mutex<Store>,
    /// 语音模型管理器（内部状态全 `Arc` 共享，可克隆）。
    pub(crate) model: ModelManager,
    /// 正在运行的会话。
    pub(crate) session: Mutex<Option<RunningSession>>,
    /// 模型下载的取消标志；`None` 表示没有进行中的下载。
    pub(crate) prepare_cancel: Mutex<Option<Arc<AtomicBool>>>,
    /// 最近一次会话的分发线程完成信号；会话结束后仍保留，供 `remove_model` 等待。
    pub(crate) dispatch_done: Mutex<Option<Arc<DispatchDone>>>,
    /// 会话生命周期队列：串行化 start/stop，消除 check-exec 竞态。
    pub(crate) lifecycle: Mutex<LifecycleQueue>,
    /// 进行中的截图问答：`None` 表示当前没有在跑。
    ///
    /// 与 `session` 完全分开：截图问答不参与面试会话的生命周期，也不被
    /// `stop_session` 取消；两者只共享「同一份模型配置」这个前提。
    pub(crate) capture: Mutex<Option<Arc<crate::commands::CaptureRun>>>,
}

/// 分发线程完成信号：`(Mutex<bool>, Condvar)` 是「完成标志 + 唤醒」对，
/// 置位 true 后 Condvar 唤醒所有等待方。类型别名压平 `AppState` 字段的
/// clippy type_complexity 嵌套计数，语义不变。
pub(crate) type DispatchDone = (Mutex<bool>, Condvar);

/// 托盘里的「开始/结束面试」菜单项，随会话状态启用/禁用。
pub(crate) struct TrayItems {
    pub(crate) session_start: MenuItem<tauri::Wry>,
    pub(crate) session_stop: MenuItem<tauri::Wry>,
    /// 托盘图标本身：会话状态变化时更新 tooltip，用户悬停即可知道是否在面试。
    pub(crate) tray: tauri::tray::TrayIcon<tauri::Wry>,
}

/// 全局热键的 4 组组合（与前端约定一致）。
const SHORTCUTS: [(&str, Code); 4] = [
    ("ctrl+shift+h", Code::KeyH),
    ("ctrl+shift+a", Code::KeyA),
    ("ctrl+shift+r", Code::KeyR),
    ("ctrl+shift+s", Code::KeyS),
];

/// 托盘 tooltip：主窗口关掉后，这是用户唯一能确认「程序在干什么」的地方。
const TRAY_TOOLTIP_IDLE: &str = "hint · 空闲（右键可退出；Ctrl+Shift+H 显示浮层）";
const TRAY_TOOLTIP_LIVE: &str = "hint · 面试进行中（右键可结束或退出）";

fn combo(code: Code) -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), code)
}

/// 浮层默认宽度占可用宽度（工作区，不含任务栏）的比例。
const OVERLAY_WIDTH_RATIO: u32 = 3;
/// 浮层高度占可用高度的比例。
const OVERLAY_HEIGHT_RATIO: u32 = 3;
/// 浮层默认宽度的逻辑像素下限：再窄就放不下答案与两路电平。
/// 浮层窗口的最小尺寸（逻辑像素），与 `tauri.conf.json` 的 `minWidth` / `minHeight`
/// 保持一致。
///
/// 为什么代码里要再写一遍：恢复几何用的是 Win32 `SetWindowPos`（物理像素），
/// 它**绕过**了 tauri 建窗时设的最小尺寸约束。真机实测：配置里写 320×180 物理、
/// dpr 1.25 时窗口变成 256×144 CSS——比设计最小值还小，各区块被压到勉强可用。
///
/// 改 tauri.conf 时这里必须同步改（两处都是「窗口能小到什么程度」的唯一真相）。
const OVERLAY_WINDOW_MIN_WIDTH_LOGICAL: f64 = 320.0;
const OVERLAY_WINDOW_MIN_HEIGHT_LOGICAL: f64 = 180.0;

/// 浮层默认几何里宽度的下限（比窗口最小宽度更宽：默认值要好看，最小尺寸只保证可用）。
const OVERLAY_MIN_WIDTH_LOGICAL: u32 = 420;
/// 浮层默认宽度的逻辑像素上限：超宽屏上 1/3 仍有 1100+ 逻辑像素，
/// 一行答案横跨半个屏反而难扫读，这里压回 900。
const OVERLAY_MAX_WIDTH_LOGICAL: u32 = 900;

/// 浮层默认几何（物理像素）：宽高各取可用区域的 1/3，水平居中、垂直贴顶。
///
/// 纯函数，只吃「显示器工作区 + 缩放比」，不读任何全局状态也不写死分辨率：
/// - **工作区而不是整屏**：任务栏在顶部/侧边的机器上，整屏坐标会把浮层压到
///   任务栏下面；`work_area` 已经是扣掉任务栏后的可用区域。
/// - **宽度同时受下限约束**：4K 屏 1/3 宽 ≈ 1280px，答案行长会难读；
///   1080p 屏 1/3 宽 ≈ 640px；超宽屏（如 3440）1/3 仍有 1146px，
///   所以再按 `--overlay` 的实际用途给一个 900px 逻辑上限，避免一行字横跨半个屏。
/// - **最小宽度按逻辑像素给**：高 DPI 下 420 逻辑像素要乘缩放比，
///   否则 150% 缩放的机器上浮层会窄得放不下「重新生成 / 清空」两个按钮。
/// - **垂直贴顶**：面试时视线在屏幕上方（摄像头附近），浮层贴顶最不容易被注意到；
///   这里只留 0 边距，位置直接取工作区上沿。
fn overlay_default_geometry(
    work_area: (i32, i32, u32, u32),
    scale_factor: f64,
) -> (tauri::PhysicalSize<u32>, tauri::PhysicalPosition<i32>) {
    let (origin_x, origin_y, available_width, available_height) = work_area;
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let min_width = (f64::from(OVERLAY_MIN_WIDTH_LOGICAL) * scale).round() as u32;
    let max_width = (f64::from(OVERLAY_MAX_WIDTH_LOGICAL) * scale).round() as u32;
    // 宽度：可用宽度 / 3 → 不超过上限 → 不小于下限 → 不超过可用宽度。
    // 先钳上限再抬下限，保证「下限 > 上限」的怪异缩放比下仍以最小宽度为准；
    // 最后再与可用宽度取小，极小屏（如 800×600 @100%）不会比屏幕还宽。
    let width = (available_width / OVERLAY_WIDTH_RATIO)
        .min(max_width)
        .max(min_width)
        .min(available_width)
        .max(1);
    let height = (available_height / OVERLAY_HEIGHT_RATIO).max(1);
    // 水平居中：左右各留相同空白；余数为奇数时左边多 1px 也无明显偏差。
    let x = origin_x + ((available_width.saturating_sub(width)) / 2) as i32;
    let y = origin_y;
    (
        tauri::PhysicalSize::new(width, height),
        tauri::PhysicalPosition::new(x, y),
    )
}

/// 把浮层摆到主显示器的默认位置与尺寸（可用区域的 1/3、水平居中、贴顶）。
///
/// **只在没有用户记录时调用**：用户拖动/改过大小之后走
/// [`apply_saved_overlay_geometry`]；这里只在首次启动、记录不可用、
/// 或记录已不在屏幕内时兜底。
///
/// 用 Win32 `SetWindowPos` 摆（物理像素），与恢复路径同一套语义——
/// tauri 的 `set_size` 在这台机器上会再乘一次 dpr，两侧不一致就永远对不齐。
///
/// 失败只记日志：浮层摆歪不该让应用起不来。
fn apply_overlay_default_geometry(window: &tauri::WebviewWindow) {
    let monitor = match window.primary_monitor() {
        Ok(Some(monitor)) => monitor,
        Ok(None) => {
            tracing::warn!("取不到主显示器，浮层沿用配置里的默认尺寸");
            return;
        }
        Err(error) => {
            tracing::warn!(%error, "读取主显示器失败，浮层沿用配置里的默认尺寸");
            return;
        }
    };
    let area = monitor.work_area();
    let (size, position) = overlay_default_geometry(
        (
            area.position.x,
            area.position.y,
            area.size.width,
            area.size.height,
        ),
        monitor.scale_factor(),
    );
    let bounds = hint_core::OverlayBounds {
        x: f64::from(position.x),
        y: f64::from(position.y),
        width: f64::from(size.width),
        height: f64::from(size.height),
    };
    if !set_overlay_physical_rect(window, bounds) {
        tracing::warn!(
            width = size.width,
            height = size.height,
            "设置浮层默认几何失败"
        );
    }
    tracing::info!(
        width = size.width,
        height = size.height,
        x = position.x,
        y = position.y,
        scale = monitor.scale_factor(),
        "浮层默认尺寸与位置已应用（可用区域的 1/3，水平居中、贴顶）"
    );
}

/// 按用户上次调整的位置/大小恢复浮层。
///
/// 返回 `true` 表示已恢复；`false` 表示没有可用记录，调用方应回退到默认几何。
///
/// **单位**：配置里存的是**物理像素**（与 Win32 `GetWindowRect` 一致），恢复时
/// 直接用 `SetWindowPos` 摆，不经过 tauri 的尺寸换算。
/// 交给 tauri 换算。
///
/// 校验分两层：
/// 1. `is_sane()` 挡掉 NaN / 非正尺寸；
/// 2. **与当前显示器工作区求交**——换过显示器、拔掉副屏、改过分辨率之后，
///    上次的坐标可能整个落在屏幕外，那时用户既看不见浮层也拖不回来。
///    交集不足 1/3 就当作没有记录，回到默认几何。

fn apply_saved_overlay_geometry(
    window: &tauri::WebviewWindow,
    bounds: hint_core::OverlayBounds,
) -> bool {
    if !bounds.is_sane() {
        tracing::info!("配置里的浮层几何不可用，改用默认几何");
        return false;
    }
    let Ok(Some(monitor)) = window.primary_monitor() else {
        tracing::info!("取不到主显示器，跳过几何恢复");
        return false;
    };
    let area = monitor.work_area();
    let scale = monitor.scale_factor();

    // 逻辑最小尺寸：与 tauri.conf.json 的 minWidth/minHeight 对齐（320×180 逻辑像素）。
    //
    // 为什么需要单独判：恢复用的是 Win32 `SetWindowPos`（物理像素），它**绕过**了
    // tauri 建窗时设的最小尺寸约束。真机实测：配置里写 320×180 物理、dpr 1.25 时
    // 窗口变成 256×144 CSS——比设计的最小值还小，各区块被压到勉强可用。
    let min_w = OVERLAY_WINDOW_MIN_WIDTH_LOGICAL * scale;
    let min_h = OVERLAY_WINDOW_MIN_HEIGHT_LOGICAL * scale;
    let width_phys = bounds.width.max(min_w);
    let height_phys = bounds.height.max(min_h);

    // 求交：`bounds` 已经是**物理**像素（保存侧用的 Win32 GetWindowRect），
    // 与工作区同一坐标系，**不要再乘 scale**。
    //
    // 这里曾经乘过一次——那是几何还按「逻辑像素」存储时的写法；改成 Win32 物理像素
    // 之后没跟着改，会把尺寸放大 1/dpr 倍参与求交，在多显示器或贴边位置误判成
    // 「不在屏幕内」而白白丢弃用户的调整。
    let x_phys = bounds.x;
    let y_phys = bounds.y;
    if !overlay_geometry_fits(
        x_phys,
        y_phys,
        width_phys,
        height_phys,
        area.position.x,
        area.position.y,
        area.size.width,
        area.size.height,
    ) {
        tracing::info!(
            x = bounds.x,
            y = bounds.y,
            width = bounds.width,
            height = bounds.height,
            "上次的浮层位置几乎不在屏幕内，改用默认几何"
        );
        return false;
    }

    // 用 Win32 摆（物理像素）：与保存侧同一套语义，不经过 tauri 的换算。
    // 尺寸先抬到逻辑最小值（见上），位置保持用户原样。
    let target = hint_core::OverlayBounds {
        x: bounds.x,
        y: bounds.y,
        width: width_phys,
        height: height_phys,
    };
    if !set_overlay_physical_rect(window, target) {
        tracing::warn!("恢复浮层位置与大小失败");
        return false;
    }
    tracing::info!(
        x = bounds.x,
        y = bounds.y,
        width = bounds.width,
        height = bounds.height,
        "已恢复上次的浮层位置与大小（物理像素）"
    );
    true
}

/// 浮层几何是否「足够在屏幕内」，值得沿用用户上次的位置。
///
/// ## 判据
///
/// 计算浮层矩形与显示器工作区的**交集面积**，若交集不足浮层总面积的 1/3，
/// 就认为位置几乎在屏幕外——换过显示器、拔掉副屏、改过分辨率之后，上次的坐标
/// 可能整个落在屏外，那时用户既看不见浮层也拖不回来。
///
/// ## 为什么抽成纯函数
///
/// 判据是纯数值计算（矩形求交 + 比例比较），但此前内联在 `apply_saved_overlay_geometry`
/// 里，**改错阈值（1/3 改成 1/2）、或把 `<` 写成 `<=`）都不会让任何测试变红**。
/// 而判据改错的后果是：要么浮层被白白丢回默认位置（用户调的尺寸不保留），
/// 要么屏外的浮层被保留（用户看不见）。
///
/// 输入是物理像素，与工作区同一坐标系。
// 参数多但都是必须的（位置、尺寸、工作区四元），打包成 struct 反而让调用点更啰嗦。
#[allow(clippy::too_many_arguments)]
fn overlay_geometry_fits(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    area_x: i32,
    area_y: i32,
    area_w: u32,
    area_h: u32,
) -> bool {
    let total = width * height;
    if total <= 0.0 {
        return false;
    }
    let left = f64::from(area_x);
    let top = f64::from(area_y);
    let right = left + f64::from(area_w);
    let bottom = top + f64::from(area_h);
    let overlap_w = (x + width).min(right) - x.max(left);
    let overlap_h = (y + height).min(bottom) - y.max(top);
    let visible = overlap_w.max(0.0) * overlap_h.max(0.0);
    visible * 3.0 >= total
}

/// 读窗口的位置与大小（**物理像素**，Win32 `GetWindowRect`）。
///
/// 为什么不用 tauri 的 `inner_size()` / `outer_position()`：真机实测这台机器上
/// 它们返回的是**物理值再乘一次 dpr**（Win32 物理宽 750、dpr 1.25 时返回 938），
/// 而 `set_size` 又把它除回去。读和写都带同一个偏差，看起来"能用"，但存进配置
/// 的值跟真实几何对不上，换台 DPI 不同的机器就摆歪。
/// 直接用 Win32 拿物理像素，语义唯一。
#[cfg(windows)]
fn overlay_physical_rect(window: &tauri::WebviewWindow) -> Option<hint_core::OverlayBounds> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let raw = window.hwnd().ok()?;
    let handle = HWND(raw.0);
    let mut rect = RECT::default();
    // 失败返回非零 WIN32_ERROR；这里只关心成败。
    if unsafe { GetWindowRect(handle, &mut rect) }.is_err() {
        return None;
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some(hint_core::OverlayBounds {
        x: f64::from(rect.left),
        y: f64::from(rect.top),
        width: f64::from(width),
        height: f64::from(height),
    })
}

#[cfg(not(windows))]
fn overlay_physical_rect(_window: &tauri::WebviewWindow) -> Option<hint_core::OverlayBounds> {
    None
}

/// 用 Win32 `SetWindowPos` 把窗口摆到指定**物理**位置与大小。
#[cfg(windows)]
fn set_overlay_physical_rect(
    window: &tauri::WebviewWindow,
    bounds: hint_core::OverlayBounds,
) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos};

    let Ok(raw) = window.hwnd() else {
        return false;
    };
    let handle = HWND(raw.0);
    let x = bounds.x.round() as i32;
    let y = bounds.y.round() as i32;
    let width = bounds.width.round() as i32;
    let height = bounds.height.round() as i32;
    unsafe {
        SetWindowPos(
            handle,
            None,
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
    .is_ok()
}

#[cfg(not(windows))]
fn set_overlay_physical_rect(
    _window: &tauri::WebviewWindow,
    _bounds: hint_core::OverlayBounds,
) -> bool {
    false
}

/// 浮层几何写盘前的静默窗口：拖动结束后等这么久再写。
const OVERLAY_BOUNDS_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(800);

/// 防抖保存浮层几何。
///
/// 实现：每次事件把 `pending` 计数 +1，然后起一个线程睡 [`OVERLAY_BOUNDS_DEBOUNCE`]；
/// 醒来后如果计数没再变（说明用户停手了），才真正读窗口几何并写盘。计数变了就说明
/// 还在拖，什么都不做——最后由最新那次线程负责保存。
fn schedule_overlay_bounds_save(app: tauri::AppHandle, state: &AppState) {
    let generation = state.overlay_bounds_gen.fetch_add(1, Ordering::AcqRel) + 1;
    let generation_ref = state.overlay_bounds_gen.clone();
    std::thread::spawn(move || {
        std::thread::sleep(OVERLAY_BOUNDS_DEBOUNCE);
        // 期间又有新事件 → 交给更新的那次线程处理。
        if generation_ref.load(Ordering::Acquire) != generation {
            return;
        }
        let Some(window) = app.get_webview_window("overlay") else {
            return;
        };
        // 用 Win32 读物理像素（见 overlay_physical_rect 的说明：
        // tauri 的 inner_size()/outer_position() 在这台机器上带 dpr 偏差）。
        let Some(bounds) = overlay_physical_rect(&window) else {
            return;
        };
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let mut settings = Settings::load(&state.settings_path);
        // 与上次相同就不写盘（避免无意义的 IO 与文件时间戳变化）。
        if settings.overlay_bounds == Some(bounds) {
            return;
        }
        settings.overlay_bounds = Some(bounds);
        match settings.save(&state.settings_path) {
            Ok(()) => tracing::debug!(
                x = bounds.x,
                y = bounds.y,
                w = bounds.width,
                h = bounds.height,
                "已记住浮层位置与大小"
            ),
            Err(error) => tracing::warn!(%error, "保存浮层几何失败"),
        }
    });
}

impl AppState {
    fn new(data_dir: &std::path::Path) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir).map_err(|error| format!("创建数据目录失败：{error}"))?;
        let db_path = data_dir.join("hint.db");
        // 打不开就退回内存库，保证应用还能启动；同时记下「降级过」，
        // 由壳层在日志就绪后告诉用户（此时日志还没初始化，这里打什么都写不出去）。
        let (store, store_degraded) = match Store::open(&db_path) {
            Ok(store) => (store, false),
            Err(error) => {
                tracing::error!(%error, path = %db_path.display(), "打开会话数据库失败，改用内存库");
                let fallback = Store::open_in_memory().map_err(|fallback| {
                    format!("会话数据库不可用：{error}；内存库也失败：{fallback}")
                })?;
                (fallback, true)
            }
        };
        // 保留最近 30 天的会话记录；清理失败不影响启动。
        match store.cleanup_older_than_days(30) {
            Ok(removed) if removed > 0 => {
                tracing::info!(removed, "已清理 30 天前的会话记录");
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, "清理旧会话记录失败");
            }
        }
        // 这里就加载一次配置，把「是否损坏」记下来：日志系统此时还没初始化，
        // 损坏警告只能延后到 setup 里发（见 `settings_corrupted` 的说明）。
        let settings_path = data_dir.join("settings.json");
        let (_, settings_corrupted) = Settings::load_reporting(&settings_path);
        Ok(Self {
            settings_path,
            log_dir: data_dir.join("logs"),
            settings_corrupted,
            store_degraded,
            close_tip_shown: AtomicBool::new(false),
            overlay_bounds_gen: Arc::new(AtomicU64::new(0)),
            store: Mutex::new(store),
            model: ModelManager::new(model_root(data_dir)),
            session: Mutex::new(None),
            prepare_cancel: Mutex::new(None),
            dispatch_done: Mutex::new(None),
            lifecycle: Mutex::new(LifecycleQueue::default()),
            capture: Mutex::new(None),
        })
    }
}

/// 语音模型的落盘根目录。
///
/// 历史上放在 `<app_data_dir>/voice`：但 Tauri NSIS 卸载器在用户勾选
/// 「删除应用数据」时会 `RmDir /r $APPDATA/<bundle-id>` 与
/// `$LOCALAPPDATA/<bundle-id>`，241 MB 的模型会连同会话库一起被删，
/// 重装后被迫重新下载。模型改放在**不含 bundle id** 的
/// `%LOCALAPPDATA%/hint-models`，卸载器不会触碰它。
///
/// 兼容：启动时若发现旧 `<app_data_dir>/voice/sensevoice` 且新位置不存在，
/// 整体搬过去（纯 `rename`，失败则继续用旧目录，不阻塞启动）。
fn model_root(data_dir: &std::path::Path) -> std::path::PathBuf {
    let legacy = data_dir.join("voice");
    let Some(new_root) = model_root_path(data_dir) else {
        tracing::warn!("无法定位 LOCALAPPDATA，语音模型继续存放在应用数据目录");
        return legacy;
    };
    if let Ok(Some(migrated)) = migrate_legacy_voice_dir(&legacy, &new_root) {
        tracing::info!(from = %legacy.display(), to = %migrated.display(), "已迁移旧语音模型目录");
    }
    new_root
}

/// 纯函数版模型根：只算路径，不迁移、不落盘。
///
/// 主程序用 [`model_root`]（带迁移），`--doctor` 用这个——诊断是只读的，
/// 不能在收集信息时顺手搬 241 MB。返回 `None` 表示环境变量都拿不到，
/// 调用方应回退到 `<data_dir>/voice`。
pub(crate) fn model_root_path(_data_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    // 模型目录也可以被环境变量覆盖。与 `HINT_DATA_DIR` 分开是因为两者的
    // 诉求不同：数据目录要隔离（配置/数据库会被破坏性测试改写），而模型
    // 241MB 通常**不该**复制一份——默认共享真实模型，只在要测「模型损坏」
    // 时才用 `HINT_MODEL_DIR` 指到测试副本。
    if let Some(override_dir) = std::env::var_os("HINT_MODEL_DIR")
        && !override_dir.is_empty()
    {
        return Some(std::path::PathBuf::from(override_dir));
    }
    // Windows 下 %LOCALAPPDATA% 即 Tauri app_local_data_dir 的来源；
    // 拿不到时回退 %APPDATA% 的父目录（Roaming 的上级即 Local）。
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("APPDATA")
                .map(std::path::PathBuf::from)
                .and_then(|roaming| roaming.parent().map(|parent| parent.to_path_buf()))
        })
        .map(|base| base.join("hint-models"))
}

/// 旧 `voice` 下的模型搬进新根；返回 `Ok(Some(新模型目录))` 表示发生了搬移。
///
/// `ModelManager` 的 `Layout` 在根目录下拼 `sensevoice/`，所以这里必须把
/// 旧的 `voice/sensevoice` 搬到 `new_root/sensevoice`——搬到 `new_root/voice`
/// 会让 manager 找不到模型、又触发一次下载（正是要修的问题）。
fn migrate_legacy_voice_dir(
    legacy: &std::path::Path,
    new_root: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, String> {
    let legacy_model_dir = legacy.join("sensevoice");
    if !legacy_model_dir.is_dir() {
        return Ok(None);
    }
    let target = new_root.join("sensevoice");
    if target.exists() {
        // 新位置已有模型：保留新位置，不覆盖。
        return Ok(None);
    }
    if let Err(error) = std::fs::create_dir_all(new_root) {
        tracing::warn!(%error, to = %new_root.display(), "创建模型根目录失败，继续使用旧位置");
        return Ok(None);
    }
    // 同卷 rename 原子且不重下 241MB。
    match std::fs::rename(&legacy_model_dir, &target) {
        Ok(()) => Ok(Some(target)),
        Err(error) => {
            tracing::warn!(
                %error,
                from = %legacy_model_dir.display(),
                to = %target.display(),
                "迁移语音模型目录失败（可能跨卷），继续使用旧位置"
            );
            Ok(None)
        }
    }
}

/// 热键（Ctrl+Shift+H）与托盘「浮层」菜单共用的开关。
///
/// ## 为什么这里也要持久化
///
/// 命令层有一份同名的 `commands::toggle_overlay`，它调了 `persist_overlay_visible`
/// 把「用户想让浮层显示还是隐藏」写进配置。这条路径原本**没有**——于是同一个开关，
/// 从设置页点会记住、按热键或走托盘却不记：用户按 Ctrl+Shift+H 收起浮层，下次启动
/// 它又自己弹出来盖住屏幕。
///
/// 配置里的 `overlay_visible` 是「用户意图」的单一真相（`overlay_is_visible` 命令
/// 的注释里写了这一点），两条入口必须都更新它。
fn toggle_overlay(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("overlay") else {
        return;
    };
    let now_visible = !window.is_visible().unwrap_or(false);
    if now_visible {
        let _ = window.show();
        // 浮层配置为 focus:false；可聚焦才能收到 Esc / 方向键。
        let _ = window.set_focusable(true);
        let _ = window.set_focus();
    } else {
        let _ = window.set_focusable(false);
        let _ = window.hide();
    }
    commands::persist_overlay_visible(app, now_visible);
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// 托盘菜单的「开始/结束面试」开关。
pub(crate) fn set_session_running(app: &tauri::AppHandle, running: bool) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.session_start.set_enabled(!running);
        let _ = items.session_stop.set_enabled(running);
        // 用户关掉主窗口后只剩托盘可见，tooltip 必须能反映会话是否在跑。
        let _ = items.tray.set_tooltip(Some(if running {
            TRAY_TOOLTIP_LIVE
        } else {
            TRAY_TOOLTIP_IDLE
        }));
    }
}

/// `--doctor` 的诊断入口：不启动窗口、不改配置，只收集环境事实。
///
/// 需要在**任何 Tauri 初始化之前**调用——诊断要能在「界面打不开」
/// 的场景下跑通，所以不能依赖 AppState。
pub fn doctor_entry() -> i32 {
    // 走与主程序**同一个**推导函数：这样设了 HINT_DATA_DIR 时，`--doctor`
    // 诊断的也是那个隔离目录，而不是真实数据目录——否则「用隔离目录复现问题、
    // 再用 --doctor 看诊断」这条最常用的排障路径会对不上号。
    let app_data_dir = fallback_app_data_dir();
    doctor::run(&app_data_dir)
}

/// 数据目录的环境变量覆盖名。
///
/// ## 为什么需要
///
/// 验证「数据库升级」「配置损坏」「旧库兼容」这类行为时，必须在**真实数据上做
/// 破坏性操作**——否则测不出东西。此前的做法是「备份 → 破坏 → 还原」，但备份
/// 有真实的失败模式：SQLite 的 WAL 在进程被强杀后可能还没 checkpoint 进主库，
/// 此时复制出来的 `.db` 是不完整的。（真机踩过：备份出来的库 sessions=0，
/// 还原后用户数据一度归零，靠另一个更早的备份救回来。）
///
/// 设了这个变量之后，应用读写的是**另一个目录**，真实数据全程不被触碰——
/// 破坏性测试从「有风险」变成「不可能影响用户」。
///
/// 用法：
///
/// ```text
/// set HINT_DATA_DIR=D:\tmp\hint-test-data
/// hint.exe
/// ```
///
/// 测试脚本应在启动前确认目录里没有真实数据。
pub(crate) const DATA_DIR_ENV: &str = "HINT_DATA_DIR";

/// 按 Tauri 的规则推导应用数据目录：`%APPDATA%\<identifier>`。
///
/// 与 `app.path().app_data_dir()` 等价（Windows），但不需要 `AppHandle`——
/// 应用状态要在 Builder 阶段注册，那时还没有 AppHandle。
/// setup 里会与官方结果比对，防止规则变化导致静默读错文件。
fn fallback_app_data_dir() -> std::path::PathBuf {
    // 环境变量优先：测试与排障用隔离目录，真实数据不受影响。
    if let Some(override_dir) = std::env::var_os(DATA_DIR_ENV)
        && !override_dir.is_empty()
    {
        return std::path::PathBuf::from(override_dir);
    }
    let identifier = "com.ojbkxc.hint";
    std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(identifier)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 应用状态必须在**窗口创建之前**注册。
    //
    // 原先 `app.manage(state)` 写在 `setup` 里，而 setup 在窗口与 webview 建好
    // 之后才跑：前端的首屏 effect 已经开始发 IPC 了，部分命令会撞上
    // 「state not managed for field `state`」直接失败（真机实测：设置页显示
    // 「读取模型状态失败：state not managed…」，表单停在空值，用户一保存就把
    // 空配置写回文件）。改成 Builder 阶段注册，任何窗口 IPC 都不可能更早。
    //
    // 代价是拿不到 AppHandle 推导 app_data_dir，这里按 Tauri 的规则
    // （`%APPDATA%\<identifier>`）自行推导；setup 里再与官方路径比对一次，
    // 不一致就报错，避免硬编码悄悄漂移。
    let data_dir = fallback_app_data_dir();
    let state = match AppState::new(&data_dir) {
        Ok(state) => state,
        Err(error) => {
            // 此时还没有日志系统，只能走 stderr；release 下用户看不到，
            // 但应用会因缺状态而无法工作，比静默半死好。
            eprintln!("hint: 初始化应用状态失败：{error}");
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .manage(state)
        // 单实例保护必须是第一个 plugin：第二个实例把参数交给首实例后直接退出。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            // 设了 HINT_DATA_DIR 时整套数据（settings / hint.db / logs / 模型）都在
            // 那个目录里。Tauri 的 `app_data_dir()` 不知道这个覆盖，所以这里必须
            // 用同一个推导函数——否则 Builder 阶段的 state 指向隔离目录、setup 里
            // 的日志却写进真实目录，两者会分叉。
            let data_dir = fallback_app_data_dir();
            let log_dir = data_dir.join("logs");
            logging::init(&log_dir);
            // 校验：只有在**没有**环境覆盖时，才与 Tauri 官方结果比对。
            // 有覆盖时两者必然不同（那是预期），比了只会报假警。
            let official_data_dir = app.path().app_data_dir()?;
            if std::env::var_os(DATA_DIR_ENV).is_none() && official_data_dir != data_dir {
                tracing::error!(
                    official = %official_data_dir.display(),
                    derived = %data_dir.display(),
                    "数据目录推导结果与 Tauri 不一致，配置可能读到另一份文件"
                );
            }
            if std::env::var_os(DATA_DIR_ENV).is_some() {
                tracing::warn!(
                    dir = %data_dir.display(),
                    "已启用数据目录覆盖（HINT_DATA_DIR）：本次运行不读写用户真实数据"
                );
            }
            session::emit_status(app.handle(), "idle", "等待连接");

            // 配置文件损坏：这里只记日志（日志与事件系统此时才就绪）。
            //
            // **不再在这里 emit_notice**：前端首屏的 listen 还没注册，事件不缓存，
            // 直接丢——真机验证时就是这个现象：日志有了、界面什么都没有。
            // 改由前端首屏主动 `invoke("config_corruption_notice")` 拉取。
            if let Some(state) = app.try_state::<AppState>()
                && state.settings_corrupted
            {
                tracing::warn!(
                    backup = %state.settings_path.with_extension("json.bad").display(),
                    "启动时配置文件损坏，已回退默认值"
                );
            }
            // 会话数据库降级到内存库：这里才写得出日志（同上一段的时序原因）。
            if let Some(state) = app.try_state::<AppState>()
                && state.store_degraded
            {
                tracing::error!(
                    path = %state.settings_path.parent().map(|dir| dir.join("hint.db").display().to_string()).unwrap_or_default(),
                    "启动时会话数据库打不开，已降级为内存库（历史会话记录不可读，重启后清空）"
                );
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 记住用户对浮层的调整（拖动 / 改大小），下次启动沿用。
            //
            // 为什么在事件里做防抖：拖动时 Resized/Moved 每帧都会触发，直接写盘
            // 会疯狂 IO。这里只在「最后一次变动之后 800ms」写一次。
            if window.label() == "overlay"
                && matches!(
                    event,
                    WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
                )
                && let Some(state) = window.try_state::<AppState>()
            {
                schedule_overlay_bounds_save(window.app_handle().clone(), &state);
            }

            if let WindowEvent::CloseRequested { api, .. } = event {
                // 关闭窗口 = 收进托盘，不退出应用；所有窗口都拦，避免浮层 /
                // 截图窗口被销毁后 `get_webview_window` 再也拿不到。
                api.prevent_close();
                let _ = window.hide();
                // 用户很容易以为点了 × 就是退出，第一次关闭主窗口时明确告知。
                if window.label() == "main"
                    && let Some(state) = window.try_state::<AppState>()
                    && !state.close_tip_shown.swap(true, Ordering::AcqRel)
                {
                    session::emit_notice(
                        window.app_handle(),
                        "info",
                        "窗口已隐藏到系统托盘；右键托盘图标可退出，按 Ctrl+Shift+H 显示浮层（左键托盘图标可恢复主窗口）。",
                    );
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::config_corruption_notice,
            commands::store_degraded_notice,
            commands::save_settings,
            commands::save_api_key,
            commands::has_api_key,
            commands::test_llm,
            commands::organize_profile,
            commands::rename_session,
            commands::export_all_sessions,
            commands::clean_empty_sessions,
            commands::count_empty_sessions,
            commands::search_sessions,
            commands::check_update,
            commands::download_update,
            commands::install_update,
            commands::profile_retrieve_preview,
            commands::fetch_models,
            commands::import_document_text,
            commands::start_session,
            commands::stop_session,
            commands::list_sessions,
            commands::session_detail,
            commands::export_session_file,
            commands::delete_session,
            commands::delete_all_sessions,
            commands::hide_overlay,
            commands::app_version,
            commands::overlay_is_visible,
            commands::toggle_overlay,
            commands::regenerate_answer,
            commands::ask_question,
            commands::list_audio_devices,
            commands::prepare_model,
            commands::verify_model_files,
            commands::model_status,
            commands::cancel_prepare,
            commands::remove_model,
            commands::start_capture,
            commands::cancel_capture,
            commands::finish_capture,
            commands::cancel_capture_answer,
            commands::open_log_dir,
            commands::doctor_report,
            commands::clean_legacy_model_dir,
        ])
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| {
            // 走到这里说明窗口/托盘/资源加载有致命问题，程序没法继续。
            // `expect` 会 panic——那时栈信息全走 stderr，而这是 GUI 子系统程序，
            // 用户双击启动时**根本看不到 stderr**，只会觉得「点了没反应」。
            // 改成显式记日志（此时 logging 已在 setup 之前初始化过）再退出。
            tracing::error!(%error, "构建 Tauri 应用失败，无法启动");
            eprintln!("hint 启动失败：{error}");
            std::process::exit(1);
        })
        .run(move |app, event| match event {
            RunEvent::Ready => {
                if let Err(error) = setup_after_ready(app) {
                    // 热键注册失败已在 register_shortcuts 里单独提示，
                    // 走到这里只可能是托盘没建起来，按托盘错误上报。
                    tracing::error!(%error, "托盘初始化失败");
                    session::emit_notice(app, "error", format!("托盘初始化失败：{error}"));
                }
            }
            RunEvent::ExitRequested { .. } => session::shutdown(app),
            // 托盘「退出」走 `app.exit(0)`，不保证经过 ExitRequested；
            // 这里再兜一次（stop_session 幂等，没有会话时直接返回 Err 被忽略）。
            RunEvent::Exit => session::shutdown(app),
            _ => {}
        });
}

/// `RunEvent::Ready` 在主线程上触发，`Menu` / 热键注册这类要求主线程的
/// API 可以同步完成。
fn setup_after_ready(app: &tauri::AppHandle) -> tauri::Result<()> {
    // 托盘与热键互不依赖：托盘构建失败（图标 / 菜单资源异常）不能让热键一起
    // 被跳过——主窗口关掉后，Ctrl+Shift+H 是用户唯一能取回浮层的入口。
    let tray = build_tray(app);
    register_shortcuts(app);
    tray?;
    match app.get_webview_window("overlay") {
        Some(window) => {
            // 默认尺寸/位置按主显示器算：高度 = 可用高度的 1/3，宽度 = 可用宽度的
            // 1/3（不小于 420 逻辑像素），水平居中、贴屏幕顶部。必须在窗口已经
            // 创建出来之后做（setup 阶段窗口由 tauri.conf.json 静态创建）。
            //
            // 但**用户调过的几何优先**：拖到别处或改过大小就沿用它，否则每次重启
            // 都把浮层摆回默认位置，用户白调一次。没有记录（或记录已不可用）时才
            // 用默认几何。
            let saved_bounds = {
                let state = app.state::<AppState>();
                Settings::load(&state.settings_path).overlay_bounds
            };
            let restored = saved_bounds
                .map(|bounds| apply_saved_overlay_geometry(&window, bounds))
                .unwrap_or(false);
            if !restored {
                apply_overlay_default_geometry(&window);
            }
            // 用户上次收起过浮层就不要再弹出来。
            //
            // `tauri.conf.json` 的 `overlay.visible = true` 是建窗初值，静态创建时
            // 窗口一出现就在屏幕上；这里按配置决定是否立刻隐藏。收起浮层通常意味着
            // 「现在不想让它挡着」，重启后原样弹回等于没听用户的。
            let want_visible = {
                let state = app.state::<AppState>();
                Settings::load(&state.settings_path).overlay_visible
            };
            if !want_visible {
                if let Err(error) = window.hide() {
                    tracing::warn!(%error, "按上次状态隐藏浮层失败");
                }
                // 隐藏态不该抢焦点。
                let _ = window.set_focusable(false);
                tracing::info!("浮层按上次状态保持隐藏（Ctrl+Shift+H 可显示）");
            }
            // 浮层默认不进入屏幕捕获：腾讯会议 / 钉钉 / OBS 这类共享屏幕时，
            // 面试官看到的是桌面，看不到浮层与答案。
            // 底层是 Windows 的 SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)
            // （Win10 2004+：从捕获中排除；更早的系统会退化成黑块）。
            // 用户可在设置里关掉（需要自己录屏留证时）。
            let protected = {
                let state = app.state::<AppState>();
                Settings::load(&state.settings_path).overlay_content_protected
            };
            match window.set_content_protected(protected) {
                Ok(()) => tracing::info!(protected, "浮层内容保护已设置"),
                Err(error) => tracing::warn!(
                    %error,
                    "设置浮层内容保护失败（共享屏幕时浮层可能被看到）"
                ),
            }
        }
        None => tracing::warn!("浮层窗口未创建，请检查 tauri.conf.json"),
    }
    // 浮层周期性重新置顶：tao 的 `set_always_on_top(true)` 在窗口已经是 TOPMOST
    // 时是空操作，别的置顶窗口（会议软件的置顶小窗等）一出现就会盖住浮层，
    // 而浮层自己不会回来——面试中看不见浮层等于产品失效。详见 commands 里的说明。
    commands::spawn_topmost_keeper(app.clone());
    Ok(())
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    if app.try_state::<TrayItems>().is_some() {
        return Ok(());
    }
    let settings_item = MenuItem::with_id(app, "settings", "打开设置", true, None::<&str>)?;
    let overlay_item = MenuItem::with_id(app, "overlay", "显示 / 隐藏浮层", true, None::<&str>)?;
    let session_start = MenuItem::with_id(app, "session-start", "开始面试", true, None::<&str>)?;
    let session_stop = MenuItem::with_id(app, "session-stop", "结束面试", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &settings_item,
            &overlay_item,
            &separator,
            &session_start,
            &session_stop,
            &separator,
            &quit,
        ],
    )?;

    let tray = TrayIconBuilder::new()
        .icon(
            app.default_window_icon()
                .cloned()
                .unwrap_or_else(|| tauri::image::Image::new_owned(vec![0u8; 4], 1, 1)),
        )
        .tooltip(TRAY_TOOLTIP_IDLE)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "settings" => show_main(app),
            "overlay" => toggle_overlay(app),
            "session-start" => session::spawn_start(app),
            "session-stop" => session::spawn_stop(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(TrayItems {
        session_start: session_start.clone(),
        session_stop: session_stop.clone(),
        tray,
    });
    Ok(())
}

fn register_shortcuts(app: &tauri::AppHandle) {
    let accelerators: Vec<&str> = SHORTCUTS
        .iter()
        .map(|(accelerator, _)| *accelerator)
        .collect();
    let result = app
        .global_shortcut()
        .on_shortcuts(accelerators, |app, shortcut, event| {
            if event.state == ShortcutState::Pressed {
                handle_pressed_shortcut(app, *shortcut);
            }
        });
    if let Err(error) = result {
        tracing::warn!(%error, "全局热键注册失败");
        commands::emit_hotkey_failure(app, &error.to_string());
    }
}

fn handle_pressed_shortcut(app: &tauri::AppHandle, shortcut: Shortcut) {
    if shortcut == combo(Code::KeyH) {
        toggle_overlay(app);
    } else if shortcut == combo(Code::KeyA) {
        session::spawn_regenerate(app);
    } else if shortcut == combo(Code::KeyR) {
        // 与设置页/命令层的切换一致：方向在生命周期队列锁内决定，
        // 启动中再按一次会排队停止，而不是被合并成一个无效的 Start。
        session::spawn_toggle(app);
    } else if shortcut == combo(Code::KeyS) {
        // M5 起截图问答已接通：热键直接拉起截图选区窗口（与设置页/托盘走
        // 同一个命令），失败只提示不弹窗。
        if let Err(error) = commands::start_capture(app.clone()) {
            session::emit_notice(app, "error", format!("截图问答启动失败：{error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1080p（工作区 1920×1040、任务栏 40px、100% 缩放）：
    /// 高度 = 1040/3 = 346，宽度 = 1920/3 = 640（落在 420–900 之间），
    /// 水平居中 x = (1920-640)/2 = 640，垂直贴工作区上沿 y = 0。
    #[test]
    fn overlay_geometry_uses_third_of_work_area_on_1080p() {
        let (size, position) = overlay_default_geometry((0, 0, 1920, 1040), 1.0);
        assert_eq!(size.width, 640);
        assert_eq!(size.height, 346);
        assert_eq!(position.x, 640);
        assert_eq!(position.y, 0);
    }

    /// 竖直方向上「贴顶」用的是**工作区**原点：任务栏在顶部时原点 y 不是 0，
    /// 浮层必须从工作区上沿开始，否则会被任务栏压住。
    #[test]
    fn overlay_geometry_anchors_to_work_area_origin() {
        let (_, position) = overlay_default_geometry((0, 48, 1920, 1032), 1.0);
        assert_eq!(position.y, 48, "任务栏在顶部时贴的是工作区上沿");
    }

    /// 宽度下限按逻辑像素给：1366×768 @150% 时可用宽度 1366/3 ≈ 455，
    /// 但 420 逻辑像素 × 1.5 = 630 物理像素，必须抬到 630。
    #[test]
    fn overlay_width_respects_minimum_in_logical_pixels() {
        let (size, _) = overlay_default_geometry((0, 0, 1366, 728), 1.5);
        assert_eq!(size.width, 630);
        assert_eq!(size.height, 242);
    }

    /// 超宽屏（3440×1400）：1/3 = 1146 超过 900 上限，按上限收窄；
    /// 水平居中仍然相对可用区域算。
    #[test]
    fn overlay_width_is_capped_on_ultrawide_monitors() {
        let (size, position) = overlay_default_geometry((0, 0, 3440, 1400), 1.0);
        assert_eq!(size.width, 900);
        assert_eq!(size.height, 466);
        assert_eq!(position.x, (3440 - 900) / 2);
    }

    /// 极小屏（800×600）：1/3 = 266 低于下限 420，但屏幕只有 800 宽，
    /// 允许取 420（仍小于屏幕宽度）；高度 200 保持 1/3。
    #[test]
    fn overlay_geometry_handles_small_screens() {
        let (size, position) = overlay_default_geometry((0, 0, 800, 600), 1.0);
        assert_eq!(size.width, 420);
        assert_eq!(size.height, 200);
        assert_eq!(position.x, (800 - 420) / 2);
    }

    /// 脏缩放比（0 / 负数 / NaN / 无穷）按 100% 处理，不能算出 0 宽窗口。
    #[test]
    fn overlay_geometry_falls_back_on_invalid_scale_factor() {
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let (size, _) = overlay_default_geometry((0, 0, 1920, 1040), scale);
            assert_eq!(size.width, 640, "缩放比 {scale} 应按 100% 处理");
            assert_eq!(size.height, 346);
        }
    }

    /// 浮层位置的**屏外回退**：交集不足 1/3 就丢回默认几何。
    ///
    /// 判据改错的后果分两个方向，都值得防：
    /// - 阈值改大（1/3 → 1/2）：用户在屏幕边缘摆的浮层被**白白丢掉**，
    ///   每次启动都回到默认位置，用户调的尺寸不保留；
    /// - 阈值改小：屏外的浮层被**保留**，用户看不见也拖不回来。
    ///
    /// 工作区用 1920×1040（扣掉任务栏），浮层取常见的 640×346。
    #[test]
    fn overlay_geometry_fits_screen_boundary_cases() {
        let area = (0i32, 0i32, 1920u32, 1040u32);
        let (w, h) = (640.0f64, 346.0f64);

        // 完全在屏幕内 → 保留
        assert!(overlay_geometry_fits(
            100.0, 100.0, w, h, area.0, area.1, area.2, area.3
        ));

        // 完全在屏幕外（右下方很远）→ 丢弃
        assert!(!overlay_geometry_fits(
            5000.0, 5000.0, w, h, area.0, area.1, area.2, area.3
        ));

        // 左上角完全在屏外 → 丢弃
        assert!(!overlay_geometry_fits(
            -1000.0, -1000.0, w, h, area.0, area.1, area.2, area.3
        ));

        // 恰好一半在屏内（交集 = 1/2 ≥ 1/3）→ 保留
        // 浮层 640 宽，从 x=-320 开始 → 右半 320 在屏内，交集 = 320×346
        assert!(overlay_geometry_fits(
            -320.0, 100.0, w, h, area.0, area.1, area.2, area.3
        ));

        // 恰好 1/3 在屏内（交集 = 1/3）→ **保留**（判据是 >=）
        // 浮层 640 宽，从 x=-1280 开始 → 只有右 1280/640 错，实际交集 213.3…
        // 精确构造：让交集面积恰好等于 total/3。
        // total = 640×346 = 221440，1/3 ≈ 73813.3。
        // 交集宽 = 640 × (1/3) / 346 … 用高度方向更直观：
        // 浮层从 y = -h/3 开始 → 交集高 = h × 2/3，面积 = w × 2h/3 > total/3。
        // 反过来从 y = 2h/3 起（浮层下半部分在屏外）→ 交集高 = h/3，
        // 面积 = w × h/3 = total/3 → 恰好等于阈值。
        assert!(
            overlay_geometry_fits(
                100.0,
                100.0 + (2.0 * h / 3.0),
                w,
                h,
                area.0,
                area.1,
                area.2,
                area.3
            ),
            "交集恰好 1/3 时应保留（判据是 >=）"
        );

        // 交集略小于 1/3 → 丢弃
        let y_just_short = 100.0 + (2.0 * h / 3.0) + 1.0;
        assert!(
            !overlay_geometry_fits(100.0, y_just_short, w, h, area.0, area.1, area.2, area.3),
            "交集略小于 1/3 应丢弃"
        );

        // 零尺寸 / 负尺寸 → 丢弃
        assert!(!overlay_geometry_fits(
            100.0, 100.0, 0.0, h, area.0, area.1, area.2, area.3
        ));
        assert!(!overlay_geometry_fits(
            100.0, 100.0, w, -1.0, area.0, area.1, area.2, area.3
        ));
    }

    /// 判据的**阈值方向**：交集 >= 1/3 才保留。
    ///
    /// 单独钉这一条，因为「恰好等于」是最容易写错的边界——把 `>=` 写成 `>`，
    /// 交集恰好 1/3 的浮层会被误判为屏外而丢弃。
    #[test]
    fn overlay_geometry_fits_threshold_is_at_least() {
        let area = (0i32, 0i32, 1920u32, 1040u32);
        let w = 600.0f64;
        let h = 300.0f64;
        let total = w * h;

        // 构造交集恰好等于 total/3：浮层下半 2/3 在屏外
        let y = 100.0 + (2.0 * h / 3.0);
        assert!(overlay_geometry_fits(
            100.0, y, w, h, area.0, area.1, area.2, area.3
        ));

        // 同一位置，但浮层略高一点（交集略小于 1/3）
        let y2 = y + 0.5;
        assert!(!overlay_geometry_fits(
            100.0, y2, w, h, area.0, area.1, area.2, area.3
        ));
    }
}
