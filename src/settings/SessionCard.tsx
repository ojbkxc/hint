import { useState } from "react";
import { safeInvoke } from "../lib/tauri";
import type { SessionControl } from "./useSessionControl";

type Props = { control: SessionControl };

/** 状态中文短名；Dashboard 的 titlebar 与卡片内部共用同一份映射。 */
export const SESSION_STATE_LABEL: Record<string, string> = {
  idle: "未开始",
  connecting: "正在启动语音引擎…",
  listening: "面试进行中",
  ready: "已就绪",
  connected: "已连接",
  error: "启动失败",
};

export default function SessionCard({ control }: Props) {
  const { state, message, running, busy, busyAction, error, notice, sessionId } = control;
  const listening = state === "listening";
  const connecting = state === "connecting";

  // 手动提问：给一条绕开语音识别的兜底路径。
  // 识别不准时用户仍然能拿到答案——模型与流式链路本身是好的（截图问答走
  // 同一份 stream_chat 已实测正常），坏的只可能是识别这一环。
  const [manualQuestion, setManualQuestion] = useState("");
  const [asking, setAsking] = useState(false);
  const [askStatus, setAskStatus] = useState("");

  async function askManual() {
    const text = manualQuestion.trim();
    if (!text) {
      setAskStatus("请先输入问题。");
      return;
    }
    setAsking(true);
    setAskStatus("");
    const result = await safeInvoke("ask_question", { question: text });
    setAsking(false);
    if (!result.ok) {
      setAskStatus(result.error);
      return;
    }
    setAskStatus("已提交，答案会流式出现在浮层。");
    setManualQuestion("");
  }

  const startLabel = connecting
    ? "正在启动…"
    : busy
      ? "启动中…"
      : state === "error"
        ? "重新开始面试"
        : "开始面试";

  return (
    <section className="settings-block">
      <div className="field-group">
        <p className="field-group-title">会话控制</p>
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
            className="secondary"
            onClick={control.stop}
            disabled={busy || !running}
          >
            {busyAction === "stop" ? "正在结束…" : "结束面试"}
          </button>
        </div>
      </div>

      <div className="field-group">
        <p className="field-group-title">手动提问</p>
        <p className="muted small">
          识别不准或听不清时，直接把问题打在这里回车，答案照样会流式出现在浮层。
          这条路径不经过语音识别，只调模型。
        </p>
        <div className="field">
          <label className="field-label" htmlFor="manual-question">
            要回答的问题
          </label>
          <div className="inline-row">
            <input
              id="manual-question"
              className="form-input"
              type="text"
              placeholder="例如：请介绍一下你在上一个项目里负责的部分"
              value={manualQuestion}
              onChange={(event) => setManualQuestion(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !asking) void askManual();
              }}
              disabled={!listening}
            />
            <button
              type="button"
              className="secondary"
              onClick={() => void askManual()}
              disabled={!listening || asking || !manualQuestion.trim()}
            >
              {asking ? "提交中…" : "提问"}
            </button>
          </div>
        </div>
        {askStatus && <p className="muted small">{askStatus}</p>}
        {!listening && <p className="muted small">开始面试后才能提问。</p>}
      </div>

      <div className="field-group">
        <p className="field-group-title">状态来源</p>
        <div className="settings-kv">
          <span className="settings-kv-key">当前状态</span>
          <span className="settings-kv-val">
            {SESSION_STATE_LABEL[state] ?? state}
          </span>
          <span className="settings-kv-key">会话编号</span>
          <span className="settings-kv-val settings-mono">
            {sessionId !== null ? sessionId : "—"}
          </span>
          <span className="settings-kv-key">界面焦点</span>
          <span className="settings-kv-val">{running ? "浮层已接管" : "设置页空闲"}</span>
        </div>
        {error && (
          <>
            <p className="error-text">{error}</p>
            <p className="muted small">可检查「模型接口」的 API Key 与「语音模型」状态后重新开始。</p>
          </>
        )}
        {notice && !error && <p className="status-info">{notice}</p>}
      </div>
    </section>
  );
}