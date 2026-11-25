import { Component, type ErrorInfo, type ReactNode } from "react";
import { safeInvoke } from "./lib/tauri";

type Props = {
  children: ReactNode;
  /** 出错时显示在哪：决定给用户的下一步动作。 */
  surface: "overlay" | "dashboard" | "capture";
};

type State = { error: Error | null };

/**
 * 渲染异常兜底。
 *
 * 为什么需要：React 18 的默认行为是「渲染期抛错就卸载整棵树」——
 * 实测过一次（用外部脚本改了 React 管理的节点，破坏了虚拟 DOM 假设），
 * 结果整个浮层变成一片空白，用户看起来就是「程序死了」，没有任何提示，
 * 也点不动任何按钮。真实场景里浏览器翻译扩展、输入法插件、其他 DOM 干扰
 * 都可能触发同类错误。
 *
 * 兜底原则：
 * - **不让界面变空白**：至少显示一句人话 + 一个能自救的按钮；
 * - 浮层额外给「隐藏浮层」出口（面试中卡住时能立刻让开视线）；
 * - 错误的 message 只截断显示，不整段铺开（可能很长且含路径）。
 */
export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // 控制台留完整栈：用户截图给我们时至少能看到一行摘要。
    console.error("[hint] 界面渲染异常", error, info.componentStack);
  }

  private reload = () => {
    this.setState({ error: null });
    // 重载当前窗口：比试图「局部恢复」可靠——出错后组件状态已不可信。
    window.location.reload();
  };

  private hideOverlay = async () => {
    try {
      // 静态导入即可：tauri.ts 不导入本模块，没有循环依赖。
      // （曾用 await import() 想按需加载，但 tauri.ts 本身被多处静态导入，
      //   Vite 会报 INEFFECTIVE_DYNAMIC_IMPORT：拆不出 chunk，纯属噪声。）
      await safeInvoke("hide_overlay");
    } catch {
      // 隐藏失败也不能再抛：这里已经在异常路径上。
    }
  };

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    const what =
      this.props.surface === "overlay"
        ? "浮层渲染出错"
        : this.props.surface === "capture"
          ? "选区窗口渲染出错"
          : "设置页渲染出错";

    return (
      <div className="crash-fallback" role="alert">
        <p className="crash-title">{what}</p>
        <p className="crash-detail">{String(error.message ?? error).slice(0, 160)}</p>
        <div className="crash-actions">
          <button type="button" className="mini" onClick={this.reload}>
            重新加载
          </button>
          {this.props.surface === "overlay" && (
            <button type="button" className="mini ghost" onClick={() => void this.hideOverlay()}>
              隐藏浮层
            </button>
          )}
        </div>
      </div>
    );
  }
}
