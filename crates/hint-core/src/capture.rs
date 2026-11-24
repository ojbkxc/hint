//! 区域截图：把用户框选的那块屏幕抓成 JPEG（base64），交给视觉模型读图。
//!
//! 边界（安全相关，动这个文件之前先读完）：
//!
//! - 只抓**调用方显式传进来的那个矩形**。这里没有全屏抓取入口、没有定时截屏、
//!   没有后台循环；想把范围放大，只能改这个模块的 API，逼着改动出现在评审里。
//! - 像素只在内存里活一轮：位图 → RGB → JPEG → base64 字符串，全程不落盘、
//!   不进日志、不进 tracing（日志只记宽高和编码后的字节数）。一张截图里完全
//!   可能有用户不想外传的东西。
//! - 编码器选 `jpeg-encoder` 而不是 `image`：我们只编码不解码，而 `image` 会把
//!   整棵解码依赖（zune-jpeg 之类）一起拖进来，包体和攻击面都白白变大。
//!
//! 坐标系：Win32 抓屏吃物理像素，前端框选拿到的是 CSS 像素，两者差一个设备
//! 缩放比（dpr）。换算只在 [`normalize_region`] 里做一次，调用方别自己乘。

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

/// 单边像素上限。超了直接报错，不帮你缩：悄悄瘦身会让用户以为模型看到的是
/// 完整画面，实际细节被抽掉了，出问题比报错更难查。
pub const MAX_SIDE: u32 = 4096;

/// base64 之后的体积上限，单位 MiB。模型接口对单图体积敏感，超了整次请求都会
/// 被拒，不如在本地就把话说清楚。
const MAX_BASE64_MIB: usize = 4;

/// [`MAX_SIDE`] 的字节版本：base64 字符串长度超过它就没有意义再往后传了。
pub const MAX_BASE64_BYTES: usize = MAX_BASE64_MIB * 1024 * 1024;

/// JPEG 质量。80 是「截屏里的文字还看得清」和「体积能接受」之间的折中点。
const JPEG_QUALITY: u8 = 80;

/// 一块屏幕矩形，单位是**物理像素**（左上角 + 宽高）。
///
/// `x`/`y` 允许为负：副屏挂在主屏左边/上边时，虚拟桌面的起点就是负的。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// 抓好的截图，`data_base64` 可以直接塞进模型请求的 `image_url.data`。
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CapturedImage {
    pub mime: String,
    pub data_base64: String,
    pub width: u32,
    pub height: u32,
}

impl fmt::Debug for CapturedImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 故意不打印 base64：这个结构体会被随手 `{:?}` 进日志，而 base64 里
        // 装的就是用户的屏幕内容。要看体积就看长度，要还原就自己解。
        f.debug_struct("CapturedImage")
            .field("mime", &self.mime)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("data_base64_len", &self.data_base64.len())
            .finish()
    }
}

/// 截图失败的原因。文案是给用户看的，写「请怎么做」而不是「哪一步 null 了」。
#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("没有框选到区域：请拖出一个至少 1×1 像素的矩形")]
    EmptyRegion,

    #[error("{}", too_large_message(*width, *height))]
    TooLarge { width: u32, height: u32 },

    #[error("选区不在屏幕上：可能切换了显示器或改了分辨率，请重新框选")]
    OutOfBounds,

    /// 平台/驱动层的失败。`String` 已经是中文说明，末尾可能带原始错误码，
    /// 方便用户报障时直接抄给我们。
    #[error("{0}")]
    Platform(String),
}

/// `TooLarge` 的文案要同时覆盖「选区尺寸超限」和「编码后体积超限」两个触发点，
/// 所以单独抽出来——和 `error.rs` 里 `transport_message` 一个路子。
fn too_large_message(width: u32, height: u32) -> String {
    let limit = format!("单边最多 {MAX_SIDE} 像素，且编码后不超过 {MAX_BASE64_MIB} MiB");
    format!("截图区域过大（{width}×{height} 像素）：{limit}。请缩小选区后重试。")
}

