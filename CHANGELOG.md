# 更新日志

本文件格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循语义化版本。

只记录每个版本对用户或开发者有实际影响的变化；更早的细节见 git history。

## [1.0.13] - 2026-11-24

### 修复

- **Anthropic 带图消息回归（1.0.12 引入）**：`anthropic_body` 带图分支做 collapsible_if 折叠时把「末条是 user 才 pop 并入」的角色检查整个删掉，末条为 assistant 时被错误弹掉、请求体少一条消息；CI 单测 `anthropic_body_merges_consecutive_roles_and_images` 拦下。补回角色条件。
- 依赖安全升级：zip 2.6.1→9.7.0（连带与 sherpa-onnx-sys 收敛为同一版本；feature `deflate` 改名 `deflate-flate2-zlib-rs`）、pdf-extract 0.9→0.12.1（带动 lopdf 0.36→0.42 栈溢出修复）。url（RUSTSEC-2026-0592）与 cookie（0594）为 tauri/wry 生态锁定的传递依赖，hint 的利用面不存在（不解析不可信 blob: URL、非 fork 的桌面进程），audit 豁免并注明理由。

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
