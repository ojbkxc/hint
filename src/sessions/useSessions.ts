import { useCallback, useEffect, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { safeInvoke } from "../lib/tauri";
import {
  normalizeSessionDetail,
  normalizeSessions,
  type SessionDetail,
  type SessionSummary,
} from "./types";

export function useSessions(enabled: boolean) {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [loading, setLoading] = useState(false);
  /**
   * 搜索词（按转录内容搜）。
   *
   * 空串 = 不过滤。命中集合为 `null` 表示「没有在搜索」，与「搜索了但没命中」
   * （空数组）必须区分——否则搜不到结果时界面会显示全部会话，看起来像没搜。
   */
  const [query, setQuery] = useState("");
  const [matchedIds, setMatchedIds] = useState<string[] | null>(null);
  const [searching, setSearching] = useState(false);
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

  // 搜索：防抖 250ms。每次按键都打一次后端没有意义，而 250ms 用户感觉不到延迟。
  useEffect(() => {
    const trimmed = query.trim();
    if (trimmed === "") {
      setMatchedIds(null);
      setSearching(false);
      return;
    }
    setSearching(true);
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        const result = await safeInvoke<string[]>("search_sessions", { query: trimmed });
        if (cancelled) return;
        if (result.ok && Array.isArray(result.data)) {
          setMatchedIds(result.data.map((id) => String(id)));
        } else {
          // 搜索失败不要把列表清空——用户至少还能按时间翻。
          setMatchedIds(null);
          setError(`搜索失败：${result.ok ? "返回格式异常" : result.error}`);
        }
        setSearching(false);
      })();
    }, 250);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query]);

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

    // 让用户先选保存位置，再由后端直接写盘。
    //
    // 为什么不用浏览器 Blob 下载（原实现）：真机验证过，WebView2 里
    // `a.download` 会走完「开始 → completed」，但文件在用户下载目录、
    // AppData、Temp 里**全都找不到**，而界面已经提示「已导出」——用户以为
    // 存好了，实际什么都没有。改由后端写盘，成败都是确定的。
    let target: string | null = null;
    try {
      target = await save({
        title: "导出面试记录",
        defaultPath: `${safeFilename(selectedId)}.md`,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
    } catch (error) {
      setNotice(`导出失败：无法打开保存对话框（${String(error)}）`);
      return;
    }
    // 用户取消：不提示成功，也不报错。
    if (!target) return;

    const result = await safeInvoke("export_session_file", { id: selectedId, path: target });
    if (!result.ok) {
      setNotice(`导出失败：${stripPrefix(result.error, "导出失败：")}`);
      return;
    }
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

  /**
   * 保存会话备注（空串 = 清除，回落到时间戳）。
   *
   * 需要同时更新两处，否则界面会不一致：
   * - `sessions`：列表标题优先取 note，不改列表就还是旧标题；
   * - `detail`：详情页的输入框绑的是它，不改会被 useEffect 回滚成旧值。
   *
   * 失败时**抛出**而不是吞掉：输入框要把原因显示出来（保存失败静默回滚会让用户
   * 以为已经存上了）。
   */
  const rename = useCallback(async (id: string, note: string) => {
    const result = await safeInvoke("rename_session", { id, note });
    if (!result.ok) {
      throw new Error(result.error || "保存备注失败");
    }
    const trimmed = note.trim();
    setSessions((prev) =>
      prev.map((item) => (item.id === id ? { ...item, note: trimmed } : item)),
    );
    setDetail((prev) =>
      prev && prev.summary.id === id
        ? { ...prev, summary: { ...prev.summary, note: trimmed } }
        : prev,
    );
  }, []);

  /**
   * 界面上实际显示的会话：没搜索时是全部，搜索时按命中 id 过滤（保持原顺序）。
   *
   * 用 `matchedIds === null` 判断「不在搜索」，而不是「命中集为空」——
   * 后者代表搜了但没结果，界面要显示「没有匹配的会话」而不是退回全部列表。
   */
  const visibleSessions =
    matchedIds === null
      ? sessions
      : sessions.filter((session) => matchedIds.includes(session.id));

  return {
    sessions: visibleSessions,
    /** 会话总数（不受搜索影响），用于「共 N 条」这类文案。 */
    totalCount: sessions.length,
    query,
    setQuery,
    searching,
    /** 是否处于搜索状态（区分「没搜」与「搜了没结果」）。 */
    isSearching: matchedIds !== null,
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
    rename,
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
