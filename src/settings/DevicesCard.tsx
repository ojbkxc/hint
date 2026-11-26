import { useCallback, useEffect, useRef, useState } from "react";
import type { AudioDevice, Settings } from "./types";

type Props = {
  form: Settings;
  loaded: boolean;
  patch: (next: Partial<Settings>) => void;
  loadDevices: () => Promise<{
    inputs: AudioDevice[];
    outputs: AudioDevice[];
    error: string;
  }>;
};

export default function DevicesCard({ form, loaded, patch, loadDevices }: Props) {
  const [inputs, setInputs] = useState<AudioDevice[]>([]);
  const [outputs, setOutputs] = useState<AudioDevice[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const attempted = useRef(false);

  // 已保存的设备 id 暂时不在列表里（未插拔/后端未接线）时保留可见项。
  const micMissing = form.micDevice !== "" && !inputs.some((d) => d.id === form.micDevice);
  const outputMissing =
    form.outputDevice !== "" && !outputs.some((d) => d.id === form.outputDevice);

  // 稳定引用：effect 与按钮共用，避免 exhaustive-deps 缺 refresh 的 stale 闭包。
  const refresh = useCallback(async () => {
    setLoading(true);
    const result = await loadDevices();
    setInputs(result.inputs);
    setOutputs(result.outputs);
    setError(result.error ? `读取设备失败：${result.error}` : "");
    setLoading(false);
  }, [loadDevices]);

  useEffect(() => {
    if (!loaded || attempted.current) return;
    attempted.current = true;
    void refresh();
  }, [loaded, refresh]);

  return (
    <section className="settings-block">
      <div className="field-group">
        <div className="field-head">
          <p className="field-group-title">输入与输出</p>
          <button type="button" className="ghost" onClick={() => void refresh()} disabled={loading}>
            {loading ? "检测中…" : "刷新设备"}
          </button>
        </div>

        <div className="field">
          <label className="field-label" htmlFor="device-mic">
            麦克风（我的声音）
          </label>
          <select
            id="device-mic"
            className="form-input"
            value={form.micDevice}
            onChange={(event) => patch({ micDevice: event.target.value })}
          >
            <option value="">系统默认</option>
            {micMissing && <option value={form.micDevice}>已保存设备（当前未检测到）</option>}
            {inputs.map((device) => (
              <option key={device.id} value={device.id}>
                {device.name}
              </option>
            ))}
          </select>
        </div>

        <div className="field">
          <label className="field-label" htmlFor="device-output">
            系统输出（对方声音）
          </label>
          <select
            id="device-output"
            className="form-input"
            value={form.outputDevice}
            onChange={(event) => patch({ outputDevice: event.target.value })}
          >
            <option value="">系统默认</option>
            {outputMissing && (
              <option value={form.outputDevice}>已保存设备（当前未检测到）</option>
            )}
            {outputs.map((device) => (
              <option key={device.id} value={device.id}>
                {device.name}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className="field-group">
        <p className="field-group-title">检测结果</p>
        <div className="settings-kv">
          <span className="settings-kv-key">输入通道</span>
          <span className="settings-kv-val settings-mono">{inputs.length}</span>
          <span className="settings-kv-key">输出通道</span>
          <span className="settings-kv-val settings-mono">{outputs.length}</span>
          <span className="settings-kv-key">采集方式</span>
          <span className="settings-kv-val">双通道（麦克风 + loopback）</span>
        </div>

        {error ? (
          <>
            <p className="error-text">{error}</p>
            <p className="muted small">请确认麦克风未被其它程序独占后点「刷新设备」。</p>
          </>
        ) : loading ? (
          <p className="status-info">正在检测音频设备…</p>
        ) : inputs.length === 0 && outputs.length === 0 ? (
          <div className="empty-state">
            <svg
              className="empty-state-icon"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden="true"
              focusable="false"
            >
              <path d="M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3z" />
              <path d="M5 11a7 7 0 0 0 14 0" />
              <path d="M12 18v3" />
              <path d="M8 21h8" />
            </svg>
            <p className="empty-state-text">
              未检测到可用音频设备。请插入麦克风/扬声器后点「刷新设备」；也可以先用「系统默认」开始。
            </p>
          </div>
        ) : (
          <p className="muted small">
            选择后记得点「保存全部配置」。系统输出走 loopback 采集，对方的声音来自这里。
            双通道电平条在浮层底部实时跳动才算通过。
          </p>
        )}
      </div>
    </section>
  );
}