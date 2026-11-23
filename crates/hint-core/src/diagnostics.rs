//! `--doctor` 报告里可测试的纯逻辑。
//!
//! 这些函数原来长在 `src-tauri/src/doctor.rs` 里，但壳层 crate 的测试
//! harness 因 `sherpa-onnx-sys` 静态库无法链接（`cargo test -p hint` 用不了），
//! 等于写完就没人守。下沉到这里，CI 的 `cargo test -p hint-core` 就能覆盖。
//!
//! 只放**不依赖 Tauri、不做 IO 之外副作用**的解析与格式化；真正的信息采集
//! 仍在壳层（要读 app_data 路径、要走 keyring）。

/// 从日志尾部挑出 WARN / ERROR 行。
///
/// - 只在最后 `max_lines` 行里找，避免整份日志扫描；
/// - 单行截断到 160 字符（踩过用户把整段堆栈贴进报告的坑）；
/// - 最多返回 `LIMIT` 条，取**最后**那批——最新的错误最有诊断价值。
pub fn tail_warnings(text: &str, max_lines: usize) -> Vec<String> {
    /// 报告里最多保留多少条告警。
    const LIMIT: usize = 15;
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    let mut picked: Vec<String> = lines[start..]
        .iter()
        .filter(|line| is_warn_or_error(line))
        .map(|line| line.trim().chars().take(160).collect::<String>())
        .collect();
    if picked.len() > LIMIT {
        picked = picked.split_off(picked.len() - LIMIT);
    }
    picked
}

/// 判定一行是否 WARN / ERROR 级别。
///
/// tracing 的默认格式是 `<timestamp>  <LEVEL> <message>`，时间戳本身不含空格，
/// 所以**第二个空白分隔的 token 就是级别**。不能用 `line.contains(" WARN ")`
/// 这类子串判断：`INFO WARN 出现在正文里` 这种「正文以 WARN 开头」的 INFO 行
/// 会被误命中（单测 `tail_warnings_picks_only_real_level_lines` 抓到过）。
pub fn is_warn_or_error(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    // 第一个是时间戳；第二个才是级别。
    let _timestamp = tokens.next();
    matches!(tokens.next(), Some("WARN") | Some("ERROR"))
}

/// 配置里的设备 id → 报告里的可读文本。
///
/// 不打印完整 cpal 端点 ID（形如 `wasapi:{0.0.0.00000000}.{长 GUID}`），
/// 那串东西对用户没意义、还会把报告撑长；只要说明「用了默认」还是「指定了」。
pub fn device_label(configured: &str, fallback: &str) -> String {
    if configured.trim().is_empty() {
        fallback.to_string()
    } else {
        "已指定（具体设备可在设置页查看）".to_string()
    }
}

/// 字节数 → `x.y MB`，报告里统一起见用这个。
pub fn format_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
2026-11-23T19:35:44.075415Z  INFO 日志已写入文件 path=hint.log
2026-11-23T19:35:44.075415Z  WARN 音频引擎错误 麦克风采集中断
2026-11-23T19:35:45.000000Z  INFO WARN 出现在正文里的一行
2026-11-23T19:35:46.000000Z ERROR 语音资产准备失败
2026-11-23T19:35:47.000000Z  INFO 正常行
";

    #[test]
    fn tail_warnings_picks_only_real_level_lines() {
        let picked = tail_warnings(LOG, 100);
        assert_eq!(picked.len(), 2, "只应命中 WARN 与 ERROR 各一行: {picked:?}");
        assert!(picked[0].contains("麦克风采集中断"));
        assert!(picked[1].contains("语音资产准备失败"));
        // 正文里出现 WARN 字样的 INFO 行不能误报。
        assert!(
            !picked.iter().any(|line| line.contains("出现在正文里")),
            "INFO 行不应入选"
        );
    }

    #[test]
    fn tail_warnings_respects_window_and_limit() {
        // 只有最后 max_lines 行参与筛选。
        let mut text = String::new();
        for index in 0..50 {
            text.push_str(&format!("2026-01-01T00:00:00Z ERROR 旧错误 {index}\n"));
        }
        text.push_str("2026-01-01T00:00:00Z  INFO 窗口外\n");
        let only_recent = tail_warnings(&text, 3);
        assert!(only_recent.len() <= 15, "条数上限");
        // 最后 3 行只有 1 条 INFO + 2 条 ERROR（其中一条被截断窗口切掉）。
        assert!(only_recent.iter().all(|line| line.contains("ERROR")));

        // 上限 15 条：给 40 条 ERROR，只留最后 15 条。
        let mut many = String::new();
        for index in 0..40 {
            many.push_str(&format!("2026-01-01T00:00:00Z ERROR 错误 {index}\n"));
        }
        let capped = tail_warnings(&many, 1000);
        assert_eq!(capped.len(), 15);
        assert!(
            capped.last().expect("非空").contains("错误 39"),
            "保留最新的"
        );
        assert!(
            !capped.iter().any(|line| line.contains("错误 24")),
            "更早的被丢弃"
        );
    }

    #[test]
    fn tail_warnings_truncates_long_lines() {
        let long = format!("2026-01-01T00:00:00Z ERROR {}\n", "字".repeat(500));
        let picked = tail_warnings(&long, 10);
        assert_eq!(picked.len(), 1);
        // 160 字符上限（含时间戳与级别前缀）。
        assert_eq!(picked[0].chars().count(), 160);
    }

    #[test]
    fn tail_warnings_handles_empty_and_no_match() {
        assert!(tail_warnings("", 100).is_empty());
        assert!(tail_warnings("2026-01-01T00:00:00Z  INFO 只有正常行\n", 100).is_empty());
    }

    #[test]
    fn device_label_reports_only_presence() {
        assert_eq!(device_label("", "系统默认"), "系统默认");
        assert_eq!(
            device_label("   ", "系统默认"),
            "系统默认",
            "空白视同未配置"
        );
        let specified = device_label("wasapi:{0.0.0.00000000}.{abc}", "系统默认");
        assert!(specified.contains("已指定"));
        // 端点 ID 本身不能出现在报告里。
        assert!(!specified.contains("wasapi"), "不得回显设备 id");
    }

    #[test]
    fn format_mb_rounds_to_one_decimal() {
        assert_eq!(format_mb(0), "0.0 MB");
        assert_eq!(format_mb(1_048_576), "1.0 MB");
        assert_eq!(format_mb(241_357_257), "230.2 MB");
    }
}
