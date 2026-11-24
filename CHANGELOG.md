# 更新日志

本文件格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循语义化版本。

只记录每个版本对用户或开发者有实际影响的变化；更早的细节见 git history。

## [1.1.0] - 2026-11-24

### 修复：共享屏幕时浮层变成黑框

1.0.26 的「浮层不进屏幕捕获」默认开，实测**害大于利**，本版改为**默认关**。同一台机器上按三条捕获路径实测同一个窗口（自建绿色窗口，抓屏后读中心像素）：

| 捕获路径 | 未保护 | 已保护 |
|---|---|---|
| GDI `CopyFromScreen` | R=0 G=255 B=0（可见） | 透出下层桌面（正确排除） |
| `PrintWindow` + `PW_RENDERFULLCONTENT` | 可见 | **R=0 G=0 B=0 纯黑块** |
| 旧式 `PrintWindow`（flag=0） | 可见 | **完全绕过保护，内容原样可见** |

腾讯会议等软件的共享屏幕走的正是后两类路径：于是开了这个开关，对方看到的是**一块黑框**（比被看到内容更尴尬，等于宣布「我在用工具」），而旧式捕获根本拦不住。

- 默认改为关闭，开关保留；UI 文案如实写明利弊与「先在本地自测一次」。
- 更可靠的替代写进文案与发布说明：面试前按 `Ctrl+Shift+H` 直接隐藏浮层。
- 1.0.26 发布说明里「共享屏幕时对方看不到」的说法已加注更正，避免继续误导。

### 修复：语音识别把中文识别成乱码

- 原先 `transcribe` 写死 `"auto"`，由模型自己判语言。中文口语里夹几个英文单词时容易判偏（日/韩），整句输出成乱码。真机对照：英文 TTS 转录一句不差，中文 TTS 全乱。
- 新增 `Settings.stt_language`，**默认 `zh`**：SenseVoice 的中文模式对中英混说有训练覆盖，夹的英文词也能正常识别。纯英文会议可切 `en`，或切回 `auto`（UI 注明中文场景不建议）。
- 设置页新增「语音识别语言」（zh / en / yue / ja / ko / auto）。
- 实时识别与断线补录两处调用共用同一份设置——口径不一致会让同一段话前后识别结果不同。

### 新增：截图区域问答（`Ctrl+Shift+S`）

- **框选屏幕区域直接提问**：`Ctrl+Shift+S` 弹出全屏选区，按住左键拖出区域（拖动时实时显示 `宽 × 高`），松开即抓取这一块屏幕；`Esc` 取消，选区小于 8×8 像素时只提示、不抓取也不发送。此前该热键只弹一句「将在 M5 启用」的提示。
- **走多模态模型流式作答**：截图与提问一起交给当前配置的模型接口，流式回来；答案显示在浮层新增的「截图问答」区块，与语音问答的答案时间线分开，不会被下一轮语音答案覆盖。
- **解码与截图下沉到 `hint-core`**：区域抓取（Win32）+ JPEG 编码都在核心层完成，壳层只做编排，不依赖 Tauri。
- 选区上限 4096×4096 像素，超出时提示缩小选区，不会硬截一张超大图再拿去上传。
- 设置页文案跟进：「回答行为 → 热键」里的「截图（M5）」改为「截图问答」并补一行说明；「模型接口」卡片的 `M5 预留` 徽章换成「需要视觉模型」，说明改为「需要支持视觉输入的模型 + 模型不接受图片时接口会报错」。
- **隐私边界写进 README 与代码注释**：截图只在内存里编码，不写磁盘、不进会话数据库、日志不含图像数据；只发往用户自己在设置页填的模型接口；不做后台持续截屏与全屏轮询，只在按下 `Ctrl+Shift+S` 并真正框选后抓取一次。

### 修复

- `finish_capture` 不再无条件返回错误：此前无论选区如何都回「截图问答尚未启用（M5 预留）。」，选完区域只会看到一句失败提示；现在按真实选区截图并作答。
- 清掉全仓残留的 M5 占位口径：README 功能概览、热键表、FAQ、已知取舍、路线图表，以及 `docs/release-checklist.md` 的 `Ctrl+Shift+S` 验收项，全部改为真实行为描述，不再让人以为该功能要等下一版。

### 验证

