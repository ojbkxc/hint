import "@testing-library/jest-dom/vitest";

/**
 * Tauri mock：按 @tauri-apps/api 实际读取的字段来伪造。
 *
 * - `src/lib/tauri.ts` 的 isTauri() 只检查 `"__TAURI_INTERNALS__" in window`；
 * - `@tauri-apps/api/core` 的 invoke() 调 `window.__TAURI_INTERNALS__.invoke(cmd, args, options)`；
 * - `@tauri-apps/api/event` 的 listen() 调 `invoke('plugin:event|listen', {...})`
 *   并用 `window.__TAURI_INTERNALS__.transformCallback(handler)` 注册回调，
 *   unlisten 会调 `window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener(event, eventId)`
 *   再 invoke('plugin:event|unlisten')。
 *
 * 这里提供最小可用的 invoke/transformCallback；具体命令的行为由各测试文件
 * 用 vi.mock("@tauri-apps/api/core") / vi.mock("@tauri-apps/api/event") 覆盖。
 */
type TauriHandler = (command: string, args: unknown) => unknown;

let nextEventId = 1;

function installTauriInternals(invokeImpl?: TauriHandler): void {
  const callbacks = new Map<number, (...rest: unknown[]) => void>();

  Object.defineProperty(window, "__TAURI_INTERNALS__", {
    configurable: true,
    writable: true,
    value: {
      // safeInvoke 的降级路径依赖这个字段存在与否；默认实现返回 null，
      // 单测里用 vi.mock("@tauri-apps/api/core") 拦截真实 invoke。
      invoke: (command: string, args: unknown) => {
        if (invokeImpl) return invokeImpl(command, args);
        return null;
      },
      transformCallback: (
        callback: (...rest: unknown[]) => void,
        once?: boolean,
      ) => {
        const id = nextEventId++;
        callbacks.set(id, callback);
        if (once) {
          const wrapped: typeof callback = (...rest) => {
            callbacks.delete(id);
            callback(...rest);
          };
          callbacks.set(id, wrapped);
        }
        return id;
      },
      unregisterCallback: (id: number) => {
        callbacks.delete(id);
      },
    },
  });

  Object.defineProperty(window, "__TAURI_EVENT_PLUGIN_INTERNALS__", {
    configurable: true,
    writable: true,
    value: {
      unregisterListener: () => undefined,
    },
  });
}

installTauriInternals();

// @testing-library/react 的 auto-cleanup 在 globals 模式下可能不会自动注册，
// 显式开启，避免用例间共享状态泄漏。
if (typeof afterEach === "function") {
  afterEach(() => {
    // jsdom 下清掉可能被 applyOverlayOpacity 设置的 CSS 变量。
    document.documentElement.style.removeProperty("--overlay-alpha");
  });
}
