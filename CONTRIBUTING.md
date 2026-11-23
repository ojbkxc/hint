# 参与开发

hint 是 Windows 桌面应用（Tauri 2 + Rust + React 19），核心业务逻辑放在不依赖 Tauri 的 `crates/hint-core`，UI 壳在 `src-tauri` 与 `src`。改动尽量落到 `hint-core`，壳层只做编排。

## 环境要求

- Windows 10/11 x64
- Visual Studio Build Tools 2022（C++ 桌面开发工作负载）或等价的 MSVC 链接器
- Rust stable，MSVC 工具链：`rustup default stable-x86_64-pc-windows-msvc`（仓库根有 `rust-toolchain.toml`，会补齐 `rustfmt`、`clippy` 与 `x86_64-pc-windows-msvc` target）
- Node.js 22+（Vite 8 要求）
- 首次编译 `hint-stt` 需要联网：`sherpa-onnx-sys` 会从 GitHub Release 下载预编译静态库

## 构建

```bash
npm ci
npm run tauri build
```

完整安装包在 GitHub Actions 的 `windows-latest` 上构建（`build.yml` 跑检查与测试并上传产物，`release.yml` 由 `v*` 标签触发发布），本地构建只用于自己验证。

## 只做前端与平台无关的检查

不需要 MSVC、也不需要下载静态库时，跑这一组：

```bash
npx tsc --noEmit
npm run build
cargo test -p hint-core -p hint-store -p hint-audio
cargo test -p hint-stt --no-default-features
```

Rust 侧的完整检查与测试以 `build.yml`（windows-latest + MSVC）为准。`cargo test -p hint` 在本地与 CI 都因 `sherpa-onnx-sys` 静态库无法链接 test harness 而不可用，所以壳层里能测的逻辑请下沉到 `hint-core`。

## 代码约定

- Rust：提交前跑 `cargo fmt`，并过一遍 `cargo clippy`（不要留下新告警）。
- 前端：`npx tsc --noEmit` 必须干净；类型错误不要用 `any` 或 `@ts-ignore` 压掉。
- 缩进与换行按 `.editorconfig`：Rust 4 空格、前端 2 空格，统一 LF 入库（`.gitattributes` 已把源码与文本类文件锁定为 LF，`*.ps1` / `*.bat` / `*.cmd` 保持 CRLF）。
- Markdown 行尾双空格用于换行，`.editorconfig` 对 `[*.md]` 特意关掉了 `trim_trailing_whitespace`，别手动删。
- 前端 `.tsx` / `.json` 用编辑器的精确替换（局部 patch）来改，不要整文件重写——这些文件里有大量手工对齐的 JSX 与中文文案，整写会把无关行一起搅进 diff，review 时看不出你真正改了什么。
- 中英文文案同步改，别只改一侧。

## 分支与提交

- 直接在 `main` 上改，不另开长期分支。
- 提交信息写**怎么坏**（根因），而不是「已完成」。例如写「浮层收起后仍抢焦点：不可聚焦窗口在 hide 前没有清焦点」，而不是写「修复浮层焦点问题」。这样看 log 的人能知道当时是在修什么因，而不是知道你做了什么动作。
- 一个提交讲清一件事；顺手重构单独提交，别混进缺陷修复里。

## 许可与依赖

- 本项目以 MIT 发布（见 `LICENSE`）。
- **不要引入 GPL / AGPL 依赖**。架构参考过 `pluely`（GPL-3.0）、`natively-cluely`、`Freely` 等项目，但按 README「参考项目」一节的声明，只借鉴思路、不复用其业务代码；直接把这类项目的代码或强 copyleft 依赖搬进来会和 MIT 冲突。
- 新增依赖前先看它的 license 与体积；Rust 侧优先选已有依赖能做到的方案。