- 前端用例随改动同步更新（`BehaviorCard.test.tsx` 热键清单、`ModelCard.test.tsx` 视觉模型说明、`CaptureOverlay.test.tsx` 选区交互），以 CI 的 `npm run test` 结果为准。
- CI（`build.yml`）全链路：`tsc`、`eslint`、`vitest`、`cargo fmt/clippy/test --locked`、`cargo-deny`、`cargo-audit`，以及 `--doctor` 冒烟断言版本三重对齐（`package.json` / `hint.exe` 文件版本 / 报告内版本，本版统一为 1.1.0）。
- 本轮开发禁止本地编译与真机运行，截图抓取与多模态接口的端到端证据以 CI 构建产物与会话验证为准；真机验收项已写进 `docs/release-checklist.md`。

## [1.0.26] - 2026-11-24

### 新增：面试时不泄露

- **浮层默认不进屏幕捕获**（`overlay_content_protected`，默认开）：腾讯会议 / 钉钉 / 飞书 / OBS 共享屏幕时，对方看到的是桌面，看不到浮层与答案；本机屏幕上照常可见可读。设置页「回答行为 → 浮层外观」可关（自己录屏留证时）。底层 `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`（tao 已封装），Win10 2004+ 生效；失败只记 WARN，不影响使用。
- **出网代理三档**（`proxy_mode` / `proxy_url`）：直连（默认）/ 跟随系统（读 `HKCU\...\Internet Settings` 的 `ProxyEnable`/`ProxyServer`，即代理软件写「系统代理」时的值）/ 手动指定。照做 `pi-agent-desktop` 的口径：按协议分开时优先 https、缺协议补 `http://`、尾部斜杠去掉。
  - 模型接口（reqwest）与语音模型下载（ureq）共用同一份设置；流式客户端的缓存键带上生效代理，改设置后立即重建（原先 `OnceCell` 不重建，表现是「改了还是连不上」）。
  - **本地回环永不代理**：`LOOPBACK_NO_PROXY` 列 localhost / 127.0.0.1 / ::1，有单测钉住。
  - 选直连时显式 `no_proxy()`，忽略 `HTTP(S)_PROXY` 环境变量。
  - 代理地址无法解析 → 退回直连并记 WARN，不塞占位地址硬走代理。

### 修复 / 调整

- **字号上限 160% → 300%**：最小值够用、最大值不够。前端常量与后端下发 clamp 同步。
- **字号滑块被底栏挡住**：两个滑块原本各占一个 field + 整段说明，比面板可视区还高（实测滑块 `top=602`、可视高 378px），用户根本发现不了。改成紧凑布局（数值移到 label 行右侧、说明合并成一句）。顺带可访问性更好：读屏会读「浮层不透明度 88%」。
- **浮层底部永远显示「未连接模型」**：`answer.model` 只有答过题才有值，没答题时永远空。改为启动时读配置的模型名存进 `configuredModel`，浮层优先显示答案实际用的模型、回落显示配置的模型；两者都空时文案改成「未配置模型」（描述配置缺失，不是连接状态）。

### 验证

- 474 用例全绿、tsc 0 错、eslint 0/0；Rust 侧经 CI 编译 / clippy / cargo-deny / audit / `--doctor` 冒烟全过。
- 真机验收：设置页「测试连接」对用户实际接口返回「连接成功 耗时 24.5s」并带内容，确认接口与凭据本身正常（问题在浮层绑定，已修）。

## [1.0.25] - 2026-11-24

### 修复

- **同一条状态提示显示两次**：Dashboard 底部固定栏与 ModelCard 都在渲染同一份 `form.status`，当前 section 是「模型接口」时用户会看到两行一模一样的提示。现在全局状态只由底部保存栏显示，ModelCard 不再接收 `status`（卡片内保留自己的测试连接结果卡与截图提示）。ProfileCard 的同名 `status` 是它自己的局部导入状态，与全局无关，未动。
- **「保存失败：保存配置失败：」双重前缀**：后端 `save_settings` 的错误本身已带「保存配置失败：」，前端又拼了一次。改为原样透传。（`save_api_key` 的「保存 API Key 失败：写入 Windows 凭据失败：…」是「动作：原因」结构，信息不重复，保持不动。）

### 验证

- 1.0.24 的保存守卫通过真机验收：清空 Base URL 后点保存被明确拒绝，配置文件内容保持不变，日志记录拒绝原因。
- 474 用例全绿、tsc 0 错、eslint 0/0、vite build 通过。

