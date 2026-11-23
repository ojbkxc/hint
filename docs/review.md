# hint 独立审查报告

- 审查日期：2026-11-17（首发前复验与版本更新：2026-11-18）
- 审查范围：`crates/**`、`src-tauri/**`、`src/**`、CI、README
- 审查方式：静态阅读 + 本机真实编译/测试（清理 workspace crate 后串行执行）+ CI workflow 推演
- 最终结论：**PASS WITH ISSUES**（阻断问题 0 个）
- 说明：本轮所有中/高危问题已由 integrator / ui-dev / audio-dev / team-lead 修复并复验；仅剩低风险项与团队确认的「已知取舍」。
- 版本：首个正式发布版为 **1.0.1**（tag `v1.0.1`）；下文 2026-11-17 的记录中出现的 `v0.1.0` 仅为当时版本号，现版本号与所有清单均已同步为 1.0.1。

---

## 一、最终验证（干净串行，真实输出）

先执行 `cargo clean -p hint-core -p hint-store -p hint-audio -p hint-stt -p hint`
（只清 5 个 workspace crate，保留第三方依赖与 sherpa-onnx 预编译库），随后串行执行：

| # | 命令 | 结果 |
|---|---|---|
| 1a | `npx tsc --noEmit` | 通过，exit 0 |
| 1b | `npm run build` | 通过，`dist/assets/index-*.js` 281.68 kB（gzip 86.82 kB），CSS 14.39 kB |
| 2 | `cargo test -p hint-core -p hint-store -p hint-audio` | core 61 + store 24 + audio 25 通过，audio 1 ignored（真实设备冒烟），doctest 1 通过 |
| 3 | `cargo test -p hint-stt --no-default-features` | 38 通过 |
| 4 | `cargo check -p hint-stt`（默认 engine） | 通过（`Checking hint-stt`，3.50s） |
| 5 | `cargo check -p hint --bins` | **本机失败**，见下文；属环境限制，非代码缺陷 |
| 6 | `cargo fmt --all -- --check` | 通过，exit 0 |

### 步骤 5 的本机失败说明

```
error: failed to run custom build command for `hint v0.1.0 (E:\github\hint\src-tauri)`
thread 'main' panicked at tauri-winres-0.3.7/src/lib.rs:543:14:
called `Result::unwrap()` on an `Err` value: NotAttempted("windres")
```

（以上为 2026-11-17 原始输出；当时版本号为 0.1.0，现为 1.0.1，报错内容与本条结论无关。）

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

建议（未改 / 后续处理）：

1. 为 sherpa 预编译库增加显式缓存/重试，避免 GitHub Release 偶发超时导致 CI 失败。
2. cargo 命令建议统一加 `--locked`，避免 CI 静默更新已提交的 `Cargo.lock`。—— **2026-11-18 已处理**：`build.yml` 的全部 `cargo check` / `cargo test` 已加 `--locked`。
3. **2026-11-18 新增**：`.github/workflows/release.yml`，推送 `v*` 标签或手动 `workflow_dispatch` 时构建并上传 `hint.exe` 与 NSIS 安装包到 GitHub Release（glob 匹配 `target/release/bundle/nsis/*.exe`，不硬编码文件名）。

---

## 三、接口一致性（含本轮新增命令）

### 3.1 命令

Rust 现注册 **26** 个 `#[tauri::command]`。前端调用覆盖情况：

- 已使用：`get_settings`、`save_settings`、`save_api_key`、`has_api_key`、`test_llm`、
  `start_session`、`stop_session`、`model_status`、`prepare_model`、`cancel_prepare`、
  `remove_model`、`list_sessions`、`session_detail`、`export_session_markdown`、
  `delete_session`、`delete_all_sessions`、`hide_overlay`、`regenerate_answer`、
  `list_audio_devices`、`cancel_capture`、`finish_capture`、`open_log_dir`（设置页「打开日志目录」）。
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
| `hint://queue` | `session.rs`（入队 / 出队 / 清空时实时推送 `{pending}`） | `useOverlayEvents.ts` |
| `hint://overlay-opacity` | `commands.rs`（`save_settings` 后推送 `{opacity}`，已 clamp 0.2–1.0） | `useOverlayEvents.ts`（写 CSS 变量 `--overlay-alpha`） |
| `hint://notice` | `session.rs`（模型下载、热键、截图、关闭窗口、LLM 失败） | `useOverlayEvents.ts` + `Dashboard.tsx` |
| `hint://capture-request` | `commands.rs` | 无（M5，已知取舍） |
| `hint://capture-region` | `commands.rs` | 无（M5，已知取舍） |

`transcript.channel` 的 `"interviewer" | "me"`、`answer-delta.reset`（1.0.1 起追加
`question` / `pending`）、`answer-done.ttftMs`（1.0.1 起失败时追加 `error` / `retryable`，
`text` 保留已生成内容）、`levels.mic/system`、`queue.pending` 均前后端一致；`hint://answer-done`
的空文本语义已在前端处理为「清空答案」（L1 修复）。LLM 生成失败与重新生成失败统一走
`hint://notice`（`level=error`），不再发 `status=error`，会话保持 `listening`。
`hint://queue` 在入队 / 出队 / 清空时实时推送，浮层「排队 N」在流式中也会立即更新；
托盘 tooltip 随会话状态切换（空闲 / 面试进行中），是主窗口关闭后确认会话是否在跑的主要手段。

### 3.3 设置与数据形状

- Rust `Settings` 为 camelCase，与前端字段完全对应；provider / answerStyle / triggerMode 枚举一致；`overlayOpacity`（f64，默认 0.88）前后端同名。
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
- `@tauri-apps/plugin-opener` 已从 `package.json` 移除（L7）。

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

