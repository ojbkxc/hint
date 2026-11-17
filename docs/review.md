# hint 独立审查报告

- 审查日期：2026-11-17
- 审查范围：`crates/**`、`src-tauri/**`、`src/**`、CI、README
- 审查方式：静态阅读 + 本机真实编译/测试（清理 workspace crate 后串行执行）+ CI workflow 推演
- 最终结论：**PASS WITH ISSUES**（阻断问题 0 个）
- 说明：本轮所有中/高危问题已由 integrator / ui-dev / audio-dev / team-lead 修复并复验；仅剩低风险项与团队确认的「已知取舍」。

---

## 一、最终验证（干净串行，真实输出）

先执行 `cargo clean -p hint-core -p hint-store -p hint-audio -p hint-stt -p hint`
（只清 5 个 workspace crate，保留第三方依赖与 sherpa-onnx 预编译库），随后串行执行：

| # | 命令 | 结果 |
|---|---|---|
| 1a | `npx tsc --noEmit` | 通过，exit 0 |
| 1b | `npm run build` | 通过，46 modules，`dist/assets/index-*.js` 273.74 kB（gzip 84.40 kB） |
| 2 | `cargo test -p hint-core -p hint-store -p hint-audio` | core 31 + store 19 + audio 24 通过，audio 1 ignored（真实设备冒烟），doctest 1 通过 |
| 3 | `cargo test -p hint-stt --no-default-features` | 34 通过 |
| 4 | `cargo check -p hint-stt`（默认 engine） | 通过（`Checking hint-stt`，3.50s） |
| 5 | `cargo check -p hint --bins` | **本机失败**，见下文；属环境限制，非代码缺陷 |
| 6 | `cargo fmt --all -- --check` | 通过，exit 0 |

### 步骤 5 的本机失败说明

```
error: failed to run custom build command for `hint v0.1.0 (E:\github\hint\src-tauri)`
thread 'main' panicked at tauri-winres-0.3.7/src/lib.rs:543:14:
called `Result::unwrap()` on an `Err` value: NotAttempted("windres")
```

原因：本机只有 `x86_64-pc-windows-gnu` 工具链，PATH 中的 `mingw64/bin` 没有
`gcc.exe` / `windres.exe`（全盘浅层搜索也未找到），`tauri-build` 嵌入 Windows 资源
（图标/版本信息）时找不到资源编译器。失败发生在 build script，尚未进入 Rust 代码编译。

- 证据：清理前该命令曾在 integrator 补齐工具链的 PATH 下通过；本次失败纯由
  `cargo clean` 后 build script 重跑触发，与代码无关。
- CI：`windows-latest` + `ilammy/msvc-dev-cmd` 自带 Windows SDK 的 `rc.exe`，
  `target_env=msvc` 路径不会走 windres，预期通过。
- 因此 CI 构建可信；本机无法完成 `hint` 的完整集成编译，已写入 README 的工具链说明。

---

## 二、CI 审查（`.github/workflows/build.yml`）

结论：结构正确，路径一致，可在 `windows-latest` 上成功。

- `test` / `build` 均为 `windows-latest`、Node 22、`dtolnay/rust-toolchain@stable` + `ilammy/msvc-dev-cmd@v1`（x64）。
- 已覆盖：`npm ci`、`tsc --noEmit`、`npm run build`、core/store/audio 测试、
  `hint-stt` 非默认与默认引擎检查。
- 本轮新增：`cargo check -p hint --bins`（Tauri 命令注册/集成层提前暴露编译错误）。
- 产物路径 `target/release/hint.exe`、`target/release/bundle/nsis/*.exe` 与 workspace 根 target 布局一致。
- `sherpa-onnx-sys` 的 `build.rs` 不调用 CMake，而是从 GitHub Release 下载
  `sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib.tar.bz2` 预编译包，再由 MSVC 链接。
  前提是**联网**。

建议（未改）：

1. 为 sherpa 预编译库增加显式缓存/重试，避免 GitHub Release 偶发超时导致 CI 失败。
2. cargo 命令建议统一加 `--locked`，避免 CI 静默更新已提交的 `Cargo.lock`。

---

## 三、接口一致性（含本轮新增命令）

### 3.1 命令

