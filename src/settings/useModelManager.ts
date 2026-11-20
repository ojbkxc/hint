import { useCallback, useEffect, useRef, useState } from "react";
import { isRecord } from "../lib/data";
import { safeInvoke, safeListen } from "../lib/tauri";
import {
  INITIAL_MODEL_VIEW,
  applyProgress,
  normalizeModelStatus,
  type ModelView,
} from "./modelTypes";

const MODEL_EVENT = "hint://model-progress";

/**
 * 语音模型状态 + 下载编排。
 * prepare_model 是 async 命令且内部 spawn_blocking，可能阻塞数十秒：
 * 点击后先本地置 downloading，不等 Promise 返回。
 */
export function useModelManager() {
  const [view, setView] = useState<ModelView>(INITIAL_MODEL_VIEW);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const disposed = useRef(false);
  const fetching = useRef(false);

  const refresh = useCallback(async () => {
    if (fetching.current) return;
    fetching.current = true;
    const result = await safeInvoke<unknown>("model_status");
    fetching.current = false;
    if (disposed.current) return;
    if (!result.ok) {
      setView((current) =>
        current.phase === "downloading"
          ? current
          : { ...current, phase: "error", error: `读取模型状态失败：${result.error}` },
      );
      return;
    }
    setView(normalizeModelStatus(result.data));
  }, []);

  useEffect(() => {
    disposed.current = false;
    void refresh();
    return () => {
      disposed.current = true;
    };
  }, [refresh]);

  // 进度事件只覆盖展示态；真正的 phase 落定由命令返回后的 refresh 决定。
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    void safeListen<unknown>(MODEL_EVENT, (payload) => {
      if (!active || !isRecord(payload)) return;
      setView((current) => applyProgress(current, payload));
    }).then((fn) => {
      if (!active) {
        fn?.();
        return;
      }
      unlisten = fn;
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  const download = useCallback(async () => {
    // 前端在 ready 态复用同一个按钮做「校验」；校验只读，不需要二次确认。
    setNotice("");
    setBusy(true);
    // 先切到下载中，避免 spawn_blocking 期间按钮仍可点。
    setView((current) => ({
      ...current,
      phase: "downloading",
      error: "",
      step: current.phase === "ready" ? "verifying" : "model",
      resource: "",
      source: "",
      completedBytes: current.completedBytes,
    }));

    const result = await safeInvoke<boolean>("prepare_model");
    setBusy(false);
    if (disposed.current) return;

    if (!result.ok) {
      // 用户主动取消时后端返回错误，期间 phase 可能仍是 downloading；
      // 用最新状态覆盖，并给出中文提示。
      await refresh();
      if (disposed.current) return;
      setView((current) => ({
        ...current,
        phase: current.phase === "ready" ? "ready" : current.phase,
        error: current.phase === "ready" ? "" : `模型下载失败：${result.error}`,
      }));
      setNotice(`模型下载未完成：${result.error}`);
      return;
    }

    // 后端返回 false 表示用户主动取消（不是失败）；已下载部分会保留。
    if (result.data === false) {
      setNotice("已取消语音模型下载，已下载部分会保留，可稍后继续。");
      await refresh();
      return;
    }

    setNotice("模型文件检查完成，可以开始面试。");
    await refresh();
  }, [refresh]);

  const cancel = useCallback(async () => {
    setNotice("");
    const result = await safeInvoke<boolean>("cancel_prepare");
    if (disposed.current) return;
    if (!result.ok) {
      setNotice(`取消下载失败：${result.error}`);
      return;
    }
    if (result.data === false) {
      setNotice("当前没有正在进行的下载。");
      await refresh();
      return;
    }
    setNotice("已请求取消下载，稍后状态会自动更新。");
    // 给下载线程一点时间收尾，再回读真实状态。
    window.setTimeout(() => void refresh(), 1500);
  }, [refresh]);

  const remove = useCallback(async () => {
    const confirmed = window.confirm(
      "确定删除语音模型吗？\n\n模型约 241MB，删除后语音识别不可用，需要重新完整下载才能开始面试。",
    );
    if (!confirmed) return;
    setNotice("");
    setBusy(true);
    const result = await safeInvoke<unknown>("remove_model");
    setBusy(false);
    if (disposed.current) return;
    if (!result.ok) {
      setNotice(`删除模型失败：${result.error}`);
      return;
    }
    setNotice("语音模型已删除。");
    await refresh();
  }, [refresh]);

  return { view, busy, notice, download, cancel, remove, refresh };
}