---

## 九、1.0.1 首发前增量（2026-11-18）

本轮在 2026-11-17 报告之后又完成了一批使用者视角修复与两项产品行为调整，均已在真实代码中复核：

- **LLM 失败语义**：`CoreError` 新增 `is_retryable()` 与 `HttpStatus{status,advice,body}`、`Stream(String)` 变体；首字前可重试错误（网络、超时、429、5xx）自动重试一次（约 1.5 秒），已输出后断流报「生成中断」且不重试；401/403/404 不重试。错误文案全部中文化并附原始响应片段。
- **会话状态不受生成失败影响**：LLM 生成失败、重新生成失败、托盘 stop 失败统一改发 `hint://notice`（`level=error`），不再发 `status=error`，会话保持 `listening`。
- **答案队列（连续多问）**：`session.rs` `QuestionQueue` 为 FIFO，上限 5（另有 1 个在答）；去重只认「归一化后完全相同」或「去掉礼貌前缀/语气尾缀后相同」（与当前题、队尾比较），真实追问不会被吞；每轮作答时快照历史（包含上一轮答案）；新增 `hint://queue` 事件在入队/出队/清空时实时推送 `{pending}`；`Ctrl+Shift+A` 取消当前并清空排队、优先回答最新问题；停止会话清空队列。
- **托盘状态可见**：`TrayItems` 持有托盘图标，会话开始/结束时切换 tooltip（空闲「hint · 空闲（右键可退出；Ctrl+Shift+H 显示浮层）」/ 面试中「hint · 面试进行中（右键可结束或退出）」），解决关闭主窗口后无法确认会话是否在跑的问题。
- **浮层小窗口适配**：`src/App.css` 的两组 `@media (max-height: 240px)` / `@media (max-height: 200px)` 原先写在基础规则之前，被文件后面同优先级的 `.overlay-transcript { max-height: 42% }`、`.overlay-answer-body { padding: … }`、`.answer-current { min-height: 4.2em }` 覆盖，小窗口适配此前从未生效（320×180 实测答案区仅 38px，内容需 87px 被裁切）。已把媒体查询移到文件末尾，并新增 200px 断点隐藏转录区、去掉答案最小高度；改后实测 320×180 答案区 108px、无横向滚动，460×340 不受影响。验收清单 6.8 覆盖此场景。
- **主窗口窄宽适配**：同一根因，`@media (max-width: 860px)` 也原先位于基础规则之前，其中 `.review-layout`、`.dashboard-grid > .card:first-child`、`.save-card` 三条覆盖被后置同特异性规则失效（复盘页固定 300px/430px 双列、卡片跨列）。已把该媒体查询移到文件末尾；实测 780px 设置页与复盘页均为单列。验收清单 6.9 覆盖此场景。
- **问题去重收紧**：原「短句被长句包含且长度 ≥80%」的近似判定会把真实追问（「介绍一下你的项目」→「介绍一下你的项目难点」）当重复静默丢弃。现只认「归一化后完全相同」或「去掉礼貌前缀/语气尾缀后相同」，内容不同的追问一律排队回答。验收清单 6.5 / 6.5.1 覆盖此场景。
- **浮层不透明度可配置（底板直映射 + 文字区独立衬底）**：`Settings` 新增 `overlayOpacity`（camelCase，默认 0.88，旧配置缺字段时取默认值）；`save_settings` 保存后经 `hint://overlay-opacity` 推送已 clamp 到 0.2–1.0 的值，前端写入 CSS 变量 `--overlay-alpha`（缺变量时回退 0.88），保存即生效、无需重启；启动时读取已保存值并同样 clamp。`src/App.css`：`.overlay-root` 背景 `rgba(13,16,22, var(--overlay-alpha, 0.88))`——滑块值直接等于底板背景 alpha（20% 即 0.2，100% 即完全不透明），附 `backdrop-filter: blur(10px)`；转录区与答案区 `rgba(10,13,18, max(0.86, 0.4 + 0.5 × var))`（不低于 0.86）；头/底栏 `max(0.88, 0.45 + 0.5 × var)`；全窗 `text-shadow`，答案正文近白 `#f4f8fd` + 双层阴影。从不使用整窗 `opacity`，文字本身永远不透明。实测 460×340、纯白 + 亮黄 45° 条纹背景：滑块 20% 底板 `rgba(13,16,22,0.2)`、文字区 0.86；50% 底板 `rgba(13,16,22,0.5)`、文字区 0.86；100% 底板 `rgb(13,16,22)` 完全不透明、文字区 0.87；变量缺失回退 `rgba(13,16,22,0.88)`。该设置只改外观，不改变窗口可见性。验收清单 7.3 覆盖此场景。**设计说明**：头部与底栏的衬底（下限 0.88）刻意比正文区（0.86）更实，用于保证状态文字在任意画面上可读，非缺陷。（1.0.2 已改为浅色皮肤 + 整个浮层统一跟随滑块，见第十节。）
- **答案时间线**：浮层答案区最新一轮在最上方，更早问答按时间倒序排在下方（最多 10 轮），`answer-delta` 的 reset 帧新增 `question` / `pending`。
- **日志落盘**：新增 `src-tauri/src/logging.rs`，stderr + `%APPDATA%\com.ojbkxc.hint\logs\hint.log` 双写，>5MB 轮转为 `hint.1.log`；新增命令 `open_log_dir`（已在 `invoke_handler` 注册，设置页已接线）。
- **音频健壮性**：单通道降级明确提示「仅麦克风可用 / 仅系统声可用」；切换默认输出设备有提示；短暂切设备/蓝牙重连有 15 秒宽限；模型损坏提示「删除后重新下载」；磁盘不足中文提示。
- **提示词重写**：防穿帮、多问题按序回答、150/250 字自适应、防注入、语言跟随、追问不重复。
- **配置/资料边界**：简历与项目资料各 6000 字上限，超出截断并标注。
- **版本与发布**：版本号统一为 1.0.1；新增 `release.yml`（tag `v*` 触发）。

