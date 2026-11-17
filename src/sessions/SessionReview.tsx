import type { SessionDetail, SessionSummary } from "./types";

type Props = {
  sessions: SessionSummary[];
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

export default function SessionReview(props: Props) {
  const {
    sessions,
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
              disabled={loading || sessions.length === 0}
              title="永久删除本机全部会话记录"
            >
              清空全部
            </button>
          </div>
        </div>

        {error ? (
          <p className="error-text">{error}</p>
        ) : !loading && sessions.length === 0 ? (
          <p className="muted small">还没有会话记录。完成一次面试后，这里会列出可回看的转录与问答。</p>
        ) : loading ? (
          <p className="muted small">正在读取会话列表…</p>
        ) : (
          <ul className="session-items">
            {sessions.map((session) => (
              <li key={session.id}>
                <button
                  type="button"
                  className={`session-item${session.id === selectedId ? " active" : ""}`}
                  onClick={() => onOpen(session.id)}
                >
                  <span className="session-title">
                    {session.note || `会话 ${session.id}`}
                  </span>
                  <span className="session-meta">
                    {session.startedAt || "时间未知"}
                    {session.model ? ` · ${session.model}` : ""}
                  </span>
                </button>
              </li>
            ))}
          </ul>
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

        {notice && <p className="status">{notice}</p>}
        {detailError && <p className="error-text">{detailError}</p>}

        {!selectedId ? (
          <p className="muted small">从左侧选择一条会话查看详情。</p>
        ) : detailLoading ? (
          <p className="muted small">正在加载会话…</p>
        ) : detail ? (
          <SessionBody detail={detail} />
        ) : (
          !detailError && <p className="muted small">没有可显示的内容。</p>
        )}
      </section>
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
      </p>

      <h3 className="review-subtitle">转录</h3>
      {detail.segments.length === 0 ? (
        <p className="muted small">该会话没有转录记录。</p>
      ) : (
        <div className="review-transcript">
          {detail.segments.map((segment) => (
            <div
              key={segment.id}
              className={`line ${segment.channel === "interviewer" ? "interviewer" : "me"}${segment.isEcho ? " echo" : ""}`}
            >
              <span className="who">{segment.channel === "interviewer" ? "面试官" : "我"}</span>
              <span className="what">
                {segment.text}
                {segment.isEcho && <em className="echo-tag">回声</em>}
              </span>
            </div>
          ))}
        </div>
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