## [1.0.24] - 2026-11-24

### 修复

- **保存配置时拒绝空的 Base URL / 模型名**：1.0.23 修掉了「表单变空」的根因（AppState 注册时机），但同一个后果还有另一条入口——只要表单里这两项为空，点「保存全部配置」就会把空值写进 `settings.json` 覆盖已填内容。现在保存前先校验，任一为空则**拒绝写盘**并返回「Base URL 为空，保存会覆盖已填好的配置，请先填写再保存。」，日志记一条 error（含协议与拒绝原因，不含字段内容）。
- 校验逻辑下沉到 `hint_core::settings::missing_required_fields` 纯函数（壳层 `cargo test -p hint` 因 sherpa 静态库不可用，放那里等于没人守），命令层只调它、不自带第二份。4 条单测进 CI 门禁：只空 Base URL / 只空模型名 / 两者都空 / 字段齐全放行（正向对照防误伤）。

## [1.0.23] - 2026-11-24

### 修复

- **「配置被清空」的真因**：`AppState` 原先写在 `.setup()` 里注册，而 setup 在窗口与 webview 建好**之后**才执行；前端首屏的 IPC 请求撞上「state not managed for field `state`」直接失败，设置页表单停在空值，点「保存全部配置」就把空的 Base URL / 模型名写回 `settings.json`，覆盖真实配置。
- 现在状态在 `Builder` 阶段（窗口创建之前）注册，任何窗口 IPC 都不可能更早到达。数据目录按 `%APPDATA%\<identifier>` 在 Builder 阶段推导，并在 setup 中与 Tauri 官方 `app_data_dir()` 比对，不一致记 error 日志暴露，避免静默读写另一份配置。

### 说明

- 上一版的 `isTauriAsync` 等待保留，但它不是本次根因（实测 IPC 在 12ms 内就注入完成）；注释里已注明。
- 之前若配置被清空，本版已修复根因，重新填写并保存一次即可。

## [1.0.22] - 2026-11-24

### 修复

- **「配置存不住」的真因**：Tauri 的 IPC 注入脚本与前端 JS 并行加载，首屏读配置时偶尔还没就绪，旧 `isTauri()` 只查一次 → 判定「不在 Tauri 环境」→ `safeInvoke` 直接降级、请求根本没到后端（后端日志零痕迹）→ 设置页表单停在 `EMPTY_SETTINGS`，用户此时点「保存全部配置」就把空字符串写回 `settings.json`，覆盖掉真实的 Base URL 与模型名。现在 `safeInvoke` / `safeListen` 改用 `isTauriAsync()`，在 1.5 秒窗口内等待注入完成；纯浏览器预览等满仍返回 false，行为不变。（Vitest 环境下窗口取 0，用 `import.meta.env` 判定，避免 fake-timers 用例白等超时。）

### 新增

- **浮层字号可调**：`Settings.overlay_font_scale`（0.8–1.6，默认 1.0）+「回答行为」页滑块（80%–160%，step 5）。存缩放比而非像素值，答案 15px / 转录 12px / 问题 11px 按同一比例放大，比例关系不变。下发与不透明度同构：保存后 emit `hint://overlay-font-scale`，浮层只改 CSS 变量不重渲染（不打断流式输出）。
- **「Ctrl+Shift+H 显示浮层」做成真按钮**：点一下调用 `toggle_overlay`（与热键同一条后端命令），按返回的可见状态切换文案「显示浮层 / 隐藏浮层」，热键仍以 `<kbd>` 显示；切换失败落页面 banner，切换中禁用防连点。

### 说明

- 之前若遇到「配置被清空」，本版已修复根因，重新填写并保存一次即可。

## [1.0.21] - 2026-11-24

### 修复