本轮复核命令：`npx tsc --noEmit`（通过）、`npm run build`（通过）、`cargo test -p hint-core -p hint-store -p hint-audio`（61 + 24 + 25，audio 1 ignored，doctest 1 通过）、`cargo test -p hint-stt --no-default-features`（38 通过）、`cargo fmt --all -- --check`（通过）。`cargo check -p hint --bins` 仍受本机缺 `windres` 限制，以 CI 为准。

## 十、1.0.2 增量（2026-11-18）

按使用者反馈完成的五项改动，Rust 侧验证全部走 GitHub Actions：

- **浅色默认皮肤**：`src/App.css` 重写为 pi-agent-desktop 式的语义化配色变量（`--bg` / `--surface` / `--text` / `--border` / `--accent` / 状态色 / 阴影），默认浅色：主窗口浅灰底 + 白卡片 + 深色文字，输入框白底，主按钮品牌蓝渐变。组件规则不再硬编码深色值，后续加深色主题只需覆盖 `:root` 变量。浮层同步改为浅色半透明（白底 + 深色文字 + 白色光晕阴影）。
- **浮层透明度「没生效」的根因修复**：原实现中转录区、答案区、头尾栏各有一层 `max(0.86/0.88, …)` 的固定深色衬底，浮层的可见面积几乎被这些衬底覆盖，滑块只影响缝隙，肉眼看不出变化。现改为整块浮层统一使用 `rgba(255,255,255, var(--overlay-alpha, 0.88))`，内容区全部透明跟随；文字可读性由深色字 + 白色 `text-shadow` 保证。同时移除 `.overlay-root` 的 `backdrop-filter`（透明窗口上它采样不到桌面内容，且在部分 WebView2 版本上会触发合成异常）。实测 460×340、彩色条纹背景：20% 时整个浮层几乎全透、文字可读；50% 半透；100% 纯白不透明。
- **资料导入 TXT / MD / DOCX / PDF**：新增 `tauri-plugin-dialog`（Rust + npm，capabilities 加 `dialog:default`）与命令 `import_document_text(path)`。纯解析逻辑放 `hint-core::import`（可单测）：UTF-8（含 BOM）优先、失败回退 GBK、再失败有损 UTF-8；DOCX 用 `zip` 读取 `word/document.xml` 并抽取 `<w:t>` 文本（保留段落、解码 XML 实体）；PDF 用 `pdf-extract` 提取文本层。壳层负责文件校验（单文件 20 MB 上限、拒绝 .doc 并提示另存、扫描版 PDF/空文件给出中文提示）与 20 万字符上限；前端 `ProfileCard` 两个输入框各有「从文件导入」按钮，导入后按 6000 字截断并提示原文长度。新增 4 项 core 单测覆盖编码、DOCX 段落/实体/未闭合标签、文本清理。
- **Base URL 智能补全与协议识别**：前端 `normalizeBaseUrl`（失焦时补 `https://`、剥掉完整端点 `/chat/completions` `/messages` `:generateContent`、OpenAI 无版本段时补 `/v1`）与 `detectProvider`（anthropic.com / generativelanguage.googleapis.com / openai.com / deepseek.com / dashscope / bigmodel / moonshot / siliconflow / localhost → 对应协议）。后端 `llm.rs` 新增 `openai_chat_url`：完整端点原样使用、任意 `vN` 版本段只拼 `/chat/completions`、裸域名补 `/v1`——修复了用户把 Base URL 填成 `https://tyapi.lxseek.com/v1/chat/completions` 时被拼成 `…/chat/completions/chat/completions` 的 404。新增 1 项 URL 单测覆盖 7 种形态。
- **测试连接使用未保存的 Key**：`test_llm` 新增可选参数 `apiKey`，表单里刚输入、尚未保存的 Key 直接用于测试；都没有时返回中文提示而不是 keyring 的 `No matching credential found`。前端在测试成功后自动保存该 Key（设置页「保存配置」也会一并保存输入框里的 Key），解决「填了 Key 直接点测试连接却报读取凭据失败」的困惑。
- **版本与发布**：版本号统一为 1.0.2；`Cargo.lock` 同步新增依赖（`tauri-plugin-dialog` 2.8.4、`pdf-extract` 0.9.0、`zip` 2.6.1、`encoding_rs` 0.8）。按用户要求，Rust 编译与测试不在本地执行，全部以 CI 结果为准。

## 十一、1.0.2 后增量（2026-11-18）

按使用者反馈完成的两项设置页交互改动，Rust 侧验证走 GitHub Actions：

