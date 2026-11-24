import type { LevelsState, SpeechState } from "./types";

type Props = {
  levels: LevelsState;
  model: string;
  ttftMs: number | null;
  streaming: boolean;
  speech: SpeechState;
};

export default function StatusBar({ levels, model, ttftMs, streaming, speech }: Props) {
  return (
    <footer className="overlay-footer">
      <span className="footer-group">
        <LevelBar label="麦克风" value={levels.mic} tone="mic" />
        <LevelBar label="系统声" value={levels.system} tone="system" />
        {speech.joint !== "neither" && (
          <span className={`speaking-badge speaking-${speech.joint}`}>
            {jointText(speech.joint)}
          </span>
        )}
      </span>
      <span className="footer-group">
        {/*
          没有模型两个来源都空时才这么说，且措辞用「未配置」而不是「未连接」：
          用户看到「未连接模型」会以为接口连不上，实际只是还没在设置里选模型。
        */}
        <span className="footer-item" title={model || "尚未在设置里选择模型"}>
          {model || "未配置模型"}
        </span>
        <span className="footer-item" title="首字延迟">
          {ttftText(ttftMs, streaming)}
        </span>
      </span>
    </footer>
  );
}

function jointText(joint: SpeechState["joint"]): string {
  switch (joint) {
    case "interviewer":
      return "面试官在说";
    case "me":
      return "你在说";
    case "both":
      return "双方同时";
    default:
      return "";
  }
}

function LevelBar({ label, value, tone }: { label: string; value: number; tone: "mic" | "system" }) {
  const percent = Math.round(Math.min(1, Math.max(0, value)) * 100);
  return (
    <span className={`level level-${tone}`} title={`${label}电平 ${percent}%`}>
      <span className="level-label">{label}</span>
      <span className="level-track">
        <span className="level-fill" style={{ width: `${Math.max(3, percent)}%` }} />
      </span>
    </span>
  );
}

function ttftText(ttftMs: number | null, streaming: boolean): string {
  if (ttftMs !== null) {
    return `首字 ${(ttftMs / 1000).toFixed(1)}s`;
  }
  return streaming ? "首字 …" : "首字 —";
}