- **模型可能因清单版本号变化而重下（隐患）**：`read_manifest` 原先要求 `version == MANIFEST_VERSION`，否则整份判无效。这意味着以后任何一次清单版本 bump（哪怕只加字段）都会让所有用户重下 241 MB。兼容性改由结构解析判断，文件是否可信仍由 `is_installed` 的 sha256 + 大小二次校验兜底；读入后版本统一规整为当前常量。新增 `stale_manifest_version_still_counts_as_installed` 守住，并修正 `corrupted_manifest_reads_as_empty` 里依赖旧门控的断言。
- **配置保存成/败均无日志**：用户反馈「配置存不住」时 `hint.log` 一片空白，无法判断是没点到、被拒绝还是写盘失败。现在保存失败记 error（含路径）、成功记 info（协议、Base URL 与模型名是否填写、不透明度——不记录内容以免泄露自建网关域名）；`get_settings` 也记一条，用于区分「没写成」与「写成了但回滚成默认」。
- **`--doctor` 在全新机器上写不出报告**：写文件前未创建数据目录，而全新安装 / 卸载重装后该目录本就不存在——恰是最需要诊断时。已补 `create_dir_all`。

### 新增

- CI 增加 `--doctor` 冒烟测试：构建后真实运行一次，断言报告 >200 字节、7 个分节齐全、版本三重对齐（`package.json` / exe 文件版本 / 报告内版本）、日志缺失不被误报为读取失败。本轮就是它抓出了上面「全新机器写不出报告」的缺陷。

### 验证说明

文件系统层面实测：Windows 下 `MoveFileEx(MOVEFILE_REPLACE_EXISTING)` 能可靠覆盖目标，`Settings::save` 的 rename→重试→copy 兜底策略可生效。（先前用 PowerShell `File.Move` 测失败是测试脚本用错 API，非产品问题。）

## [1.0.20] - 2026-11-24

### 修复

- **`--doctor` 的控制台输出从未生效**：真机跑 1.0.19 才发现 `hint.exe --doctor` 在 cmd 与 PowerShell 下 stdout 恒为空，只有报告文件能拿到。根因是程序为 Windows GUI 子系统，进程启动时没有控制台、stdout/stderr 句柄无效；原先只调了 `AttachConsole(ATTACH_PARENT_PROCESS)`，控制台附加上了但 Rust 的 `std::io::stdout()` 仍指向无效句柄。现在补上 `CreateFileW("CONOUT$")` + `SetStdHandle` 重绑输出与错误流。**实测 stdout 从 0 行变为 45 行**（cmd）。
- 报告冗余：「模型根目录」与「安装位置」在就绪态是同一路径，删掉前者。
- 旧模型目录提示细化：仍有数据时报体积并说明「确认新位置可用后再删」；只剩空壳（迁移成功的正常残留）则直接说「可安全删除」。真机上此前只给一句含糊的「若非残留可手动删除」。

### 说明

- PowerShell 直接调用 `hint.exe --doctor` 仍看不到输出——那是 PowerShell 不等待 GUI 进程、且对这类进程的管道捕获行为不同所致，不是程序缺陷。README 已写明 PowerShell 下改读 `doctor-report.txt`。

## [1.0.19] - 2026-11-24

### 修复

- **`--doctor` 的日志级别误判**：原先用 `line.contains(" WARN ")` 判断级别，正文以 WARN 开头的 INFO 行（如 `INFO WARN 出现在正文里`）会被算成告警。改为按 tracing 的字段格式解析——时间戳不含空格，第二个空白分隔 token 就是级别，精确匹配 `WARN` / `ERROR`。已用真机 `hint.log`（27 行）验算：命中 5 条真告警，INFO 行不再误报。

### 工程

- `--doctor` 的纯逻辑下沉到 `hint-core::diagnostics`：`tail_warnings`（窗口裁剪 + 15 条上限 + 160 字截断）、`is_warn_or_error`、`device_label`（只报「默认/已指定」，不回显 wasapi 端点 ID）、`format_mb`。壳层 `doctor.rs` 只保留 IO 包装。
- 6 个新单测进 CI 门禁（`cargo test -p hint-core`）——此前这些逻辑长在 `src-tauri` 里，而 `cargo test -p hint` 因 `sherpa-onnx-sys` 静态库链不上 harness 而不可用，等于没有测试守护。这次正是新单测抓出了上面的级别误判。

## [1.0.18] - 2026-11-24

### 新增

- **`hint.exe --doctor` 自助诊断**：不启动界面，只读体检——版本、数据目录、语音模型就绪态（含是否需重下、旧目录是否残留）、当前协议与 API Key 是否已保存、Base URL/模型名是否填写、会话数据库结构版本与 `integrity_check`、可枚举的输入/输出设备、最近日志 WARN/ERROR。有阻塞项标 `[阻塞]` 并在末尾汇总数量，退出码非 0 便于脚本判断。
- 报告双出口：优先 `AttachConsole(ATTACH_PARENT_PROCESS)` 抢父控制台打印；同时写 `%APPDATA%\com.ojbkxc.hint\doctor-report.txt`（release 是 windows 子系统、默认无控制台，文件是可靠兜底）。
- 模型根规则抽成纯函数 `model_root_path()`：主程序版带迁移副作用，`--doctor` 用纯版——诊断绝不能顺手搬 241 MB。

