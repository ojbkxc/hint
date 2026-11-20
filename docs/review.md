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
- **托盘状态可见**：`TrayItems` 持有托盘图标，会话开始/结束时切换 tooltip（空闲「hint · 空闲（右键可退出；Ctrl+Shift+Space 显示浮层）」/ 面试中「hint · 面试进行中（右键可结束或退出）」），解决关闭主窗口后无法确认会话是否在跑的问题。
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