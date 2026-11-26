import { memo, useRef } from "react";
import type { Channel, TranscriptLine } from "./types";
import { useStickToBottom } from "./useStickToBottom";

type Props = { lines: TranscriptLine[] };

function TranscriptPanel({ lines }: Props) {
  const listRef = useRef<HTMLDivElement | null>(null);
  // 最新一行始终可见，旧的往上溢出（固定 3 行高，见 App.css 的 .overlay-transcript）。
  const { onScroll } = useStickToBottom(listRef, lines);

  return (
    <div
      className="overlay-transcript"
      // 可聚焦才能用键盘/方向键滚动；role=log 让读屏知道这是追加型文本流。
      role="log"
      aria-label="实时转录"
      tabIndex={0}
      ref={listRef}
      onScroll={onScroll}
    >
      {lines.length === 0 ? (
        <div className="overlay-empty">麦克风与系统声通道的转录会出现在这里。</div>
      ) : (
        // 内层包裹用 margin-top: auto 贴底：内容不足 3 行时从下往上排，
        // 超出时正常滚动（外层不用 justify-content: flex-end，那会让溢出的
        // 行永远滚不到）。
        <div className="overlay-transcript-body">
          {lines.map((line) => (
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
          ))}
        </div>
      )}
    </div>
  );
}

function labelOf(channel: Channel): string {
  return channel === "interviewer" ? "提问" : "我";
}

/** 电平事件约 30 fps，转录只在新增行时重渲染。 */
export default memo(TranscriptPanel);
