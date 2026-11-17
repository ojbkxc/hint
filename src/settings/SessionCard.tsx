import type { SessionControl } from "./useSessionControl";

type Props = { control: SessionControl };

const STATE_LABEL: Record<string, string> = {
  idle: "未开始",
  connecting: "正在启动…",
  listening: "面试进行中",
  ready: "已就绪",
  connected: "已连接",
  error: "启动失败",
};

export default function SessionCard({ control }: Props) {
  const { state, message, running, busy, error, notice, sessionId } = control;
  const listening = state === "listening";

  return (
    <section className="card">
      <div className="card-head">
        <h2>面试会话</h2>
        <span className={`session-state state-${state}`}>{STATE_LABEL[state] ?? state}</span>
      </div>

      <p className="muted small">
        开始后面板会启动双通道采集与本地识别，转录和答案实时显示在浮层。
        快捷键 Ctrl+Shift+R 也可开始 / 结束。
      </p>

      {message && !error && <p className="muted small">当前状态：{message}</p>}
      {sessionId !== null && listening && (
        <p className="muted small">本次会话编号：{sessionId}</p>
      )}

      <div className="action-row">
        <button
          type="button"
          className="primary"
          onClick={control.start}
          disabled={busy || running}
        >
          {busy && !running ? "启动中…" : "开始面试"}
        </button>
        <button
          type="button"
          onClick={control.stop}
          disabled={busy || !running}
        >
          {busy && running ? "结束中…" : "结束面试"}
        </button>
      </div>

      {error && <p className="error-text">{error}</p>}
      {notice && !error && <p className="status">{notice}</p>}
    </section>
  );
}