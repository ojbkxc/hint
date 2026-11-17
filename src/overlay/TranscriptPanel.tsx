import type { Channel, TranscriptLine } from "./types";

type Props = { lines: TranscriptLine[] };

export default function TranscriptPanel({ lines }: Props) {
  return (
    <div className="overlay-transcript">
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
            </span>
          </div>
        ))
      )}
    </div>
  );
}

function labelOf(channel: Channel): string {
  return channel === "interviewer" ? "面试官" : "我";
}