### 说明

- 报告里 Base URL 与模型名只报「是否填写」不打印内容，避免用户随手转发时泄露自建网关域名或账号信息。

## [1.0.17] - 2026-11-24

### 工程

- **测试覆盖补完最后一块**：`src/main.tsx` 窗口分发 7 例（`?window=overlay|capture` 参数优先、`getCurrentWindow().label` 回落、异常与未知 label 回落 Dashboard、StrictMode 只挂载一次）。总计 473 用例、24 个测试文件，全部组件与入口均有覆盖。
- **`recovered` 列 Rust 侧补测**：实测此前 `add_segment` 全部单测都传 `recovered=false`，该列在 Rust 侧零往返覆盖。新增 test 覆盖实时 false / 补录 true / 补录+回声两标记独立 / 绕开 `add_segment` 直插模拟 V1 老行验证 `DEFAULT 0` 回落。
- 文档与门禁对齐：SECURITY.md 增「依赖与供应链」章节（audit / deny / clippy+fmt / 前端三件套四道门），CONTRIBUTING.md 与 README.md 的本地检查清单补齐 lint、test、fmt --check、clippy --all-targets、cargo deny。
- `main.tsx` 兜底注释与行为对齐（回落 `"main"` → Dashboard）。

### 验证说明

`recovered` 列的**端到端**证据（真实断网触发补录、段落以 `recovered=1` 落库）仍未取得——库层写读、列约束、代码层分流三方面已分别验证，但缺一次真机断网会话。已在 CHANGELOG 与 release 说明中如实标注，不宣称端到端通过。

## [1.0.16] - 2026-11-24

### 新增

- **cargo-deny 三级门禁**：`deny.toml` 许可证白名单（拒绝 GPL/AGPL，守住「仅借鉴 pluely 思路」边界）、bans 重复版本 warn、sources 锁 crates.io。本机预编译二进制预演三关全过（`libbz2-rs-sys` 的 `bzip2-1.0.6` 显式例外，类 BSD）。
- 前端测试第四波 +83（总 466）：Dashboard 59（rail 导航全分支、六类状态徽标 table 驱动、fake timers banner 6s/10s 失活、tab 切换、保存/日志目录/测试连接流转）、CaptureOverlay 24（四方向 normalizeRect、8px 边界、Esc/pointerCancel、监听清理）。两处变异验证（hidden 反转 4 红、min→max 6 红）。全组件测试覆盖完成。

### 修复

- **截图选区三处**（测试轮发现）：`setPointerCapture` 无 try（宿主缺 API 时选区整个打死）；`releasePointerCapture` 前缺能力检测；`finishSelection` 用 state 的 `current` 而非 pointerup 坐标——快速拖拽时最后一次 move 可能早于 up，产生偏小选区（真缺陷）。
- `speechEdge` 枚举大小写容错（后端若变 `"BOTH"` 不再被静默丢弃）。
- 仓库 license 元数据：src-tauri 补 `license = "MIT"`（三个 crate 已有）。

## [1.0.15] - 2026-11-24

### 工程

- **前端测试第三波**（+167 用例，总 383 全绿）：useSettingsForm 36（save/testConnection/loadModels 全路径，含 providerRef 协议守卫与请求序号双守卫、非 Tauri 降级分支）、ModelCard 25、ModelDownloadCard 15、ProfileCard 14、BehaviorCard 8、DevicesCard 11、SessionCard 15、StatusBar 14、Overlay 29（memo 渲染稳定性、Esc/重新生成/notice toast 全分支）。总覆盖率 16.7%→**80.4%**（settings 96.4%、overlay 96.9%、sessions 95.8%）。providerRef 与序号两处守卫完成「改坏必红」变异验证。
- 消除最后一个 lint warning：`DevicesCard` 的 `refresh` 提 `useCallback` 补 `exhaustive-deps` 依赖。ESLint 现在 0 error 0 warning。
- 1.0.12 卸载保护真机端到端验收通过：卸载（含程序目录）/重装后无重新下载 241 MB；`%LOCALAPPDATA%\hint-models` 原子迁移；数据库 `user_version=2` 与 `recovered` 列自动应用。

