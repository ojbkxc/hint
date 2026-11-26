import { deriveStatusTone, statusToneClass } from "../settings/statusTone";
import type { SessionDetail, SessionSummary } from "./types";

type Props = {
  /** 当前要显示的会话（已按搜索过滤）。 */
  sessions: SessionSummary[];
  /** 会话总数，不受搜索影响：用于「清空全部（N）」，避免搜索时显示成过滤后的数量。 */
  totalCount: number;
  query: string;
  onQueryChange: (next: string) => void;
  searching: boolean;
  /** 是否处于搜索状态：区分「没搜」与「搜了没结果」。 */
  isSearching: boolean;
  loading: boolean;
  error: string;
  selectedId: string;
  detail: SessionDetail | null;
  detailLoading: boolean;
  detailError: string;
  notice: string;
  onRefresh: () => void;
  onOpen: (id: string) => void;
  onExport: () => void;
  onDelete: () => void;
  onDeleteAll: () => void;
};

const SKELETON_KEYS = ["skeleton-1", "skeleton-2", "skeleton-3"];

export default function SessionReview(props: Props) {
  const {
    sessions,
    totalCount,
    query,
    onQueryChange,
    searching,
    isSearching,
    loading,
    error,
    selectedId,
    detail,
    detailLoading,
    detailError,
    notice,
    onRefresh,
    onOpen,
    onExport,
    onDelete,
    onDeleteAll,
  } = props;

  return (
    <div className="review-layout">
      <section className="card review-list">
        <div className="card-head">
          <h2>历史会话</h2>
          <div className="action-row">
            <button type="button" className="ghost" onClick={onRefresh} disabled={loading}>
              {loading ? "加载中…" : "刷新"}
            </button>
            <button
              type="button"
              className="danger small-danger"
              onClick={onDeleteAll}
              disabled={loading || totalCount === 0}
              title={
                totalCount > 0
                  ? `永久删除本机全部 ${totalCount} 条会话记录`
                  : "永久删除本机全部会话记录"
              }
            >
              {totalCount > 0 ? `清空全部（${totalCount}）` : "清空全部"}
            </button>
          </div>
        </div>

        {/* 按转录内容搜索：会话攒到几十条后，用户记得的是「问过 kafka 的那次」，
            不是「10 月 17 日 15:37 那次」。 */}
        <div className="session-search">
          <input
            type="search"
            className="form-input"
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder="搜索会话内容（如某道题、某个技术词）"
            aria-label="搜索会话内容"
          />
          {searching && <span className="session-search-state">搜索中…</span>}
          {!searching && isSearching && (
            <span className="session-search-state">
              {sessions.length > 0
                ? `匹配 ${sessions.length} 条`
                : "没有匹配的会话"}
            </span>
          )}
        </div>

        {error ? (
          <div className="card-body">
            <p className="error-text" role="alert">
              {error}
            </p>
          </div>
        ) : loading ? (
          <div className="card-body" aria-busy="true">
            <span className="visually-hidden">正在读取会话列表…</span>
            <ul className="session-items" aria-hidden="true">
              {SKELETON_KEYS.map((key) => (
                <li key={key}>
                  <div className="skeleton skeleton-text" />
                </li>
              ))}
            </ul>
          </div>
        ) : sessions.length === 0 ? (
          <div className="card-body">
            <EmptyState text="还没有会话记录。完成一次面试后，这里会列出可回看的转录与问答。" />
          </div>
        ) : (
          <div className="card-body">
            <ul className="session-items">
              {sessions.map((session) => {
                const title = session.note || session.startedAt || `会话 ${session.id}`;
                // meta 恒非空（模型缺失时已回落「模型未知」）；「时间未知」
                // 兜的其实是时长缺失——列表里没有时间列，时长才是第二段。
                const meta = [session.model || "模型未知", durationText(session) || "时间未知"].join(" · ");
                const active = session.id === selectedId;
                return (
                  <li key={session.id}>
                    <button
                      type="button"
                      className={`session-item${active ? " active" : ""}`}
                      onClick={() => onOpen(session.id)}
                      aria-current={active ? "true" : undefined}
                      title={`${title}${meta ? `（${meta}）` : ""}`}
                    >
                      <span className="session-title">{title}</span>
                      <span className="session-meta">{meta || "时间未知"}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        )}
      </section>

      <section className="card review-detail">
        <div className="card-head">
          <h2>转录与问答</h2>
          <div className="action-row">
            <button
              type="button"
              onClick={onExport}
              disabled={!selectedId || detailLoading}
            >
              导出 Markdown
            </button>
            <button
              type="button"
              className="danger"
              onClick={onDelete}
              disabled={!selectedId || detailLoading}
            >
              删除
            </button>
          </div>
        </div>

        <div className="card-body" aria-busy={detailLoading || undefined}>
          {notice && (
            <p className={statusToneClass(deriveStatusTone(notice))} role="status">
              {notice}
            </p>
          )}
          {detailError && (
            <p className="error-text" role="alert">
              {detailError}
            </p>
          )}

          {!selectedId ? (
            <EmptyState text="从左侧选择一条会话查看详情。" />
          ) : detailLoading ? (
            <>
              <span className="visually-hidden">正在加载会话…</span>
              {SKELETON_KEYS.map((key) => (
                <div key={key} className="skeleton skeleton-text" />
              ))}
            </>
          ) : detail ? (
            <SessionBody detail={detail} />
          ) : (
            !detailError && <EmptyState text="没有可显示的内容。" />
          )}
        </div>
      </section>
    </div>
  );
}

function EmptyState({ text }: { text: string }) {
  return (
    <div className="empty-state">
      <svg
        className="empty-state-icon"
        width="36"
        height="36"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        aria-hidden="true"
      >
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <path d="M7 9h10M7 13h10M7 17h6" />
      </svg>
      <p className="empty-state-text">{text}</p>
    </div>
  );
}

function SessionBody({ detail }: { detail: SessionDetail }) {
  return (
    <div className="review-body">
      <p className="muted small">
        {detail.summary.startedAt || "开始时间未知"}
        {detail.summary.endedAt ? ` → ${detail.summary.endedAt}` : ""}
        {detail.summary.model ? ` · ${detail.summary.model}` : ""}
        {durationText(detail.summary) ? ` · 时长 ${durationText(detail.summary)}` : ""}
      </p>

      <h3 className="review-subtitle">转录</h3>
      {detail.segments.length === 0 ? (
        <p className="muted small">该会话没有转录记录。</p>
      ) : (
        <>
          <div className="legend" aria-label="转录颜色图例">
            <span className="legend-item">
              <i className="legend-swatch swatch-interviewer" aria-hidden="true" />
              提问
            </span>
            <span className="legend-item">
              <i className="legend-swatch swatch-me" aria-hidden="true" />
              我
            </span>
            <span className="legend-item">
              <i className="legend-swatch swatch-echo" aria-hidden="true" />
              回声
            </span>
            <span className="legend-item">
              <i className="legend-swatch swatch-recovered" aria-hidden="true" />
              补录
            </span>
          </div>
          <div className="review-transcript">
            {detail.segments.map((segment) => (
              <div
                key={segment.id}
                className={`line ${segment.channel === "interviewer" ? "interviewer" : "me"}${segment.isEcho ? " echo" : ""}${segment.recovered ? " recovered" : ""}`}
              >
                <span className="who">{segment.channel === "interviewer" ? "提问" : "我"}</span>
                <span className="what">
                  {segment.text}
                  {segment.isEcho && <em className="echo-tag">回声</em>}
                  {segment.recovered && <em className="recovered-tag">补录</em>}
                </span>
              </div>
            ))}
          </div>
        </>
      )}

      <h3 className="review-subtitle">回答</h3>
      {detail.answers.length === 0 ? (
        <p className="muted small">该会话没有生成过答案。</p>
      ) : (
        <div className="review-answers">
          {detail.answers.map((answer) => (
            <div key={answer.id} className="review-answer">
              <pre className="answer-box">{answer.text || "（空答案）"}</pre>
              <p className="muted small">
                {answer.model || "模型未知"}
                {answer.ttftMs !== null ? ` · 首字 ${(answer.ttftMs / 1000).toFixed(1)}s` : ""}
              </p>
            </div>
          ))}
        </div>
      )}

      {(detail.segments.length === 0 && detail.answers.length === 0) && (
        <p className="muted small">
          该会话内容为空。若后端已接入存储，请确认会话在结束面试时已写入。
        </p>
      )}
    </div>
  );
}

/** 由 startedAt / endedAt 文本推算时长；后端格式为「YYYY-MM-DD HH:MM:SS」。 */
function durationText(summary: { startedAt: string; endedAt: string }): string {
  const start = parseTime(summary.startedAt);
  const end = parseTime(summary.endedAt);
  if (start === null || end === null || end < start) return "";
  const totalSeconds = Math.round((end - start) / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) return `${hours} 小时 ${minutes} 分`;
  if (minutes > 0) return `${minutes} 分 ${seconds} 秒`;
  return `${seconds} 秒`;
}

function parseTime(text: string): number | null {
  if (!text) return null;
  const parsed = Date.parse(text.replace(" ", "T"));
  return Number.isFinite(parsed) ? parsed : null;
}