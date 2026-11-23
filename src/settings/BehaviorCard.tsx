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
    <section className="settings-block">
      <div className="field-group">
        <p className="field-group-title">答案</p>

        <div className="field">
          <label className="field-label" htmlFor="behavior-style">
            答案风格
          </label>
          <select
            id="behavior-style"
            className="form-input"
            value={form.answerStyle}
            onChange={(event) =>
              patch({ answerStyle: event.target.value as AnswerStyle })
            }
          >
            <option value="concise">口语短答（可直接照念）</option>
            <option value="outline">要点提纲</option>
          </select>
        </div>

        <div className="field">
          <label className="field-label" htmlFor="behavior-trigger">
            触发方式
          </label>
          <select
            id="behavior-trigger"
            className="form-input"
            value={form.triggerMode}
            onChange={(event) =>
              patch({ triggerMode: event.target.value as TriggerMode })
            }
          >
            <option value="auto">自动（检测到问题就生成答案）</option>
            <option value="manual">手动（按 Ctrl+Shift+A 对最新问题生成答案）</option>
          </select>
        </div>

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
      </div>

      <div className="field-group">
        <p className="field-group-title">浮层</p>
        <div className="field">
          <label className="field-label" htmlFor="behavior-opacity">
            浮层不透明度
          </label>
          <div className="inline-row slider-row">
            <input
              id="behavior-opacity"
              className="slider"
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
        </div>
        <p className="muted small">
          浮层背景不透明度。视频面试时可调低以便看清画面；文字始终清晰。
          仅影响浮层，不改变窗口是否可见。修改后记得点「保存全部配置」。
        </p>
      </div>

      <div className="field-group">
        <p className="field-group-title">热键</p>
        <div className="settings-kv">
          <span className="settings-kv-key">显示浮层</span>
          <span className="settings-kv-val settings-mono">Ctrl+Shift+H</span>
          <span className="settings-kv-key">重新生成</span>
          <span className="settings-kv-val settings-mono">Ctrl+Shift+A</span>
          <span className="settings-kv-key">开始 / 结束</span>
          <span className="settings-kv-val settings-mono">Ctrl+Shift+R</span>
          <span className="settings-kv-key">截图（M5）</span>
          <span className="settings-kv-val settings-mono">Ctrl+Shift+S</span>
          <span className="settings-kv-key">隐藏浮层</span>
          <span className="settings-kv-val settings-mono">Esc</span>
        </div>
        <p className="muted small">
          手动模式下 Ctrl+Shift+A 即「生成答案」。
        </p>
      </div>
    </section>
  );
}