## [1.0.14] - 2026-11-24

### 修复

- **复盘页「补录」图例成死 UI（上线以来）**：断线恢复补录的段落只在浮层事件带 `recovered:true`，写库时丢字段——`segments` 表没有该列。V2 迁移（`ALTER TABLE ... ADD COLUMN recovered`，幂等由 `user_version` 保证）；实时/补录两条写库路径分别传 false/true；前端 `normalizeSessionDetail` 补归一化；复盘页输出 `recovered` 类、「补录」标签与左指示条样式。迁移幂等测试改为断言 `MIGRATIONS.len()`，后续加迁移不再跟着改。
- **启动竞态吞错误态**：`useSessionControl.start` 成功返回只保护 `listening`，后端先发 `error` 事件时被覆盖回 `connecting`（只留文案不留状态）。保护面对齐失败分支。两个把旧行为固化成断言的测试同步改为修复后语义。
- **复盘列表「时间未知」兜底不可达**：meta 由「模型缺失回落『模型未知』+ 时长」拼接，模型回落使 meta 恒非空。改为时长缺失直接显示「时间未知」。

### 新增

- **CSP 最小集启用**（README/SECURITY 的承诺兑现）：生产 `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; connect-src ipc: http://ipc.localhost`；dev 额外放行 `localhost:1420` 与 ws（Vite HMR）。已核实无可破坏面：无 asset: 协议引用、无 canvas 导出、无前端 fetch、dist 无 eval。
- **前端测试第二波**（+150 用例，总 216）：AnswerPanel 26、TranscriptPanel 9、useAutoScroll 10、useSessions 20、SessionReview 20、useSessionControl 19、useModelManager 21、statusTone 25；整体覆盖率 16.7%→50.7%，新覆盖文件 95–100%。两条 hook 关键分支完成「改坏必红」变异验证。

## [1.0.13] - 2026-11-24

### 修复

- **Anthropic 带图消息回归（1.0.12 引入）**：`anthropic_body` 带图分支做 collapsible_if 折叠时把「末条是 user 才 pop 并入」的角色检查整个删掉，末条为 assistant 时被错误弹掉、请求体少一条消息；CI 单测 `anthropic_body_merges_consecutive_roles_and_images` 拦下。补回角色条件。
- 依赖安全升级：zip 2.6.1→9.7.0（连带与 sherpa-onnx-sys 收敛为同一版本；feature `deflate` 改名 `deflate-flate2-zlib-rs`）、pdf-extract 0.9→0.12.1（带动 lopdf 0.36→0.42 栈溢出修复）。url（RUSTSEC-2026-0592）与 cookie（0594）为 tauri/wry 生态锁定的传递依赖，hint 的利用面不存在（不解析不可信 blob: URL、非 fork 的桌面进程），audit 豁免并注明理由。
- **CSP 启用（最小集）**：README 承诺的「后续版本再补」兑现——default-src 'self'、script-src 'self'、connect-src 仅 ipc、style-src 允许 'unsafe-inline'（电平/进度/透明度是运行时内联样式的必要面），img/font 允许 data:。

### 工程

- clippy `-D warnings` 清零：17 处既有写法修复（collapsible_if let-chains 折叠、needless_borrow、question_mark、manual_pattern_char_comparison、manual_is_multiple_of、chunks_exact→as_chunks、type_complexity 提 `DispatchDone` 别名）。
- cargo-audit 加入 CI 并清零（zip/lopdf 已升级；url/cookie 带理由豁免）。

## [1.0.12] - 2026-11-23

### 修复

- **卸载重装后语音模型被删、被迫重新下载 241 MB**：Tauri NSIS 卸载器在勾选「删除应用数据」时会整目录删除 `%APPDATA%\com.ojbkxc.hint`，语音模型原来就存放其中。模型改存于 `%LOCALAPPDATA%\hint-models`（不含应用标识，卸载器不触碰）；从旧版本升级时已下载的模型自动原子搬移（同卷 rename），失败则继续使用旧位置不阻塞启动。
- **浮层答案文字发灰**：浮层不透明度滑块只作用于白色背景；模型返回的答案正文新增显式实色规则（`#14161a`、`opacity: 1`、`text-shadow: none`），任何不透明度下正文都为实色。

