import { memo } from "react";
import type { Channel, TranscriptLine } from "./types";

type Props = { lines: TranscriptLine[] };

function TranscriptPanel({ lines }: Props) {
  return (
    <div
      className="overlay-transcript"
      // 可聚焦才能用键盘/方向键滚动；role=log 让读屏知道这是追加型文本流。
      role="log"
      aria-label="实时转录"
      tabIndex={0}
    >
      {lines.length === 0 ? (
        <div className="overlay-empty">麦克风与系统声通道的转录会出现在这里。</div>
      ) : (
        lines.map((line) => (
          <div
            key={line.id}
            className={`line ${line.channel === "interviewer" ? "interviewer" : "me"}${line.isEcho ? " echo" : ""}`}
          >
            <span className="who">{labelOf(line.channel)}</span>
            <span className="what">
              {line.text}
              {line.isEcho && <em className="echo-tag">回声</em>}
              {line.recovered && <em className="recovered-tag">补录</em>}
            </span>
          </div>
        ))
      )}
    </div>
  );
}

function labelOf(channel: Channel): string {
  return channel === "interviewer" ? "提问" : "我";
}

/** 电平事件约 30 fps，转录只在新增行时重渲染。 */
export default memo(TranscriptPanel);