- **模型名从接口下拉**：后端 `hint-core::llm` 新增 `fetch_models`（三协议各自的 `/models` 端点 + 鉴权头：OpenAI `GET {base}/models` 取 `data[].id`、Anthropic `GET {base}/v1/models` 取 `data[].id`、Gemini `GET {base}/v1beta/models` 取 `models[].name` 并剥掉 `models/` 前缀），并新增 `openai_models_url` / `anthropic_models_url` / `gemini_models_url` 复用版本段去重逻辑，避免 `/v1/v1/models`。壳层新增 `fetch_models` 命令，与 `test_llm` 同理支持用表单里未保存的 Key。前端 `useSettingsForm` 增加 `models` 状态与 `loadModels`，`ModelCard` 的模型名输入框改为 `input list` + `datalist`：点「刷新模型」拉取后可下拉选择，也支持手动输入列表外模型（兼容不支持 `/models` 的自建网关，空列表时提示手动填写）。新增 URL 单测覆盖三协议 7 种 base 形态。
- **去掉 API Key 旁边的冗余保存按钮**：`save()` 本就会把输入框里未保存的 Key 一并写入凭据管理器，`testConnection` 成功后也会自动保存，卡片内独立的「保存」按钮与底部「保存配置」职责重复。现移除 `ModelCard` 的 `onSaveKey` 按钮与 `useSettingsForm` 的 `saveKey`，`FormAction` 去掉 `"key"`、新增 `"models"`，Key 保存统一走底部「保存配置」，并在 Key 输入框下方加一行说明。

## 十二、语音链路深度修复（2026-11-19）

以真机日志（下载失败后日志仅 126 字节）与端到端验证（真实下载 241 MB 模型 + worker 识别 TTS 中文语音）为线索，修复以下缺陷：

- **资产清单实测有效**：三个文件的 URL、字节数、sha256 与清单逐字节一致；服务器返回 206 + `accept-ranges`，断点续传可用；worker 加载模型约 2.5 秒，5.57 秒中文语音识别 0.41 秒、结果与 TTS 原文一字不差。
- **STT 看门狗真正生效**：`WorkerClient::transcribe` 全程持 `state` 锁、最长 120 秒，dispatcher 阻塞其中时执行不到 stall 回收分支，且 `client.stop()` 同样被锁阻塞。现新增 `WorkerClient::abort()`（只写原子标志、不抢锁），`post_wav` 读循环改为 250ms 轮询并检查 abort（总超时仍 120 秒兜底）；**音频回调线程**检测到 stall 时直接 `abort()`，`transcribe` 被中断后按失败路径回收 worker，下次自动冷启。
- **补录与实时时间轴统一**：环形缓冲用音频时间轴（Segmenter 累计毫秒），dispatcher 原用自身 `Instant` 起点，两者错位会让 outage 的 `range()` 查不到音频。现统一用 `SttJob.end_ms` 驱动 `on_transcript` / `on_error`，删除 dispatcher 独立时钟。
- **补录不再饿死实时转录**：`run_recovery` 逐片段识别期间新到的实时段暂存（`parked`）并立即返回，下个空闲周期用 `resume_from_ms` 从断点继续；成功/失败/空结果都推进游标，避免同一片段反复重试。去重窗口从 `i64::MAX` 收敛到 `[start-5s, end+5s]`，空识别结果不进入 live 文本池。
- **下载失败可诊断**：补开始下载、每次尝试失败（含失败分类与源标签）、最终失败、磁盘预检失败、`prepare` 整体失败的日志；此前失败时日志只有启动一行。
- **磁盘不足文案**：不再写死「约 300MB」，按实际预算输出「可用约 X MB，本次还需要约 Y MB」，续传时不再误导。

## 十三、健壮性与提示完善（2026-11-19）

- **Gemini 模型列表**：`models_from_gemini` 只保留 `supportedGenerationMethods` 含 `generateContent` 的模型（Gemini 会返回 embedding / aqa 等不能对话的模型），字段缺失时保守保留；加单测。
- **设置页模型列表**：切换协议时清空上一接口的列表；`loadModels` 做数组类型防御与去重，异常响应不再让设置页报错。
- **取消下载不误报**：新增 `SttError::is_cancelled` 与共享 `CANCELLED_MESSAGE`，`prepare_model` 识别用户主动取消并给普通提示，不再显示「模型下载失败」。
- **错误提示中文化**：`user_message_for` 改用 `user_message()`（此前用英文 Display，会把已诊断的「模型文件损坏」显示成「未知错误」）；Engine / Worker / Config 补中文映射，并加单测。
- **文档**：README 补联合状态、噪声门、断线补录的说明与限制；验收清单新增 3.5 / 4.5 / 4.6 及模板行。

## 十四、会话生命周期竞态与意图收敛（2026-11-20）

本轮针对「快速连点开始/结束」「启动中再按热键」这类真实操作路径，修正了生命周期队列的两处竞态与三处语义问题：

- **决策与执行之间的窗口**：`submit_lifecycle` 原先在锁内只做 `request`，解锁后才 `begin`。两个线程同时请求 Start 时都可能拿到 `Run`，第二个 `start_session` 因「会话已在进行中」失败并把状态回写成 Idle，而会话其实在跑——之后 Stop 被判成 Noop，会话再也停不下来。现把 `begin` 放进 `request` 同一把锁内，从结构上保证同一时刻只有一个在途请求。
- **在途同向请求吞掉最后一次意图**：原合并规则「在途方向相同即合并」会在「启动中 → 用户按了 Stop（排队）→ 又按 Start」时把最后的 Start 静默吸收，队列执行完 Stop 后停在 Idle。现在排队槽只保留最后意图，新意图覆盖旧意图；目标已达成时直接跳过，最终落在最后一次意图上（第十五节进一步收敛为单槽）。
- **启动中切换热键**：`toggle_session` 原先按 `is_active()` 判断，启动中（Starting）未激活，再按热键仍判成 Start 并被合并，用户无法取消 10–30 秒的模型加载。现按生命周期状态取反：`Idle / Stopping → Start`，`Starting / Active → Stop`；第十五节把方向决策移进队列锁内（`request_toggle`），命令与热键共用。
- **音频致命中断的状态一致性**：`end_session_after_audio_failure` 原先直调 `stop_session`，绕过了生命周期队列，队列状态停在 Active，之后的热键/托盘切换会一直判成 Stop。现改走 `submit_lifecycle(Stop)`。
- **失败回写以真实会话为准**：`run_lifecycle` 失败时不再一律按方向回退，而是检查 `is_running`：Start 失败但会话在跑（重复启动）记 Active，Stop 失败但会话已收尾记 Idle，避免队列状态与真实会话长期不一致。
- **顺带清理**：`Segmenter::push` 返回本块 RMS 供断线看门狗复用，采集路径只算一次 `audio_metrics`；`test_llm` / `fetch_models` 的 Key 解析统一为 `resolve_api_key`。
- **识别错误提示中文化**：看门狗中止、引擎无响应、连接失败、启动超时/退出这几类 Worker 英文消息映射为中文短句，避免浮层出现「语音识别失败，稍后自动重试：inference aborted by the watchdog」。