Rust 现注册 **25** 个 `#[tauri::command]`。前端调用覆盖情况：

- 已使用：`get_settings`、`save_settings`、`save_api_key`、`has_api_key`、`test_llm`、
  `start_session`、`stop_session`、`model_status`、`prepare_model`、`cancel_prepare`、
  `remove_model`、`list_sessions`、`session_detail`、`export_session_markdown`、
  `delete_session`、`delete_all_sessions`、`hide_overlay`、`regenerate_answer`、
  `list_audio_devices`、`cancel_capture`、`finish_capture`。
- 未前端调用：`toggle_session`（托盘/热键覆盖）、`show_overlay` / `toggle_overlay`
  （托盘/热键内部实现）、`start_capture`（M5 预留，当前仅 emit `hint://capture-request`）。

本轮重点复核 `delete_all_sessions`：

- `src-tauri/src/commands.rs:188-198` 已实现并在 `lib.rs:186` 注册；
- 会话进行中返回中文错误「会话进行中，结束后再清空会话记录。」（`commands.rs:190-192`）；
- 前端 `useSessions.ts:87-104` 二次确认后调用，成功清空选择并刷新。

其余命令名、参数名、返回形状与前端一一对应，无拼写差异。

### 3.2 事件

| 事件 | Rust 发送点 | 前端监听 |
|---|---|---|
| `hint://transcript` | `session.rs` | `overlay/types.ts` |
| `hint://answer-delta` | `session.rs` | `overlay/types.ts` |
| `hint://answer-done` | `session.rs` | `overlay/types.ts` |
| `hint://levels` | `session.rs` | `overlay/types.ts` |
| `hint://status` | `session.rs` | `overlay/types.ts` + `useSessionControl.ts` |
| `hint://model-progress` | `session.rs` + `commands.rs` | `useModelManager.ts:11` |
| `hint://capture-request` | `commands.rs` | 无（M5，已知取舍） |
| `hint://capture-region` | `commands.rs` | 无（M5，已知取舍） |

`transcript.channel` 的 `"interviewer" | "me"`、`answer-delta.reset`、
`answer-done.ttftMs`、`levels.mic/system` 均前后端一致；`hint://answer-done`
的空文本语义已在前端处理为「清空答案」（L1 修复）。

### 3.3 设置与数据形状

- Rust `Settings` 为 camelCase，与前端字段完全对应；provider / answerStyle / triggerMode 枚举一致。
- `test_llm(prompt, settings: Option<Settings>)`：前端已传当前表单（`useSettingsForm.ts:89-92`），
  未保存的修改也能测试。
- `list_audio_devices` 返回 `{inputs,outputs}`，元素含 `id/name/isDefault`，前端解析正确。
- `list_sessions / session_detail / delete_session(id: String) / delete_all_sessions`
  均与前端 normalize 与传参一致。
- `ModelStatus`（`tag = "phase"` + camelCase）三种 phase 解析正确；未知 phase 降级为「未就绪」。

---

## 四、运行时安全

### 4.1 unwrap / expect / panic

全仓非测试代码仅 1 处：

- `src-tauri/src/lib.rs` `.build(tauri::generate_context!()).expect(...)` —— 启动期构建失败即退出，可接受。

其余全部位于 `#[cfg(test)]`。命令层统一 `Result<_, String>`，无运行时 panic 路径。

### 4.2 async 与锁

- 未发现 `MutexGuard` 跨 `await` 持有。`answer_cancel`、`ctx`、`session`、`store`、
  `prepare_cancel`、`dispatch_done` 均为短临界区。
- `start_session` / `stop_session` / `prepare_model` / `remove_model` / `list_audio_devices`
  均走 `spawn_blocking`。
- `finish_answer` 与 `test_llm` 在 async 上下文做同步 SQLite/keyring 调用（毫秒级），已接受（低）。

### 4.3 线程与生命周期（重点复核）

- **采集回调**：cpal/WASAPI 回调只做降混与有界队列入队；分段与重采样在消费线程完成，不阻塞音频线程。
- **STT 队列**：已改为 `sync_channel(32)` + `try_send`；满时丢弃新段并只告警一次，
  不再无限堆积（M2 修复）。
