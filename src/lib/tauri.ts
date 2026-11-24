import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type InvokeResult<T> =
  | { ok: true; data: T }
  | { ok: false; error: string };

/**
 * 是否运行在 Tauri WebView 内。
 *
 * **不能只看一次**：Tauri 的 IPC 注入脚本与本应用的 JS 并行加载，首屏
 * effect 跑起来时 `__TAURI_INTERNALS__` 有时还没挂到 window 上。旧实现
 * 直接返回 false → `safeInvoke` 降级成「当前不在 Tauri 环境中运行」→
 * 请求根本没到后端（日志零痕迹）→ 设置页表单停在 EMPTY_SETTINGS，
 * 用户此时点「保存全部配置」就把空值写回 settings.json 覆盖真实配置。
 * （真机实测：首次启动 Base URL 为空，页面重载后才正常。）
 */
/**
 * 等待 IPC 注入脚本挂载的窗口（毫秒）。
 *
 * Tauri 的 IPC 注入脚本与本应用的 JS 并行加载，首屏 effect 跑起来时
 * `__TAURI_INTERNALS__` 有时还没挂到 window 上。旧实现只看一次 → 返回 false
 * → `safeInvoke` 降级成「当前不在 Tauri 环境中运行」→ 请求根本没到后端
 * （日志零痕迹）→ 设置页表单停在 EMPTY_SETTINGS，用户此时点「保存全部配置」
 * 就把空值写回 settings.json 覆盖真实配置。
 * （真机实测：首次启动 Base URL 为空，页面重载后才正常。）
 *
 * 单元测试里大量用 fake timers，真实 `setTimeout` 不推进，等待窗口会让
 * 「降级分支」用例白等并超时——所以在 Vitest 环境下取 0（回到「只看一次」
 * 的语义），生产构建取 1500。用 `import.meta.env` 而不是运行时开关，
 * 因为模块实例可能被 `vi.mock` 替换，运行时变量跨不过去。
 */
const IPC_WAIT_MS =
  typeof import.meta !== "undefined" && import.meta.env?.VITEST ? 0 : 1500;
const IPC_POLL_MS = 50;

function hasIpc(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

/** 判断是否在 Tauri 内；必要时短暂等待注入完成。 */
export async function isTauriAsync(): Promise<boolean> {
  if (hasIpc()) return true;
  const deadline = Date.now() + IPC_WAIT_MS;
  while (Date.now() < deadline) {
    await sleep(IPC_POLL_MS);
    if (hasIpc()) return true;
  }
  return hasIpc();
}

/** 同步版本，供只做「是否支持」提示的调用方使用（不等待）。 */
export function isTauri(): boolean {
  return hasIpc();
}

export function describeError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    const json = JSON.stringify(error);
    return typeof json === "string" ? json : String(error);
  } catch {
    return String(error);
  }
}

/** 所有 invoke 统一走这里，永远不抛异常，失败时返回中文可读的 error。 */
export async function safeInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<InvokeResult<T>> {
  // 用带等待的版本：首屏 effect 可能比 IPC 注入脚本更早执行。
  if (!(await isTauriAsync())) {
    return { ok: false, error: "当前不在 Tauri 环境中运行，后端命令不可用" };
  }
  try {
    const data = await invoke<T>(command, args);
    return { ok: true, data };
  } catch (error) {
    return { ok: false, error: describeError(error) };
  }
}

/** 监听后端事件；事件不存在或注册失败时返回 null，不抛异常。 */
export async function safeListen<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn | null> {
  if (!(await isTauriAsync())) return null;
  try {
    return await listen<T>(event, (incoming) => handler(incoming.payload));
  } catch {
    return null;
  }
}