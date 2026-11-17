import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type InvokeResult<T> =
  | { ok: true; data: T }
  | { ok: false; error: string };

/** 是否运行在 Tauri WebView 内。纯浏览器预览时所有后端调用会优雅降级。 */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
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
  if (!isTauri()) {
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
  if (!isTauri()) return null;
  try {
    return await listen<T>(event, (incoming) => handler(incoming.payload));
  } catch {
    return null;
  }
}