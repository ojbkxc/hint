import { useEffect, useRef, useState } from "react";
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

  async function refresh() {
    setLoading(true);
    const result = await loadDevices();
    setInputs(result.inputs);
    setOutputs(result.outputs);
    setError(result.error ? `读取设备失败：${result.error}` : "");
    setLoading(false);
  }

  useEffect(() => {
    if (!loaded || attempted.current) return;
    attempted.current = true;
    void refresh();
  }, [loaded]);

  return (
    <section className="card">
      <div className="card-head">
        <h2>设备</h2>
        <button type="button" className="ghost" onClick={() => void refresh()} disabled={loading}>
          {loading ? "检测中…" : "刷新设备"}
        </button>
      </div>

      <label>
        麦克风（我的声音）
        <select
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
      </label>

      <label>
        系统输出（面试官声音）
        <select
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
      </label>

      {error ? (
        <p className="error-text">{error}</p>
      ) : (
        <p className="muted small">
          选择后记得点「保存配置」。系统输出走 loopback 采集，面试官的声音来自这里。
          双通道电平条在浮层底部实时跳动才算通过。
        </p>
      )}
    </section>
  );
}