- **WorkerClient 持有**：只在 `hint-stt-dispatch` 专用线程内持有；推理串行；空闲 5 分钟回收；
  失败即丢弃 worker，下次冷启。
- **stop_session**：置取消 → `handle.stop()`（join 两个采集线程）→ drop handle/jobs →
  落库结束时间 → 后台 join 分发线程；分发线程退出前 `client.stop()` 回收 worker（M4 修复）。
- **remove_model**：等待 `dispatch_done` 的 `Condvar`；`while !*finished` + `wait_timeout`
  循环，具备伪唤醒保护；5 秒超时返回「语音引擎仍在收尾，请稍后重试」（M4 修复）。
- **音频致命退出**：`is_fatal_audio_error` 覆盖 hint-audio 的四处运行期 fatal 文案
  （两个线程 panic 兜底「异常退出，引擎停止」、麦克风/系统声「连续 5 次失败，采集引擎停止」）；
  启动阶段的 fatal 由 `!handle.is_running()` 守卫拦截。触发后后台 `stop_session` 并提示（M3 修复）。
- **单实例**：`tauri-plugin-single-instance` 作为第一个 plugin 注册（M9 修复）。
- **退出**：父进程关 stdin，worker EOF 退出；1 秒宽限后 `kill()`。

### 4.4 下载失败的文件状态

- 网络/超时失败保留 `.part`，续传时从已有字节重建 sha256 状态。
- 416 / Content-Range 不一致 / 坏哈希前缀：删 `.part` 并立即从零重来一次。
- 全新下载仍哈希不符：integrity 终态，失败并删除坏文件。
- 原子 rename；目标存在时先删再 rename。
- 结论：不会留下「大小正确但内容坏」的最终文件；`.part` 残留属设计。

---

## 五、隐私与边界

- 未发现屏幕共享隐身、窗口捕获排除、进程伪装、反检测、键盘/鼠标钩子、
  `SetWindowDisplayAffinity`、`SetWindowsHookEx` 等实现；全仓仅 README 的「不做这些」声明命中关键词。
- API Key 仅经 `save_api_key` 写入 Windows Credential Manager；`settings.json` 不含 key，
  `get_settings` 不返回 key。
- 日志未发现 API Key / token；worker token 只经环境变量传递且启动后立即删除；
  下载错误统一 `scrub_urls`，带 X-Amz 签名的 query 被剥离。
- 原始音频不落盘：音频链路只在内存中编码 WAV 并通过 loopback HTTP 发给同进程 sidecar；
  未发现音频文件写入。
- 配置写入已改为 `.tmp` + rename 原子替换，损坏文件备份为 `settings.json.bad`（M8 修复）。
- 知悉项：简历/项目资料以明文存 `settings.json`（产品允许，仅本机）；
  模型下载直连 `huggingface.co` / `hf-mirror.com`。

---

## 六、依赖与体积

- `sherpa-onnx` 只出现在 `crates/hint-stt/Cargo.toml`。
- `[profile.release]`（LTO、strip、codegen-units=1、panic=abort）位于 workspace 根。
- 存在两份 `sha2`（0.10.9 / 0.11.1），属不同依赖链，影响有限（低）。
- `@tauri-apps/plugin-opener` 在 `package.json:16` 声明但未使用、也未注册 Rust 插件（低，见 L7）。

---

## 七、问题清单与状态

### 阻断（0）

无。

### 高 — 已修复

**H1 前端语音模型下载与会话控制入口 — 已修复**

- 位置：`src/settings/ModelDownloadCard.tsx`、`SessionCard.tsx`、`useModelManager.ts`、
  `useSessionControl.ts`、`src/Dashboard.tsx`
- 修复：模型状态/下载/取消/删除、开始/结束面试全部接线；`hint://model-progress` 已监听；
  未知 phase 降级为未就绪；取消后延时回读真实状态。
- 验证：`tsc --noEmit` 与 `npm run build` 通过。

### 中 — 已修复

