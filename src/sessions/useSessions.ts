import { useCallback, useEffect, useState } from "react";
import { safeInvoke } from "../lib/tauri";
import {
  downloadMarkdown,
  normalizeMarkdown,
  normalizeSessionDetail,
  normalizeSessions,
  type SessionDetail,
  type SessionSummary,
} from "./types";

export function useSessions(enabled: boolean) {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [selectedId, setSelectedId] = useState("");
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState("");
  const [notice, setNotice] = useState("");

  const refresh = useCallback(async () => {
    setLoading(true);
    setError("");
    const result = await safeInvoke<unknown>("list_sessions");
    if (!result.ok) {
      setSessions([]);
      setError(`读取会话失败：${result.error}`);
    } else {
      setSessions(normalizeSessions(result.data));
    }
    setLoading(false);
  }, []);

  useEffect(() => {
    if (enabled) void refresh();
  }, [enabled, refresh]);

  const open = useCallback(async (id: string) => {
    setSelectedId(id);
    setDetail(null);
    setDetailError("");
    setNotice("");
    setDetailLoading(true);
    const result = await safeInvoke<unknown>("session_detail", { id });
    if (!result.ok) {
      setDetailError(`加载会话失败：${result.error}`);
    } else {
      setDetail(normalizeSessionDetail(result.data, id));
    }
    setDetailLoading(false);
  }, []);

  const exportMarkdown = useCallback(async () => {
    if (!selectedId) return;
    setNotice("");
    const result = await safeInvoke<unknown>("export_session_markdown", { id: selectedId });
    if (!result.ok) {
      setNotice(`导出失败：${result.error}`);
      return;
    }
    const markdown = normalizeMarkdown(result.data);
    if (!markdown.trim()) {
      setNotice("导出失败：后端返回内容为空");
      return;
    }
    downloadMarkdown(`${safeFilename(selectedId)}.md`, markdown);
    setNotice("已导出 Markdown 文件");
  }, [selectedId]);

  const remove = useCallback(async () => {
    if (!selectedId) return;
    const confirmed = window.confirm("确定删除这条会话吗？删除后不可恢复。");
    if (!confirmed) return;
    setNotice("");
    const result = await safeInvoke<unknown>("delete_session", { id: selectedId });
    if (!result.ok) {
      setNotice(`删除失败：${result.error}`);
      return;
    }
    setNotice("会话已删除");
    setDetail(null);
    setSelectedId("");
    await refresh();
  }, [selectedId, refresh]);

  const removeAll = useCallback(async () => {
    const first = window.confirm(
      "确定清空全部会话记录吗？所有转录与问答都会被永久删除，无法恢复。",
    );
    if (!first) return;
    const second = window.confirm("再次确认：清空全部会话记录？此操作不可撤销。");
    if (!second) return;
    setNotice("");
    const result = await safeInvoke<unknown>("delete_all_sessions");
    if (!result.ok) {
      setNotice(`清空失败：${result.error}（若后端尚未提供该命令，请单条删除）`);
      return;
    }
    setNotice("全部会话记录已清空");
    setDetail(null);
    setSelectedId("");
    await refresh();
  }, [refresh]);

  return {
    sessions,
    loading,
    error,
    selectedId,
    detail,
    detailLoading,
    detailError,
    notice,
    refresh,
    open,
    exportMarkdown,
    remove,
    removeAll,
  };
}

function safeFilename(id: string): string {
  const cleaned = id.replace(/[\\/:*?"<>|\s]+/g, "-").replace(/^-+|-+$/g, "");
  return cleaned || "hint-session";
}