### 新增

- 设置页按「模型管理」审美重构：左侧分组导航（接口与模型 / 会话 / 资料与行为 / 其他，实时状态徽标）+ 右侧单区块面板 + 常驻底栏保存按钮；760×560 最小窗口无横向滚动。
- 前端测试与 lint 体系：Vitest + Testing Library 66 用例（含 1.0.9 回归点的变异验证）、ESLint flat config；CI 新增 vitest / eslint / clippy / fmt / audit 质量门。
- 开源仓库配套：LICENSE（MIT）、SECURITY.md、CONTRIBUTING.md、CHANGELOG.md、.gitattributes、.editorconfig、rust-toolchain.toml。

## [1.0.11] - 2026-11-23

纯文档收口版本，**二进制与 1.0.10 相同**，功能无变化。

### 文档

- 热键统一为 `Ctrl+Shift+H` 显示 / 隐藏浮层（此前为 `Ctrl+Shift+Space`）。代码、README、前端徽标文案、托盘 tooltip 已同步，发布说明与两份设计文档补齐同一口径。
- 验收清单补用例：「浮层随面试自动显示、结束自动收起」，并写明空闲状态下手动开关浮层不受影响。

## [1.0.10] - 2026-11-23

### 修复

- 麦克风 WASAPI 瞬时缓冲欠载不再触发全量重建采集流。此前一次抖动会走完整重连路径（实测 41 次 underrun 引发 43 次重连），每次重连丢 0.3–2.3 秒音频，15 秒内 5 次失败还会把整个会话判死。
- 取消模型下载不再按 ERROR 记成失败，日志与界面状态不再误报。
- 模型接口卡片改为三段式协议切换（分段控件 + 卡内「预设 / 凭据」分组），切协议时不再和其它字段互相挤压。

### 新增

- 开始面试时浮层自动显示、结束自动收起；安装后浮层默认可见。

## [1.0.9] - 2026-11-23

对照 `pluely` / `natively-cluely` / `Freely` 三个参考项目的第六轮复查，修复 30 处缺陷。

### 修复

- 切换默认输出设备被误判为通道失败，15 秒内 5 次即把系统声通道判死；现在设备切换不再计入失败计数。
- loopback 缓冲零余量导致偶发漏字，缓冲区预留余量。
- 系统声通道失败会连带吞掉全局热键；托盘与热键解耦，一路出错不再拖垮另一路。
- 流式过程中点「清空」后，已在途的答案会整段复活；清空后不再接收上一轮的流式增量。
- 网关忽略 `stream: true`（直接返回整包）时浮层停在空白答案，现在按非流式响应兜底渲染。
- 非 2xx 错误体读取不可取消，最坏拖满 60 秒占住整条作答链路；错误体读取与作答共用取消语义。
- WAV 格式不符等 8 类 `invalid input` 只报原始错误串，现在给中文说明，并且不再因此丢弃健康 worker。
- 模型就绪判定只比文件大小，截断或损坏的同尺寸文件会被当成可用；改为校验 sha256。
- 206 区间校验过严，导致 239 MB 的下载进度整份作废；放宽为按实际返回区间推进。
- 11 个同步命令跑在 UI 主线程上，现在移到运行时线程执行，避免阻塞界面。
- 「本场已答 N 题」计数到 11 题后不再增长，修正计数上限。
- 空问题会误触发自动作答，问题判定前先剔空空串。
- 浮层答案与转录面板未做 memo 化导致无关重渲染，补齐。
- DOCX 解压炸弹：XML 解压后上限 32 MB，超出即拒绝解析。

### 无障碍

- 转录面板可键盘聚焦。
- toast 挂 `live region`，读屏能播报状态变化。

## [1.0.8] - 2026-11-21

### 变更

- 配置页与会话复盘页整体 UI 重构，采用 AIGX 设计语言：
  - 统一设计令牌（字号 / 间距 / 圆角 / 阴影 / 控件高度），不再逐处写魔法值。
  - 六张卡片统一为「卡片头 + 卡体」结构。
  - 状态反馈按语义分色（成功 / 提示 / 警告 / 错误）。
  - 可访问性：焦点环、标签页与面板的读屏关联。
  - 复盘页补转录图例。

## [1.0.7] 及更早

见 git history。