另外修正了语音模型下载的并发窗口：`cancel_prepare` 原先直接把槽位 `take()` 掉，取消后立刻重新点下载会与尚未退出的旧任务并发写同一批 `.part` 文件；`PrepareGuard` 析构时又无条件清空槽位，可能抹掉新任务的取消标志。现在取消只置位、不腾槽，槽位由旧任务退出时清理，且 `PrepareGuard` 只清自己的标志，从结构上杜绝并发下载与「取消按钮失效」。`prepare_model` 同时改为返回 `bool`（true=完成，false=用户取消），前端据此区分文案——此前取消后设置页会显示「模型文件检查完成，可以开始面试」。下载进度里的 `source` 也修正为约定好的短标签 `official` / `mirror`（原先把源地址传了进去，设置页因此显示 `https://huggingface.co` 而不是「官方源」）。

## 十五、1.0.3 后全量复查（2026-11-20）

对 v1.0.2→v1.0.3 的全部改动做了一轮逐文件复查，修复了以下问题，并在 CI 上重新验证：

- **启动中的破坏性操作**：`remove_model` / `delete_session` / `delete_all_sessions` 原先只查 `state.session`；启动的 10–30 秒内会话尚未注册，删模型会与 worker 加载并发、清空记录会删掉正在创建的会话。新增 `session::is_busy`（会话存在或生命周期非 Idle）统一拦截。
- **停止瞬间仍继续流式**：`answer_one` 在检查 `cancelled` 之后才创建新的取消标志，若停止恰好发生在两步之间，`stop_session` 取消的是旧标志，本轮会继续请求并推 delta。现在创建标志后再查一次会话级 `cancelled`，delta 回调也同时看会话级标志。
- **切换请求的原子性**：`toggle_dir` + `submit_lifecycle` 是两次加锁，并发双击可能都读到同一状态。新增 `LifecycleQueue::request_toggle`，方向与 `begin` 在同一把锁内完成；命令层与 `Ctrl+Shift+R` 统一走 `submit_toggle` / `spawn_toggle`，删除 `toggle_dir`。
- **排队意图收敛**：排队槽从 `VecDeque` 改为单槽「最后意图优先」。连点热键不会积压几十次启动/停止，`drain_lifecycle` 也不会递归很深；中间意图本来就不需要真的执行。
- **下载重试分类**：`from_ureq` 把所有非 2xx 归为 Http（不可重试），408/429/5xx 不会在同源重试。现改为归入可重试的 Network，兑现「每源重试 5 次」。
- **补录短文本去重**：归一化后 <3 字符直接返回「未覆盖」，会让「好的」「收到」这类补录与实时结果重复上屏；现在先做精确匹配。
- **模型列表**：`anthropic_models_url` 未剥 `/messages`，Base URL 填完整端点时会把 models 拼成 `…/messages/v1/models`。已修复并加单测。
- **取消延迟**：三协议流式的 `send()` 改为可取消（`send_cancellable`），用户按「重新生成」或结束会话时不再被挂起的建连拖住最长 60 秒。
- **资料导入编码**：新增 UTF-16 BOM（记事本「Unicode」），且先于 UTF-8 判断——UTF-16LE 的 ASCII 字节是合法 UTF-8（带 NUL），先试 UTF-8 会解出整份乱码。
- **英文面试回声**：回声门槛原先只认汉字，英文麦克风文本永远不算回声；现在英文 ≥12 个字母同样按 2-gram 覆盖率判定。
- **问题判定**：请求动词补充「说明 / 解释 / 评价 / 分析」，英文支持 `Please …` 前缀，漏答面进一步收窄。
- **配置保存**：Windows 上「先删旧文件再 rename」在第二次失败时会丢掉整个 `settings.json`；改为 rename 重试 3 次后复制覆盖，失败时旧文件仍在。
- **音频重连状态**：通道短暂断线重连时先 `finish_channel`（清电平、重置迟滞边、联合状态回静默），避免重连后卡在「你在说 / 面试官在说」。初始化等待从 10 秒放宽到 15 秒，减少蓝牙/虚拟声卡误报。
- **健壮性**：chunked 响应长度改用 `checked_add`（畸形长度不再 panic）；worker 查询串 `%xx` 改在字节上解码（多字节字符不再可能触发字符串切片 panic）。
- **前端**：切协议时清空输入框里未保存的 Key（避免 A 的 Key 存进 B）；Base URL 变化时清空模型列表并丢弃过期响应；会话详情加请求序号，快速连点不再显示错会话；「清空」保留排队数；长英文/URL 在 320px 下可换行；修正两处重复的「拉取模型列表失败 / 模型下载失败」前缀与过时的「保存」按钮文案。