/// CSS 像素矩形 + 设备缩放比 → 物理像素整数矩形。
///
/// - 拖拽方向任意：宽高为负时先把「右下角」归成左上角，见 [`anchor_left_top`]。
/// - dpr 非法（<= 0 / NaN / ∞）按 1.0 处理：算偏一两个像素远好过整块截图作废。
/// - 左右两条边各自四舍五入后相减，而不是把宽高单独取整：后者在 x.5 的情况下
///   会多抓或少抓一列，而且偏差会随选区变大累积。
pub fn normalize_region(x: f64, y: f64, width: f64, height: f64, dpr: f64) -> Region {
    let dpr = sanitize_dpr(dpr);
    let (left, width) = anchor_left_top(x, width);
    let (top, height) = anchor_left_top(y, height);
    let left_px = (left * dpr).round();
    let top_px = (top * dpr).round();
    let width_px = ((left + width) * dpr).round() - left_px;
    let height_px = ((top + height) * dpr).round() - top_px;
    // 浮点 `as` 是饱和转换：NaN 变 0、越界变边界值，不会 panic，也不会像
    // `try_into` 那样逼出一堆「前端传了脏数据」的错误分支。
    Region {
        x: left_px as i32,
        y: top_px as i32,
        width: width_px.max(0.0) as u32,
        height: height_px.max(0.0) as u32,
    }
}

/// 把「可能为负的起点 + 带符号的尺寸」规整成「左上角 + 正尺寸」。
fn anchor_left_top(origin: f64, extent: f64) -> (f64, f64) {
    if extent < 0.0 {
        return (origin + extent, -extent);
    }
    (origin, extent)
}

/// 缩放比兜底。写成早返回而不是 `if … { a } else { b }`，是为了避免表达式的
/// 换行位置随宽度阈值漂移（那一行刚好卡在 rustfmt 的单行阈值上）。
fn sanitize_dpr(dpr: f64) -> f64 {
    if !dpr.is_finite() || dpr <= 0.0 {
        return 1.0;
    }
    dpr
}

