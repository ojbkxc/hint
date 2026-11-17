# hint

> 面试实时提词器。本地语音识别，双通道区分「面试官」与「我」，多协议大模型流式作答。

hint 是一个 Windows 桌面小工具：它同时监听你的麦克风和系统播放的声音，把两路音频分别转成文字，判断面试官提出的问题，并把你事先准备好的简历与项目资料作为上下文，生成一段可以直接照着说的回答。

它只服务你本人：窗口可见、可随时关闭，不做屏幕共享隐身，也不做任何规避检测。

## 当前进度

M0–M4 主链路已实现，M5 仅预留窗口与命令：

- Tauri 2 + React 19 + TypeScript，托盘、透明置顶浮层、全局热键
- 设置页：三种 LLM 协议、简历与项目资料、回答风格、触发方式、音频设备
- API Key 只写入 Windows 凭据管理器（`keyring`），不落配置文件
- 三协议流式 LLM（OpenAI 兼容 / Anthropic Messages / Gemini），支持取消与重新生成
- 双通道采集：cpal 麦克风 + WASAPI loopback，16k 单声道重采样与电平上报
- 本地 STT：sherpa-onnx 2.0.1 + SenseVoice Small INT8 + Silero VAD，模型按需下载（断点续传、sha256 校验）
- 问题判定、回声过滤、SQLite 会话留存、Markdown 导出与会话复盘页
- 设置页内置「语音模型」与「面试会话」卡片：下载/取消/删除模型，开始/结束面试
- M5 截图问答：选区窗口、`start_capture` / `cancel_capture` / `finish_capture` 已预留，返回「尚未启用」

## 路线图

| 阶段 | 内容 | 状态 |
|---|---|---|
| M0 | Tauri 骨架、托盘、热键、浮层、设置、keyring | 完成 |
| M1 | 三协议流式输出 | 完成 |
| M2 | 双通道采集与 16k 重采样 | 完成 |
| M3 | 本地 STT（sherpa-onnx + SenseVoice + Silero VAD） | 完成 |
| M4 | 问题判定、回声过滤、会话存储 | 完成 |
| M5 | 截图区域问答 | 仅预留 |
| M6 | NSIS 安装包与分发 | CI 构建，待真机验收 |

## 技术栈

- **壳**：Tauri 2（Rust 后端 + 系统 WebView2）
- **前端**：React 19 + TypeScript + Vite
- **核心逻辑**：`crates/hint-core`，不依赖 Tauri，便于测试与日后换壳
- **LLM**：`reqwest` + 三种协议
- **凭证**：`keyring`（Windows Credential Manager）
- **STT（M3 起）**：`sherpa-onnx` 2.x + SenseVoice Small INT8 + Silero VAD

## 目录结构

```
hint/
├── crates/
│   ├── hint-core/           # 不依赖 UI：配置、凭证、LLM、问题判定/回声
│   ├── hint-audio/          # cpal 麦克风 + WASAPI loopback、重采样、电平
│   ├── hint-stt/            # 模型下载、sidecar worker、SenseVoice + Silero VAD
│   └── hint-store/          # SQLite 会话、段落、答案与 Markdown 导出
├── src-tauri/               # Tauri 壳：窗口、托盘、热键、命令、会话编排
├── src/                     # React：设置页、浮层、复盘页、截图选区
├── .github/workflows/       # 云端构建
└── Cargo.toml               # Cargo workspace（release profile 在此）
```

## 构建与测试

完整安装包在 GitHub Actions 的 `windows-latest` 上构建（`.github/workflows/build.yml`），
CI 使用 `x86_64-pc-windows-msvc` 工具链，产出 `hint.exe` 与 NSIS 安装包。

本地构建需要：

1. Visual Studio Build Tools 2022（C++ 桌面开发工作负载）或等价的 MSVC 链接器
2. Rust MSVC 工具链：`rustup default stable-x86_64-pc-windows-msvc`
3. Node.js 22+（Vite 8 要求）
4. 首次编译 `hint-stt` 需要联网：`sherpa-onnx-sys` 会从 GitHub Release 下载预编译静态库

```bash
npm ci
npm run tauri build
```

只做前端与平台无关部分检查（不需要 MSVC / 网络下载静态库）：

```bash
npx tsc --noEmit
npm run build
cargo test -p hint-core -p hint-store
cargo test -p hint-audio
cargo test -p hint-stt --no-default-features
```

注意：若 Rust 宿主是 `x86_64-pc-windows-gnu` 且 PATH 中没有 MinGW 的 `gcc.exe` / `windres.exe`，
`cargo test -p hint-stt`（默认引擎）与 `cargo check -p hint` 的 Windows 资源嵌入步骤会失败；
本机实测即由此失败。CI 使用 MSVC 工具链（含 `ilammy/msvc-dev-cmd`），不受影响。
默认引擎的完整测试与打包请以 CI 的 MSVC 结果为准。

## 产物

GitHub Actions 每次构建会在 run 页面提供 artifact：

| 文件 | 体积（实测） |
|---|---|
| `hint.exe` | 约 27 MB（含静态链接的 sherpa-onnx + onnxruntime） |
| `hint_0.1.0_x64-setup.exe`（NSIS） | 约 7.8 MB |

安装包体积小于 exe 是因为 NSIS 使用 LZMA 压缩。语音模型不在包内，首次使用时按需下载约 241 MB。

## 隐私

- 简历、项目资料、会话记录只存在本机。
- API Key 存在 Windows 凭据管理器，不写配置文件，也不进日志。
- 原始音频不落盘：采集 → 分段 → WAV 内存 → sidecar 推理，全程无文件写入。
- 下载日志只记录经过脱敏的 URL（去掉 query 与临时签名）。
- 除了你自己配置的模型接口，不向任何第三方发送数据。

## 边界

- 不实现「对屏幕共享隐身」「进程伪装」「规避检测」一类功能。
- 只支持 Windows。macOS 的音频采集与权限模型差异过大，不在计划内。

## 参考项目

架构与实现参考了以下开源项目，仅借鉴思路，不复用其业务代码：

- [pluely](https://github.com/iamsrikanthnani/pluely)：Tauri 2 + Rust 窗口与 WASAPI loopback
- [natively-cluely](https://github.com/natively-cluely/natively-cluely)：双通道说话人区分与回声过滤
- [cc-haha](https://github.com/NanmiCoder/cc-haha)：sherpa-onnx 模型下载与 sidecar 设计
- [Freely](https://github.com/KMalek101/Freely)：本地 Whisper 与 Provider 抽象