**保留的已知取舍**：断线补录的去重仍以整段 2-gram 覆盖率 0.7 判定，跨越「失败段 + 成功段」的片段可能漏补或重复，属启发式限制（README 已注明补录只补文字、不重新触发回答）；stall 看门狗 15 秒阈值未调整，冷启动极慢时仍可能触发一次误回收，但下一轮会自愈。

## 十六、对照三个参考项目的第三轮复查（2026-11-20）

逐一精读 pluely / natively-cluely / Freely 的音频与问答实现后，对照 hint 修复了以下有代码依据的问题：

- **补录片段的静音裁剪**：`split_into_speech_clips` 原先把撕开的整段帧（可能从长时间静音开始）直接送 SenseVoice，`pad_ms` 完全未使用。现在只保留语音本体与两侧各最多 300ms；stall 的 outage 起点常是 `quiet_since`，此前会把十几秒静音送进推理，既慢又容易产生幻觉。
- **识别失败的回溯范围**：outage 原先固定回溯 3 秒，而单个分段最长 12 秒；一个 10 秒段失败时前 7 秒永远补不回来。现在按失败任务的实际时长 + 3 秒回溯。
- **近期面试官文本的时效**：`recent_interviewer_texts` 原先只在新文本到达时剪枝，面试官静默 30 秒后仍参与回声判定，候选人的正常回答可能被误判成回声。现在取用时按 8 秒窗口过滤。
- **双层 VAD 门槛打架**：会话分段器最短语音 250ms，worker 的 Silero VAD 默认 `min_speech` 也是 0.25s，切出来的短促「嗯 / 对 / 好的」会在第二层被当作噪声丢掉。worker 默认降到 0.15s。
- **流式取消的盲区**：`send_cancellable` 只覆盖响应头之前；服务端回头部后长时间不发数据时，`pump_sse` 的 `next()` 会挂到 60 秒总超时，期间「重新生成 / 结束会话」无法打断。现在 `next()` 与取消轮询做 select，最多 100ms 响应。
- **英文问题判定**：补充 `What's …`（缩写按 what 处理）、`Is there …`、`Should we …`、`Does …` 等常见句式，减少英文面试漏答。

**对照后确认不做的取舍**：参考项目的自适应 VAD 阈值（噪声 EMA×3）需要持续电平统计，短期替换会破坏现有阈值语义；12 秒强制断段与 15 秒 stall 看门狗是配套设计，单独放宽会引入误回收；默认麦克风跟随依赖 cpal 的 `DeviceChanged` 自动迁移（`mic.rs` 已处理该分支），不再自建轮询；回声判定仍是 2-gram 覆盖率启发式，短回答重复问题措辞时可能误判，需要真实音频调参后再动。

## 十七、设置页一致性与错误体上限（2026-11-21）

第四轮细节复查，集中在文档与实现不一致、以及外部输入边界：

- **模型接口卡片的重复保存入口**：README 与设置页其他卡片都说明「保存统一走底部按钮、卡片内不再单独放保存按钮」，但 `ModelCard` 仍保留了一个「保存配置」按钮，且与底部「保存全部配置」文案不一致，用户可能以为两者行为不同。现在删除卡片内按钮，仅保留「测试连接」（测试成功会自动保存），提示文案改为指向底部「保存全部配置」。
- **文案统一**：`DevicesCard` 的「选择后记得点『保存配置』」与 README 中四处「保存配置」统一为「保存全部配置」，与按钮实际文案一致。后端日志/错误前缀「保存配置失败」保持不变。
- **错误响应体上限**：`ensure_success` 原先用 `response.text()` 无上限读取非 2xx 响应体，恶意或异常端点（例如网关返回超大 HTML 错误页）可能占用大量内存，之后才截断到 500 字。现在用 `bytes_stream()` 最多读取 64 KB 再截断，错误提示内容不受影响。

**复查后确认不动的点**：`SessionCard` 启动中「结束面试」按钮仍随 `busy` 禁用（此时可用 `Ctrl+Shift+R` 排队停止，属有意设计）；`ProfileCard` 前端 6000 字截断与后端截断重复但净结果正确，去重会引入多余分支；`hint-store` 迁移失败的事务回滚依赖 SQLite 默认行为，风险极低。

## 十八、第五轮细节复查（2026-11-21）

本轮从「用户实际点击路径」出发复查，修复了三个问题：

- **重新生成可能丢失上一轮答案**：前端 `regenerate()` 拿到命令成功后立刻把答案清空成 `EMPTY_ANSWER`；而后端随后发出的 reset 帧才负责把上一轮答案归档进「更早的回答」。若命令响应先于 reset 事件被处理，reset 帧看到的 `previous.text` 已为空，旧答案就不会被归档，从时间线里消失。现在前端只把状态切成流式（保留旧文本），清空与归档统一交给 reset 帧。
- **纯标点问题导致「永远在生成」**：`Ctrl+Shift+A` 在 `last_question` 只有标点/空白时（STT 静音幻觉）会先取消当前流、清空队列，再入队；该问题归一化后为空会被判重复静默丢弃，结果没有任何新请求，浮层一直显示流式状态。现在 `regenerate` 在取消之前先校验问题含可识别文字，否则直接返回中文错误。
- **会话卡片按钮文案错位**：`useSessionControl` 只有一个 `busy` 标志，启动中（`connecting` 且 `busy`）时「结束面试」按钮虽被禁用却显示「正在结束…」，与实际动作相反。现在区分 `busyAction`（start/stop），只有停止时才显示「正在结束…」。
- **SSE 单行缓冲上限**：`pump_sse` 的 `buffer` 只按换行切分，异常/恶意端点持续发送无换行数据会让它无限增长。现在处理完完整行后若剩余半行超过 1MB 即停止生成并返回中文错误；正常增量远小于该阈值。

