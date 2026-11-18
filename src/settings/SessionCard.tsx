import type { SessionControl } from "./useSessionControl";

type Props = { control: SessionControl };

const STATE_LABEL: Record<string, string> = {
  idle: "未开始",
  connecting: "正在启动语音引擎…",
  listening: "面试进行中",
  ready: "已就绪",
  connected: "已连接",
  error: "启动失败",
};

export default function SessionCard({ control }: Props) {
  const { state, message, running, busy, error, notice, sessionId } = control;
  const listening = state === "listening";
  const connecting = state === "connecting";

  const startLabel = connecting
    ? "正在启动…"
    : busy
      ? "启动中…"
      : state === "error"
        ? "重新开始面试"
        : "开始面试";

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

      {connecting && (
        <p className="muted small">
          正在加载本地语音模型，首次启动可能需要 10–30 秒，期间请勿重复点击。
        </p>
      )}

      {message && !error && !connecting && <p className="muted small">当前状态：{message}</p>}
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
          {startLabel}
        </button>
        <button
          type="button"
          onClick={control.stop}
          disabled={busy || !running}
        >
          {busy && running ? "正在结束…" : "结束面试"}
        </button>
      </div>

      {error && (
        <p className="error-text">
          {error}
          <br />
          可检查上方 API Key / 语音模型状态后重新开始。
        </p>
      )}
      {notice && !error && <p className="status">{notice}</p>}
    </section>
  );
}