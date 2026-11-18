import type { AnswerStyle, Settings, TriggerMode } from "./types";
import {
  MAX_OVERLAY_OPACITY,
  MIN_OVERLAY_OPACITY,
  normalizeOpacity,
} from "./types";

type Props = {
  form: Settings;
  patch: (next: Partial<Settings>) => void;
};

export default function BehaviorCard({ form, patch }: Props) {
  const opacityPercent = Math.round(normalizeOpacity(form.overlayOpacity) * 100);
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
          <option value="auto">自动（检测到问题就生成答案）</option>
          <option value="manual">手动（按 Ctrl+Shift+A 对最新问题生成答案）</option>
        </select>
      </label>
      {form.triggerMode === "auto" ? (
        <p className="muted small">
          自动模式：面试官说完一句话，只要被判定为问题就会立即生成答案。
        </p>
      ) : (
        <p className="muted small">
          手动模式：不会自动生成答案。面试官提问后按 Ctrl+Shift+A（或浮层里的「重新生成」）
          才会生成。只有被识别为问题的话才会被记住，判断有遗漏时请直接用浮层的「重新生成」。
        </p>
      )}
      <p className="muted small">
        热键：Ctrl+Shift+Space 显示浮层 · Ctrl+Shift+A 重新生成（手动模式下即「生成答案」）
        · Ctrl+Shift+S 截图（M5） · Ctrl+Shift+R 开始/结束面试 · Esc 隐藏浮层。
      </p>

      <label>
        浮层不透明度
        <div className="inline-row slider-row">
          <input
            type="range"
            min={MIN_OVERLAY_OPACITY * 100}
            max={MAX_OVERLAY_OPACITY * 100}
            step={1}
            value={opacityPercent}
            onChange={(event) =>
              patch({ overlayOpacity: normalizeOpacity(Number(event.target.value) / 100) })
            }
          />
          <span className="slider-value">{opacityPercent}%</span>
        </div>
      </label>
      <p className="muted small">
        浮层背景不透明度。视频面试时可调低以便看清画面；文字始终清晰。
        仅影响浮层，不改变窗口是否可见。修改后记得点「保存全部配置」。
      </p>
    </section>
  );
}