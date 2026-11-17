import type { AnswerStyle, Settings, TriggerMode } from "./types";

type Props = {
  form: Settings;
  patch: (next: Partial<Settings>) => void;
};

export default function BehaviorCard({ form, patch }: Props) {
  return (
    <section className="card">
      <h2>回答行为</h2>
      <label>
        答案风格
        <select
          value={form.answerStyle}
          onChange={(event) =>
            patch({ answerStyle: event.target.value as AnswerStyle })
          }
        >
          <option value="concise">口语短答（可直接照念）</option>
          <option value="outline">要点提纲</option>
        </select>
      </label>
      <label>
        触发方式
        <select
          value={form.triggerMode}
          onChange={(event) =>
            patch({ triggerMode: event.target.value as TriggerMode })
          }
        >
          <option value="auto">自动（检测到问题就生成）</option>
          <option value="manual">按键触发（Ctrl+Shift+A）</option>
        </select>
      </label>
      <p className="muted small">
        热键：Ctrl+Shift+Space 显示浮层 · Ctrl+Shift+A 重新生成 · Ctrl+Shift+S 截图（M5）
        · Ctrl+Shift+R 开始/结束面试 · Esc 隐藏浮层。
      </p>
    </section>
  );
}