import { useCallback, useEffect, useRef, useState } from "react";
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
  // 详情请求序号：快速连点两条会话时，只接受最后一次的结果。
  const detailRequest = useRef(0);
  // 列表请求序号：切 Tab 与手动刷新可能并发，旧响应不能覆盖新列表。
  const listRequest = useRef(0);

  const refresh = useCallback(async () => {
    const requestId = ++listRequest.current;
    setLoading(true);
    setError("");
    const result = await safeInvoke<unknown>("list_sessions");
    if (listRequest.current !== requestId) return;
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
    const requestId = ++detailRequest.current;
    setSelectedId(id);
    setDetail(null);
    setDetailError("");
    setNotice("");
    setDetailLoading(true);
    const result = await safeInvoke<unknown>("session_detail", { id });
    // 期间用户又点了别的会话：旧响应不能覆盖新选择。
    if (detailRequest.current !== requestId) return;
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
      // 后端错误已带「导出失败：」前缀，剥掉避免双重前缀。
      setNotice(`导出失败：${stripPrefix(result.error, "导出失败：")}`);
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
    const current = sessions.find((item) => item.id === selectedId);
    const label = current?.startedAt ? `「${current.startedAt}」` : "这条会话";
    const confirmed = window.confirm(
      `确定删除${label}吗？\n它的转录与问答会被永久删除，无法恢复。`,
    );
    if (!confirmed) return;
    setNotice("");
    const result = await safeInvoke<unknown>("delete_session", { id: selectedId });
    if (!result.ok) {
      // 后端错误是「删除会话失败：…」，这里的前缀是「删除失败：」。
      setNotice(`删除失败：${stripPrefix(result.error, "删除会话失败：")}`);
      return;
    }
    setNotice("会话已删除");
    setDetail(null);
    setSelectedId("");
    await refresh();
  }, [selectedId, sessions, refresh]);

  const removeAll = useCallback(async () => {
    const count = sessions.length;
    if (count === 0) return;
    const first = window.confirm(
      `确定清空全部 ${count} 条会话记录吗？\n所有转录与问答都会被永久删除，无法恢复。`,
    );
    if (!first) return;
    const second = window.confirm(
      `再次确认：将永久删除 ${count} 条会话记录，删除后无法撤销。`,
    );
    if (!second) return;
    setNotice("");
    const result = await safeInvoke<unknown>("delete_all_sessions");
    if (!result.ok) {
      setNotice(`清空失败：${result.error}`);
      return;
    }
    setNotice(`全部 ${count} 条会话记录已清空`);
    setDetail(null);
    setSelectedId("");
    await refresh();
  }, [sessions, refresh]);

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

/** 后端错误里已含同名中文前缀时剥掉，避免「导出失败：导出失败：…」。 */
function stripPrefix(text: string, prefix: string): string {
  const stripped = text.startsWith(prefix) ? text.slice(prefix.length) : text;
  return stripped.trim() || text.trim();
}
