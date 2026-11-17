# hint

> 面试实时提词器。本地语音识别，双通道区分「面试官」与「我」，多协议大模型流式作答。

hint 是一个 Windows 桌面小工具：它同时监听你的麦克风和系统播放的声音，把两路音频分别转成文字，判断面试官提出的问题，并把你事先准备好的简历与项目资料作为上下文，生成一段可以直接照着说的回答。

它只服务你本人：窗口可见、可随时关闭，不做屏幕共享隐身，也不做任何规避检测。

## 当前进度

M0 骨架已完成：

- Tauri 2 + React 19 + TypeScript
- 托盘菜单（打开设置 / 显示浮层 / 退出）
- 全局热键 `Ctrl+Shift+Space` 显示或隐藏浮层
- 透明置顶浮层（无边框、可拖动、可缩放）
- 设置页：三种 LLM 协议、简历与项目资料、回答风格、触发方式
- API Key 写入 Windows 凭据管理器（`keyring`），不落明文
- 一次非流式 LLM 调用（连接测试）

## 路线图

| 阶段 | 内容 |
|---|---|
| M1 | 三协议流式输出：OpenAI 兼容 / Anthropic Messages / Gemini |
| M2 | 双通道采集：cpal 麦克风 + WASAPI loopback，16k 重采样，电平显示 |
| M3 | 本地 STT：sherpa-onnx 静态链接 + SenseVoice Small INT8 + Silero VAD |
| M4 | 问题判定、自动/按键生成、麦克风回声过滤、会话存储 |
| M5 | 截图区域问答（架构已预留） |
| M6 | NSIS 安装包与分发 |

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
├── crates/hint-core/        # 不依赖 UI 的核心逻辑
│   └── src/
│       ├── settings.rs      # 配置结构与提示词组装
│       ├── secret.rs        # Windows 凭据管理器封装
│       ├── llm.rs           # 三协议 LLM 调用
│       └── error.rs
├── src-tauri/               # Tauri 壳：窗口、托盘、热键、命令
├── src/                     # React：设置页与浮层
├── .github/workflows/       # 云端构建
└── Cargo.toml               # Cargo workspace
```

## 构建

本项目在 GitHub Actions 的 `windows-latest` 上构建，本机不需要安装 Visual Studio 或 Windows SDK。

推送到 `main` 后，Actions 会产出：

- `hint.exe`
- NSIS 安装包

也可以在本地构建，但需要：

1. Visual Studio Build Tools 2022（C++ 桌面开发工作负载）
2. Rust MSVC 工具链：`rustup default stable-x86_64-pc-windows-msvc`
3. Node.js 20+

```bash
npm install
npm run tauri build
```

## 隐私

- 简历、项目资料、会话记录只存在本机。
- API Key 存在 Windows 凭据管理器，不写配置文件。
- 原始音频不落盘（M2 起生效）。
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