**复查后确认不动的点**：`SessionCard` 在启动中仍禁用「结束面试」（可用热键排队停止，属有意设计）；`ClipOptions.min_speech_ms` 仍为 400ms（补录切段对噪声更保守，实时路径的 250ms 不受影响）；`response.text()` 读取成功响应体仍不设上限（只针对用户自己配置的模型接口，且受 60 秒超时约束）。

## 十九、第六轮复查：对照三个参考项目逐层重读（2026-11-23）

本轮按四条链路分工重读（core / 壳层 / STT+音频 / 前端），并与 pluely、natively-cluely、Freely 的对应实现逐段比对。共修 30 处，其中高 2、中 14、低 14。

### 19.1 音频与 STT（对照 pluely speaker、natively VAD、Freely STT）

- **切换默认输出设备被当成通道失败**：`follow_default` 的迁移分支返回 `Err`，引擎按「采集中断」上报并累加 `failures`；插拔耳机 / 蓝牙闪断 5 次（15 秒宽限）就把系统声通道判死，用户此后听不到面试官。现在引入 `SessionEnd::{Stopped, Migrated}`，迁移走 `warning` 并清零失败计数与降级提示，与「显式指定设备」的提示区分开。
- **loopback 缓冲零余量**：共享模式 buffer 直接取 `min_period`（常见 10ms），采集线程晚一两个调度周期 WASAPI 就覆盖未读包，症状是「偶发漏字」而不是报错。改为 `min_period.max(200ms)`，仍封顶 1 秒。
- **就绪快路径只比大小**：`is_installed` 不看记录里的 sha256，升级换模型（或同大小换过文件）被判成已安装，worker 随后加载坏模型。现在要求记录摘要等于清单当前摘要，再走大小快路径，并补单测 `readiness_requires_the_manifest_digest`。
- **进度条与 `prepare` 判据不一致**：`downloaded_bytes` 只看「有记录」，摘要对不上的旧文件也算已下载，进度条接近 100% 却要整份重下。
- **206 区间过严**：原先要求 `end == bytes-1`，只回一段区间的 CDN / 代理会让 239MB 的 `.part` 被整份删掉重下；现在接受「起点正是续传点、终点不越界」的短区间。
- **请求级拒绝连带丢掉健康 worker**：400 / `invalid-input`（WAV 格式不符、语言不支持）也走「丢弃 worker」，下一段音频要重新加载 239MB 模型（约 2.5 秒）。现在只对真正的会话失效丢弃。
- **英文错误直出状态栏**：`InvalidInput` / `Json` 走 `Display`，界面出现「语音识别失败，稍后自动重试：invalid input: audio must be mono」。新增 7 类中文映射 + 中文兜底前缀。
- **慢速滴流绕过推理超时**：`post_wav` 只在 `WouldBlock` 分支查总截止时间，每 250ms 吐一个字节的连接能把持锁拖到响应体上限。现在读到数据后也查 120 秒截止。
- **取消后仍要等握手**：`transfer` 在建连前不查取消，点「取消」还要等最长 15 秒握手超时。
- **并发下载拒绝文案是英文**：经 `user_message_for` 显示成「配置错误：a download is already running」。

### 19.2 核心逻辑（LLM / 问题判定 / 配置）

- **空流式响应被当成成功**：`stream()` 把 `Ok((text, ttft))` 一律当成功，`pump_sse` 一行 `data:` 都没拿到时返回空 `ChatStats`；前端只在「text 与 model 都空」时才清空，于是浮层停在空白答案且没有任何提示（网关忽略 `stream:true` 直接回整包 JSON 时必现）。现在未取消却一个字都没收到时报可读错误。
- **错误响应体读取不可取消**：`ensure_success` / `read_error_body` 在非 2xx 后读 body 不查取消，服务端挂着不发 body 会拖满 60 秒尝试超时，串行的答案 worker 期间无法被「重新生成 / 结束会话」打断（与 §15、§16 同类盲区的最后一处）。
- **长纯符号串被判成问题**：「含问号即问题」在超过 24 字符时跳过寒暄剥离，STT 静音幻觉（「？？？」「……」）会入队一个空问题、浮层空跑一轮流式（§18 只在前端 `regenerate` 层堵住）。
- **固定搭配误命中问句标志词**：「哪怕需求变了」「无论如何都要上线」「这个改动多少有点风险」都会触发一次无意义自动作答。`CN_NON_QUESTION_FRAGMENTS` 补 7 条搭配并加 6 条陈述句用例。
- **非测试代码残留 `expect`**：`LifecycleQueue::request` 的 `expect("刚入队，队首必然存在")` 与 §4.1「非测试仅 1 处」不符，`panic = "abort"` 下无法兜底。改为 `if let Some`。
- **并发保存互踩临时文件**：临时文件名固定为 `json.tmp`，两个 `save_settings`（前端连点 / 托盘同时保存）互相覆盖，先 rename 走的那次让另一次报「文件不存在」并留残骸。现在带进程内序号，且序列化与写盘失败都会清临时文件。
- **`dropped_ms` 虚报**：断线窗口的丢弃时长按「所有被淘汰帧」累加，没有按 outage 起点过滤（natively 的实现是 `f.startMs + f.durMs > outage.startMs`），字段语义是「窗口内」却算进了窗口外的旧帧。

### 19.3 Tauri 壳层与 CI