/// 把选区裁到屏幕内。
///
/// 用户可能框完又切了显示器、改了分辨率，于是「框的时候合法、抓的时候越界」；
/// 这时候裁剪比直接失败友好，但裁完什么都不剩就必须报错——否则上层会拿着一
/// 张 0×0 的图去调模型，错误信息跑到很远的地方才冒出来。
pub fn clamp_region(region: Region, screen: Region) -> Result<Region, CaptureError> {
    // 全程用 i64 算：左上角是 i32、宽高是 u32，边界上直接相加会溢出。
    let screen_left = i64::from(screen.x);
    let screen_top = i64::from(screen.y);
    let screen_right = screen_left + i64::from(screen.width);
    let screen_bottom = screen_top + i64::from(screen.height);
    let left = i64::from(region.x).max(screen_left);
    let top = i64::from(region.y).max(screen_top);
    let right = (i64::from(region.x) + i64::from(region.width)).min(screen_right);
    let bottom = (i64::from(region.y) + i64::from(region.height)).min(screen_bottom);
    if right <= left || bottom <= top {
        return Err(CaptureError::OutOfBounds);
    }
    // left/top 已经被夹进屏幕范围，回 i32 不会截断。
    Ok(Region {
        x: left as i32,
        y: top as i32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
}

/// 校验选区本身是否可用（非空、单边不超限）。
///
/// 先校验再接屏幕裁剪：顺序反过来的话，一个 8000 像素宽的框会被悄悄裁成屏幕
/// 宽度，「超限」这条分支就永远查不出来了。
pub fn validate_region(region: Region) -> Result<(), CaptureError> {
    if region.width == 0 || region.height == 0 {
        return Err(CaptureError::EmptyRegion);
    }
    if region.width > MAX_SIDE || region.height > MAX_SIDE {
        return Err(CaptureError::TooLarge {
            width: region.width,
            height: region.height,
        });
    }
    Ok(())
}

/// 把用户框选的区域抓成 JPEG（base64）。
///
/// `region` 必须是**物理像素**；前端把 CSS 像素算成物理像素要用
/// [`normalize_region`]，不要自己乘 dpr。
#[cfg(windows)]
pub fn capture_region(region: Region) -> Result<CapturedImage, CaptureError> {
    validate_region(region)?;
    let region = clamp_region(region, virtual_screen_region())?;
    let pixels = win32::grab_bgra(region)?;
    let rgb = encode::bgra_to_rgb(&pixels);
    encode::assemble(&rgb, region)
}

/// 非 Windows（CI 的 fmt/clippy 阶段、开发者本机 macOS）直接说清楚不支持，
/// 而不是回退成「黑图」让上层以为抓成功了。
#[cfg(not(windows))]
pub fn capture_region(_region: Region) -> Result<CapturedImage, CaptureError> {
    Err(platform("截图目前只支持 Windows 平台"))
}

/// 虚拟桌面（所有显示器拼成的大矩形）的范围，物理像素。
///
/// 多显示器时 `x`/`y` 可能为负，这正是 BitBlt 想要的坐标系。
#[cfg(windows)]
pub fn virtual_screen_region() -> Region {
    win32::virtual_screen_region()
}

/// 非 Windows 拿不到屏幕指标，返回零尺寸；调用方配合 [`validate_region`] 就能
/// 把「平台不支持」和「选区为空」区分开。
#[cfg(not(windows))]
pub fn virtual_screen_region() -> Region {
    Region::default()
}

/// 平台层失败的统一入口：文案一定要中文、一定要说得清是哪一步。
fn platform(message: impl Into<String>) -> CaptureError {
    CaptureError::Platform(message.into())
}

#[cfg(windows)]
mod win32 {
    //! Win32 GDI 抓屏。句柄全靠 guard 释放：GDI 句柄是进程级配额（默认一万个），
    //! 漏一个就少一个，截上几百次后会开始「随机」失败，而且报错点离真凶很远。

    use super::{CaptureError, Region, platform};
    use std::mem::size_of;
    use windows::Win32::Graphics::{Dwm, Gdi};
    use windows::Win32::UI::WindowsAndMessaging;

    /// 屏幕 DC（`GetDC(NULL)`）要用 `ReleaseDC` 归还，不能用 `DeleteDC`。
    struct ScreenDc(Gdi::HDC);

    impl Drop for ScreenDc {
        fn drop(&mut self) {
            // SAFETY: 句柄来自 GetDC(None)，且只在这里归还一次。
            unsafe {
                let _ = Gdi::ReleaseDC(None, self.0);
            }
        }
    }

    /// 内存 DC 用 `DeleteDC`；`ReleaseDC` 对它无效（会静默失败并泄漏）。
    struct MemoryDc(Gdi::HDC);

    impl Drop for MemoryDc {
        fn drop(&mut self) {
            // SAFETY: 句柄来自 CreateCompatibleDC，且只在这里删一次。
            unsafe {
                let _ = Gdi::DeleteDC(self.0);
            }
        }
    }

    /// 位图是 GDI 对象，用 `DeleteObject` 回收（它既不是 DC 也不是窗口）。
    struct Bitmap(Gdi::HBITMAP);

    impl Drop for Bitmap {
        fn drop(&mut self) {
            // SAFETY: 句柄来自 CreateCompatibleBitmap；内存 DC 里原先选中的对象
            // 已由 `SelectedBitmap` 换回去了，此时删除不会失败。返回值仍忽略：
            // Drop 里没地方报错，而且清理失败也不应该让整个截图流程 panic。
            unsafe {
                let _ = Gdi::DeleteObject(Gdi::HGDIOBJ(self.0.0));
            }
        }
    }

    /// 记下内存 DC 里**原来**选中的对象，Drop 时换回去。
    /// 不换就 DeleteObject：位图还被 DC 占着，删除失败 → 泄漏一个位图。
    struct SelectedBitmap {
        dc: Gdi::HDC,
        previous: Gdi::HGDIOBJ,
    }

    impl Drop for SelectedBitmap {
        fn drop(&mut self) {
            // SAFETY: `previous` 是刚才 SelectObject 返回的原对象，句柄仍有效。
            unsafe {
                let _ = Gdi::SelectObject(self.dc, self.previous);
            }
        }
    }

    /// 读一个虚拟桌面指标。副屏挂在主屏左边/上边时，返回的坐标是负数。
    fn metric(index: WindowsAndMessaging::SYSTEM_METRICS_INDEX) -> i32 {
        // SAFETY: GetSystemMetrics 只读系统指标，无副作用，也不需要配对释放。
        unsafe { WindowsAndMessaging::GetSystemMetrics(index) }
    }

    pub(super) fn virtual_screen_region() -> Region {
        let x = metric(WindowsAndMessaging::SM_XVIRTUALSCREEN);
        let y = metric(WindowsAndMessaging::SM_YVIRTUALSCREEN);
        let width = metric(WindowsAndMessaging::SM_CXVIRTUALSCREEN);
        let height = metric(WindowsAndMessaging::SM_CYVIRTUALSCREEN);
        // 拿不到指标时 GetSystemMetrics 返回 0；max(0) 顺便挂底，防止负数
        // 转 u32 后变成一个天文数字的宽度。
        Region {
            x,
            y,
            width: width.max(0) as u32,
            height: height.max(0) as u32,
        }
    }

    /// 抓屏并把像素读成 BGRA 缓冲。
    pub(super) fn grab_bgra(region: Region) -> Result<Vec<u8>, CaptureError> {
        // 调用前已经过 validate_region：宽高 <= MAX_SIDE(4096)，转 i32 不会截断。
        let width = region.width as i32;
        let height = region.height as i32;
        // 先等合成落地：刚弹出的选区遮罩如果还没合成完，会抓到上一帧的画面。
        // 远端桌面或关了 DWM 时这里会报错，忽略继续抓就行。
        unsafe {
            let _ = Dwm::DwmFlush();
        }
        // SAFETY: 全是 Win32 句柄调用；返回的句柄逐个判 is_invalid 再使用，释放
        // 统一交给上面的 guard。局部变量按声明的逆序析构，于是恰好是
        // SelectedBitmap（换回原对象）→ Bitmap → MemoryDc → ScreenDc，正合顺序。
        unsafe {
            let screen = ScreenDc(Gdi::GetDC(None));
            if screen.0.is_invalid() {
                return Err(platform("拿不到屏幕设备上下文（GetDC 失败）"));
            }
            let memory = MemoryDc(Gdi::CreateCompatibleDC(Some(screen.0)));
            if memory.0.is_invalid() {
                return Err(platform("创建内存设备上下文失败（CreateCompatibleDC）"));
            }
            let bitmap = Bitmap(Gdi::CreateCompatibleBitmap(screen.0, width, height));
            if bitmap.0.is_invalid() {
                return Err(platform("创建位图失败（CreateCompatibleBitmap）"));
            }
            let selected = SelectedBitmap {
                dc: memory.0,
                previous: Gdi::SelectObject(memory.0, Gdi::HGDIOBJ(bitmap.0.0)),
            };
            if selected.previous.is_invalid() {
                return Err(platform("把位图选进内存 DC 失败（SelectObject）"));
            }
            // CAPTUREBLT 让分层窗口（阴影、半透明面板）也进画面；不加会抓到
            // 「选区里有个洞」的效果。
            Gdi::BitBlt(
                memory.0,
                0,
                0,
                width,
                height,
                Some(screen.0),
                region.x,
                region.y,
                Gdi::SRCCOPY | Gdi::CAPTUREBLT,
            )
            .map_err(|error| platform(format!("抓取屏幕像素失败（BitBlt）：{error}")))?;
            // 高度取负 = 自上而下存行序，省掉一次垂直翻转。
            let mut info = Gdi::BITMAPINFO {
                bmiHeader: Gdi::BITMAPINFOHEADER {
                    biSize: size_of::<Gdi::BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: Gdi::BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = vec![0u8; width as usize * height as usize * 4];
            let scanned = Gdi::GetDIBits(
                memory.0,
                bitmap.0,
                0,
                height as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut info,
                Gdi::DIB_RGB_COLORS,
            );
            if scanned == 0 {
                return Err(platform("读取截图像素失败（GetDIBits）"));
            }
            Ok(pixels)
        }
    }
}

/// JPEG 编码 + base64 这一段跟 Win32 无关，是纯数据处理；只有 Windows 抓屏链路
/// 会调它，所以非 Windows 下允许它「暂时没人用」，免得本机 `cargo check` 报警。
#[cfg_attr(not(windows), allow(dead_code))]
mod encode {
    use super::{CaptureError, CapturedImage, JPEG_QUALITY, MAX_BASE64_BYTES, Region};
    use base64::Engine;
    use tracing::debug;

    /// BGRA → RGB。BitBlt 给的是 32 位像素，编码器只要 24 位。
    pub(super) fn bgra_to_rgb(bgra: &[u8]) -> Vec<u8> {
        // 顺手丢掉 alpha：屏幕上没有透明度，留着只是白占 1/4 内存。
        let mut rgb = Vec::with_capacity(bgra.len() / 4 * 3);
        // as_chunks 比 chunks_exact 好在类型上就把「一整像素」写死了：
        // pixel 是 &[u8; 4]，不会因为切分逻辑改动而静默错位。
        for pixel in bgra.as_chunks::<4>().0 {
            rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
        rgb
    }

    /// 编码后的体积也要卡一道：尺寸没超不代表 base64 长度没超。
    pub(super) fn check_encoded_size(len: usize, region: Region) -> Result<(), CaptureError> {
        if len > MAX_BASE64_BYTES {
            return Err(CaptureError::TooLarge {
                width: region.width,
                height: region.height,
            });
        }
        Ok(())
    }

    pub(super) fn assemble(rgb: &[u8], region: Region) -> Result<CapturedImage, CaptureError> {
        let jpeg = jpeg_of(rgb, region)?;
        // 标准 base64（带 padding）：视觉模型接口都吃这个口味。
        let data_base64 = base64::engine::general_purpose::STANDARD.encode(&jpeg);
        check_encoded_size(data_base64.len(), region)?;
        // 只记尺寸和字节数：日志是纯文本、会进用户排障包里，沾不得像素。
        debug!(
            width = region.width,
            height = region.height,
            jpeg_bytes = jpeg.len(),
            base64_bytes = data_base64.len(),
            "区域截图编码完成"
        );
        Ok(CapturedImage {
            mime: "image/jpeg".to_string(),
            data_base64,
            width: region.width,
            height: region.height,
        })
    }

    fn jpeg_of(rgb: &[u8], region: Region) -> Result<Vec<u8>, CaptureError> {
        // MAX_SIDE 保证宽高都 <= 4096，转 u16 不会截断。
        let width = region.width as u16;
        let height = region.height as u16;
        // 输出到内存里的 Vec，不用 `Encoder::new_file`：像素不落盘。
        let mut jpeg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg, JPEG_QUALITY)
            .encode(rgb, width, height, jpeg_encoder::ColorType::Rgb)
            .map_err(|error| CaptureError::Platform(format!("编码为 JPEG 失败：{error}")))?;
        Ok(jpeg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> Region {
        Region {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn normalize_keeps_css_rect_at_dpr_one() {
        let region = normalize_region(100.0, 200.0, 300.0, 150.0, 1.0);
        assert_eq!(region, rect(100, 200, 300, 150));
    }

    #[test]
    fn normalize_scales_by_dpr() {
        // 150% 缩放：CSS 100×50 → 物理 150×75，位置也跟着乘。
        let region = normalize_region(10.0, 20.0, 100.0, 50.0, 1.5);
        assert_eq!(region, rect(15, 30, 150, 75));
    }

    #[test]
    fn normalize_handles_negative_drag_direction() {
        // 从右下往左上拖：宽高都是负的，结果仍必须是「左上角 + 正尺寸」。
        let region = normalize_region(400.0, 300.0, -100.0, -75.0, 1.0);
        assert_eq!(region, rect(300, 225, 100, 75));
    }

    #[test]
    fn normalize_falls_back_to_dpr_one() {
        // 0、负数、NaN、∞ 都当 1.0：宁可偏一两像素，也不要把整块截图搞废。
        for dpr in [0.0, -1.5, f64::NAN, f64::INFINITY] {
            let region = normalize_region(10.0, 10.0, 20.0, 20.0, dpr);
            assert_eq!(region, rect(10, 10, 20, 20), "dpr={dpr} 应按 1.0 处理");
        }
    }

    #[test]
    fn normalize_turns_nan_size_into_empty_region() {
        // 前端传了脏数据时，`as` 的饱和转换把 NaN 归成 0，结果会被下面的
        // 校验拦住并报「空选区」，而不是一场 panic。
        let region = normalize_region(f64::NAN, f64::NAN, f64::NAN, f64::NAN, 1.0);
        assert_eq!(region.width, 0);
        assert_eq!(region.height, 0);
        assert!(matches!(
            validate_region(region),
            Err(CaptureError::EmptyRegion)
        ));
    }

    #[test]
    fn clamp_keeps_region_inside_screen_untouched() {
        let screen = rect(0, 0, 1920, 1080);
        let region = clamp_region(rect(100, 100, 400, 300), screen).expect("应原样通过");
        assert_eq!(region, rect(100, 100, 400, 300));
    }

    #[test]
    fn clamp_clips_each_edge() {
        let screen = rect(0, 0, 1920, 1080);
        // 左边越界：左边贴 0，宽度减掉被切掉的 50。
        let left = clamp_region(rect(-50, 100, 200, 100), screen).expect("应裁剪");
        assert_eq!(left, rect(0, 100, 150, 100));
        // 上边越界。
        let top = clamp_region(rect(100, -50, 200, 100), screen).expect("应裁剪");
        assert_eq!(top, rect(100, 0, 200, 50));
        // 右下越界：宽高都缩。
        let corner = clamp_region(rect(1800, 1000, 400, 300), screen).expect("应裁剪");
        assert_eq!(corner, rect(1800, 1000, 120, 80));
    }

    #[test]
    fn clamp_supports_negative_origin_screen() {
        // 副屏挂在主屏左边时，虚拟桌面的原点是负的。
        let screen = rect(-1920, 0, 3840, 1080);
        let region = clamp_region(rect(-2000, -20, 200, 200), screen).expect("应裁剪");
        assert_eq!(region, rect(-1920, 0, 120, 180));
    }

    #[test]
    fn clamp_rejects_region_completely_off_screen() {
        let screen = rect(0, 0, 1920, 1080);
        let away = clamp_region(rect(5000, 5000, 100, 100), screen);
        assert!(matches!(away, Err(CaptureError::OutOfBounds)));
        // 紧贴屏幕右边、零重叠，也抓不出像素，一样算越界。
        let touching = clamp_region(rect(1920, 0, 100, 100), screen);
        assert!(matches!(touching, Err(CaptureError::OutOfBounds)));
        // 屏幕指标拿不到时是零尺寸，必须报错，而不是返回一张 0×0 的图。
        let blind = clamp_region(rect(0, 0, 10, 10), rect(0, 0, 0, 0));
        assert!(matches!(blind, Err(CaptureError::OutOfBounds)));
    }

    #[test]
    fn validate_reports_empty_and_oversized() {
        assert!(matches!(
            validate_region(rect(0, 0, 0, 10)),
            Err(CaptureError::EmptyRegion)
        ));
        assert!(matches!(
            validate_region(rect(0, 0, 10, 0)),
            Err(CaptureError::EmptyRegion)
        ));
        // 恰好等于上限要放行：否则 4K 整屏截图会被白白卡掉。
        assert!(validate_region(rect(0, 0, MAX_SIDE, MAX_SIDE)).is_ok());
        match validate_region(rect(0, 0, MAX_SIDE + 1, 10)) {
            Err(CaptureError::TooLarge { width, height }) => {
                assert_eq!(width, MAX_SIDE + 1);
                assert_eq!(height, 10);
            }
            other => panic!("应报 TooLarge，实际：{other:?}"),
        }
    }

    #[test]
    fn encoded_size_limit_is_enforced() {
        let region = rect(0, 0, 100, 100);
        assert!(encode::check_encoded_size(MAX_BASE64_BYTES, region).is_ok());
        let over = encode::check_encoded_size(MAX_BASE64_BYTES + 1, region);
        assert!(matches!(over, Err(CaptureError::TooLarge { .. })));
    }

    #[test]
    fn bgra_to_rgb_swaps_channel_order() {
        // [B, G, R, A] → [R, G, B]，alpha 直接丢掉。
        let bgra = [10u8, 20, 30, 0, 40, 50, 60, 255];
        assert_eq!(encode::bgra_to_rgb(&bgra), vec![30, 20, 10, 60, 50, 40]);
        assert!(encode::bgra_to_rgb(&[]).is_empty());
    }

    #[test]
    fn assemble_returns_base64_jpeg() {
        // 2×2 纯红。这条用例同时盯三件事：编码链路通、base64 能解回 JPEG、
        // 元信息没接反。把编码器换掉时它会最先响。
        let rgb = vec![255u8, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0];
        let image = encode::assemble(&rgb, rect(0, 0, 2, 2)).expect("编码应成功");
        assert_eq!(image.mime, "image/jpeg");
        assert_eq!((image.width, image.height), (2, 2));
        let decoded = base64::engine::general_purpose::STANDARD.decode(&image.data_base64);
        let bytes = decoded.expect("base64 应可解码");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "缺少 SOI");
        assert_eq!(&bytes[bytes.len() - 2..], &[0xFF, 0xD9], "缺少 EOI");
    }

    #[test]
    fn debug_never_prints_the_payload() {
        let image = CapturedImage {
            mime: "image/jpeg".to_string(),
            data_base64: "U0VDUkVUUElYRUxT".to_string(),
            width: 2,
            height: 2,
        };
        let shown = format!("{image:?}");
        assert!(
            !shown.contains("U0VDUkVU"),
            "Debug 不得打印截图内容：{shown}"
        );
        assert!(shown.contains("image/jpeg"));
        assert!(shown.contains("data_base64_len"));
    }

    #[test]
    fn error_messages_are_chinese_and_carry_numbers() {
        assert!(CaptureError::EmptyRegion.to_string().contains("框选"));
        assert!(CaptureError::OutOfBounds.to_string().contains("不在屏幕上"));
        let too_large = CaptureError::TooLarge {
            width: 5000,
            height: 10,
        };
        let text = too_large.to_string();
        // 用户看到的是这句话，所以尺寸和两个上限都必须出现在里面。
        assert!(text.contains("5000×10"), "要带实际尺寸：{text}");
        assert!(text.contains("4096"), "要带单边上限：{text}");
        assert!(text.contains("4 MiB"), "要带体积上限：{text}");
        let wrapped = CaptureError::Platform("抓屏失败".to_string());
        assert_eq!(wrapped.to_string(), "抓屏失败");
    }
}
