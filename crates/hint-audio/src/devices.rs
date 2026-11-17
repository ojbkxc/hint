//! 音频设备枚举。
//!
//! 输入与输出都通过 cpal 的默认 host（Windows 下即 WASAPI）枚举，
//! 返回的 `id` 是 cpal `DeviceId` 的完整字符串（形如
//! `wasapi:{端点ID}`）。`AudioConfig::mic_device` / `output_device`
//! 直接填写该字符串即可；系统声采集在内部会自动去掉 `wasapi:` 前缀，
//! 把原始端点 ID 交给 WASAPI 的 `DeviceEnumerator::get_device`。
//!
//! 非 Windows 平台同样可以枚举设备，但 [`crate::AudioEngine::start`]
//! 会返回 `Unsupported`。

use crate::DeviceInfo;
use crate::error::{AudioError, Result};
use cpal::traits::{DeviceTrait, HostTrait};

/// 列出所有输入设备（麦克风）。
pub fn list_input_devices() -> Result<Vec<DeviceInfo>> {
    enumerate(true)
}

/// 列出所有输出设备（系统声 loopback 的候选源）。
pub fn list_output_devices() -> Result<Vec<DeviceInfo>> {
    enumerate(false)
}

fn enumerate(input: bool) -> Result<Vec<DeviceInfo>> {
    let host = cpal::default_host();
    let default_id = if input {
        host.default_input_device()
            .and_then(|device| device.id().ok())
            .map(|id| id.to_string())
    } else {
        host.default_output_device()
            .and_then(|device| device.id().ok())
            .map(|id| id.to_string())
    };

    let devices = if input {
        host.input_devices()
            .map_err(|e| AudioError::Backend(format!("枚举输入设备失败：{e}")))?
    } else {
        host.output_devices()
            .map_err(|e| AudioError::Backend(format!("枚举输出设备失败：{e}")))?
    };

    let mut result = Vec::new();
    for (index, device) in devices.enumerate() {
        let id = match device.id() {
            Ok(id) => id.to_string(),
            Err(error) => {
                tracing::debug!("跳过无法获取 ID 的音频设备（序号 {index}）：{error}");
                continue;
            }
        };
        let name = device
            .description()
            .map(|description| description.name().to_string())
            .unwrap_or_else(|_| format!("音频设备 {index}"));
        let is_default = default_id.as_deref() == Some(id.as_str());
        result.push(DeviceInfo {
            id,
            name,
            is_default,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手动冒烟：验证本机设备枚举与 ID 格式。
    ///
    /// 默认忽略（CI 无音频设备）：
    /// `cargo test -p hint-audio -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实音频设备，手动运行"]
    fn smoke_list_devices() {
        let inputs = list_input_devices().expect("枚举输入设备失败");
        let outputs = list_output_devices().expect("枚举输出设备失败");
        println!("输入设备：{inputs:#?}");
        println!("输出设备：{outputs:#?}");
        assert!(inputs.iter().all(|device| !device.id.is_empty()));
        assert!(outputs.iter().all(|device| !device.id.is_empty()));
    }
}
