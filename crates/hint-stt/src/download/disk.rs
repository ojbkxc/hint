//! 磁盘空间查询。预检失败（拿不到数值）时按「未知」处理，不阻断下载。

use std::path::{Path, PathBuf};

/// 查询 `path` 所在卷的可用字节数。路径不存在时沿祖先向上找。
///
/// 返回 `None` 表示平台暂不支持或调用失败——此时跳过预检。
pub fn free_bytes(path: &Path) -> Option<u64> {
    let anchor = existing_ancestor(path)?;
    platform_free_bytes(&anchor)
}

fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = {
        let mut buf = PathBuf::new();
        for component in path.components() {
            buf.push(component);
        }
        buf
    };
    loop {
        if current.exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

#[cfg(windows)]
fn platform_free_bytes(path: &Path) -> Option<u64> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let wide: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut available: u64 = 0;
    let mut total: u64 = 0;
    let mut free: u64 = 0;
    // SAFETY: `wide` 是以 NUL 结尾的 UTF-16 缓冲区，三个输出指针都指向本栈帧内的有效 u64。
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            &mut total,
            &mut free,
        )
    };
    if ok == 0 { None } else { Some(available) }
}

#[cfg(unix)]
fn platform_free_bytes(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: 缓冲区由 `statvfs` 填充；返回值 0 表示成功。
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return None;
    }
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}

#[cfg(not(any(windows, unix)))]
fn platform_free_bytes(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_some_space_for_the_temp_dir() {
        if let Some(free) = free_bytes(&std::env::temp_dir()) {
            assert!(free > 0);
        }
    }

    #[test]
    fn missing_path_falls_back_to_ancestor() {
        let dir = std::env::temp_dir().join("hint-stt-does-not-exist-xyz");
        if let Some(free) = free_bytes(&dir) {
            assert!(free > 0);
        }
    }
}