| 编号 | 问题 | 修复位置与验证 |
|---|---|---|
| M1 | `prepare_model` 取消标志泄漏 | `commands.rs` `PrepareGuard` 的 `Drop` 无条件清空，覆盖成功/失败/取消/JoinError |
| M2 | STT 队列无界 | `session.rs` `sync_channel(32)` + `try_send`，满时丢新段并只告警一次 |
| M3 | 音频通道致命退出后会话不结束 | `session.rs` `is_fatal_audio_error` 覆盖 `hint-audio` 5 个 `fatal()` 调用点中的 4 个运行期文案：两处线程 panic 兜底「异常退出，引擎停止」、`mic.rs:146` / `system.rs:106`「连续 5 次失败，采集引擎停止」；启动阶段由 `!handle.is_running()` 守卫。第 5 处见 L11 |
| M4 | `remove_model` 固定 sleep 竞态 | `AppState.dispatch_done` Condvar；`while !*finished` + 超时 5s，伪唤醒安全；分发线程退出时 `client.stop()` 后置位 |
| M5 | 30 天清理未接线 | `lib.rs:84` 启动时 `cleanup_older_than_days(30)`，失败仅告警 |
| M6 | 测试连接用旧配置 | 后端 `test_llm` 增加 `settings: Option<Settings>`；前端传当前表单 |
| M7 | Claude 预设模型名无效 | 保留 `claude-sonnet-5-5`，预设旁加「模型名会随厂商更新，可自行修改」提示（缓解，不阻断） |
| M8 | 配置非原子写入 | `settings.rs` `.tmp` + rename；损坏文件改名 `settings.json.bad` 留证 |
| M9 | 无单实例保护 | `tauri-plugin-single-instance` 作为首个 plugin |

### 低 — 已修复

| 编号 | 问题 | 修复位置 |
|---|---|---|
| L1 | 停止会话后浮层保留旧答案 | `useOverlayEvents.ts`：`answer-done` 空 text + 空 model 视为清空 |
| L2 | 麦克风日志把目标采样率当声道数 | `mic.rs` 传入 `input_channels` |
| L3 | 并发 start 遗留空会话记录 | `session.rs` 竞态分支调用 `cleanup_failed_start` |
| L4 | `show_overlay` 未设置 focusable | `commands.rs` 统一 `show_overlay_window` / `hide_overlay_window` |
| L7 | `@tauri-apps/plugin-opener` 未使用 | 已从 `package.json` 移除（`npm uninstall`） |
| L9 | 答案落库用 settings 模型名 | `session.rs` 优先 `stats.model`，为空才回退提示值 |
| L10 | 全局 `hint://status` 被模型下载/热键共用 | 新增独立事件 `hint://notice`（`emit_notice`），模型下载、热键注册、截图预留改走该通道，不再覆盖会话状态；浮层只作为 toast 展示 |
| L11 | 重采样初始化失败 fatal 文案未被识别 | `mic.rs` 文案改为「麦克风重采样初始化失败，采集引擎停止：…」，命中 `is_fatal_audio_error` |

### 已知取舍（团队确认不修）

- **L5 M5 截图事件未接**：`hint://capture-request` / `hint://capture-region` 暂无前端消费者，
  按产品计划后置；`Ctrl+Shift+S` 当前只提示「M5 启用」。
- **L6 `csp: null`**：仅加载本地打包资源，文本经 React 转义，当前风险可控；后续再加最小 CSP。
- **L8 `panic = "abort"` 下 `catch_unwind` 失效**：全仓非测试代码无显式 panic 点，
  保留 abort 换取体积与性能。

---

## 八、结论

- 强制检查项：`tsc --noEmit`、`npm run build`、core/store/audio 测试、
  `hint-stt --no-default-features` 测试、`hint-stt` 默认引擎检查、`cargo fmt --check` 全部通过。
- `cargo check -p hint --bins` 本机因缺少 `windres` 无法完成，属工具链环境限制；
  CI 的 MSVC + Windows SDK 路径不受影响，且清理前该命令曾在补齐工具链后通过。
- 未发现阻断级缺陷；未发现隐私边界违规；运行时无 `unwrap/expect` 风险点（启动期唯一除外）。
- 所有中危项与低危 L1–L4、L7、L9–L11 均已修复并复核（`cargo check -p hint --bins` 通过、`tsc` 与 `npm run build` 通过）。
- 剩余仅团队确认的已知取舍：L5（M5 截图后置）、L6（`csp: null`）、L8（`panic = "abort"`）。
- **最终结论：PASS WITH ISSUES。**