- **托盘失败连带吞掉热键**：`setup_after_ready` 用 `?` 把托盘与热键串起来，托盘构建失败会直接返回，四个全局热键永不注册，而且提示被误报成「全局热键注册失败」。现在先注册热键再判托盘，托盘失败单独提示。
- **异步任务里做阻塞调用**：`answer_one` 在 Tauri 异步任务里直接读 Windows 凭据管理器，域账号漫游或凭据服务卡住会占住 runtime 线程，整条作答链路被拖住。改为 `spawn_blocking`，任务失败并入同一条失败回报路径。
- **11 个同步命令在 UI 主线程做 IO**：`list_sessions`（每个会话两次子查询）、`export_session_markdown`、`delete_all_sessions`、`has_api_key`、`save_api_key`、`get_settings` / `save_settings`、`open_log_dir`、`model_status` 在 UI 线程跑 SQLite / 凭据 / 配置文件，面试中打开设置或复盘会卡窗口。统一标 `#[tauri::command(async)]`（tauri-macros 2.7 的 `sync_threadpool` 执行上下文），前端签名零变化。
- **DOCX 解压炸弹**：`read_docx` 用无上限 `read_to_string` 读解压后的 `word/document.xml`，20 MB 的包可膨胀到数 GB。改为 `take(32MB + 1)` + `read_to_end`，超限给中文提示。
- CI / capabilities / `tauri.conf.json` 逐项核对后无异常（toolchain `stable` 满足 edition 2024、artifact 路径与 workspace target 一致、NSIS 三键在 `NsisConfig` 白名单内、release 有 `contents: write`）。

### 19.4 前端

- **流式中「清空」后答案复活**：清空只清前端文本，后端流还在跑，下一帧增量走 reset 分支从半句处重新长出来，`answer-done` 到达时又贴回全文。新增 `answer.muted`：本轮后续增量与最终全文都不上屏，直到下一轮 reset。
- **33ms 一次的电平把整棵树重渲染**：`hint://levels` 每秒约 30 帧，浮层所有状态在一个 `useState` 里，长答案文本每次都要 diff 与重算 `text-shadow`。`AnswerPanel` / `TranscriptPanel` 加 `memo`，`Overlay` 内回调改 `useCallback` 稳定引用。
- **「本场已答 N 题」到 11 就停**：计数用 `history.length`，而 history 被 `slice(-10)` 截断。改为独立 `answeredTotal`。
- **保存 / 测试期间切协议污染 Key**：回写用的是旧闭包里的 `provider`，「Key 已保存」画到新协议头上，`setApiKey("")` 还会清掉刚粘的新 Key。加 `providerRef` 守卫，并在提示里说明结果属于哪个协议。
- **会话列表刷新无请求序号**：切 Tab 与手动刷新并发时，先发的旧响应可能覆盖新列表（详情请求早已有序号）。
- **时间线 key 用 reverse 后的下标**：第 11 条起 `slice(-10)` 让所有 key 位移、整段重挂。改为归档时分配单调 `id`。
- **错误前缀重复 / 过期提示**：`导出失败：导出失败：…`、`删除失败：删除会话失败：…`；「清空全部」失败时还附「若后端尚未提供该命令」（命令早已存在）。
- **a11y**：方向键被全局劫持到答案区（转录面板键盘翻不动）、生成中只有 `<i title>`、toast 无 live region。补 `role=log` + `tabIndex` + 向内焦点环、`visually-hidden` 文案、toast 的 `role` / `aria-live`。

### 19.5 本轮验证与待远端确认

本机未跑任何 Rust 编译（仓库约定）。已验证的层：

- 前端类型：`node_modules\.bin\tsc.cmd --noEmit -p tsconfig.json` → 退出码 0，无输出。
- 前端构建：`npm.cmd run build` → 48 modules，`dist/assets/index-BbawwSVl.css` + `index-9FPeIGs9.js`，退出码 0；连跑两次产物 hash 一致。
- Rust 语法：14 个改动文件用 `rustfmt 1.11.0-stable --edition 2024 --emit stdout` 逐个解析，全部通过（语法级，不含类型检查）。
- 浮层行为：`vite dev` + 无头浏览器注入 `hint://*` 伪事件，实测「清空后被清空轮不再上屏」「已答 N 题随归档递增」「转录可聚焦」「320×180 底部不被裁」。

待 CI（windows-latest + MSVC）确认：`cargo check --locked -p hint --bins`、`cargo test --locked -p hint-core -p hint-store -p hint-audio`、`cargo test --locked -p hint-stt`（含两个新单测）。真机需重点验：切换默认输出设备只出现一条警告并继续采集、蓝牙输出连播 10 分钟无漏字、网关忽略 `stream:true` 时的中文错误提示。

### 19.6 考虑过但不改

- worker 请求头 `read_line` 无界（仅 127.0.0.1、爆炸点在 token 校验前，改用 `fill_buf`/`consume` 会牵动读体逻辑）；401/413 未排空请求体可能触发 RST（客户端已预校验长度，概率极低）。
- `engine` 启动 15 秒是两通道串行等待、最坏 30 秒（合并为单一预算要重写无锁轮询，风险高于收益）。
- `AudioHandle::stop()` 无超时 `join`（`JoinHandle` 无带超时 join，属整块重构）。
- OpenAI / Anthropic 补「非 SSE 整包 JSON」兜底（Gemini 已有，需各复制一套正文抽取，本机无法验证）。
- 答案 worker panic 会让 `worker_running` 永久为真（只有 crate 内 `llm::stream` panic 才触发，Drop 守卫需重新设计）。
- 5.1 声道下混权重、`wav.rs` 不校验 `audioFormat == 1`、`Settings::load` 读失败回退默认值后可能覆盖用户资料：都需要真机/真实文件调参或改签名，不在本轮夹带。
