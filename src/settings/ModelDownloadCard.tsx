import {
  percentOf,
  formatBytes,
  sourceLabel,
  stepLabel,
  type ModelView,
} from "./modelTypes";
import { deriveStatusTone, statusToneClass } from "./statusTone";

type Props = {
  view: ModelView;
  busy: boolean;
  notice: string;
  onDownload: () => void;
  onCancel: () => void;
  onRemove: () => void;
};

export default function ModelDownloadCard({
  view,
  busy,
  notice,
  onDownload,
  onCancel,
  onRemove,
}: Props) {
  return (
    <section className="settings-block">
      <div className="field-group">
        <p className="field-group-title">模型状态</p>
        <div className="settings-kv">
          <span className="settings-kv-key">当前阶段</span>
          <span className="settings-kv-val">
            {view.phase === "ready" && <span className="tag-ok">已就绪</span>}
            {view.phase === "loading" && "读取中…"}
            {view.phase === "downloading" && "下载中"}
            {view.phase === "unprepared" && "未下载"}
            {view.phase === "error" && "读取失败"}
          </span>
          <span className="settings-kv-key">模型体积</span>
          <span className="settings-kv-val settings-mono">{formatBytes(view.totalBytes)}</span>
          <span className="settings-kv-key">模型目录</span>
          <span className="settings-kv-val settings-mono">
            {view.modelDir || (view.phase === "ready" ? "默认目录" : "—")}
          </span>
        </div>
        <p className="muted small">
          本地 SenseVoice 语音识别模型，首次使用前必须下载。模型只存本机，
          不参与任何上传。
        </p>
      </div>

      <div className="field-group">
        <p className="field-group-title">下载与校验</p>

        {view.phase === "loading" ? (
          <p className="muted small">正在读取模型状态…</p>
        ) : view.phase === "ready" ? (
          <ReadyBody view={view} />
        ) : view.phase === "downloading" ? (
          <ProgressBody view={view} />
        ) : (
          <UnpreparedBody view={view} />
        )}

        <div className="action-row">
          {view.phase === "downloading" ? (
            <button type="button" className="secondary" onClick={onCancel}>
              取消下载
            </button>
          ) : (
            <button
              type="button"
              className="primary"
              onClick={onDownload}
              disabled={busy || view.phase === "loading"}
              title={
                view.phase === "ready"
                  ? "只核对文件是否存在且大小一致，不会重新下载或覆盖"
                  : undefined
              }
            >
              {view.phase === "ready" ? "检查模型文件" : "下载模型"}
            </button>
          )}
          {view.phase === "ready" && (
            <button type="button" className="danger" onClick={onRemove} disabled={busy}>
              删除模型
            </button>
          )}
        </div>

        {notice && <p className={statusToneClass(deriveStatusTone(notice))}>{notice}</p>}
        {view.error && <p className="error-text">{view.error}</p>}
      </div>
    </section>
  );
}

function ReadyBody({ view }: { view: ModelView }) {
  return (
    <div className="model-ready">
      <p className="muted small">
        删除前请先结束正在进行的面试；删除后需要重新下载约 {formatBytes(view.totalBytes)} 才能恢复语音识别。
        「检查模型文件」只核对文件是否存在、大小是否一致，不会重新下载。
      </p>
    </div>
  );
}

function UnpreparedBody({ view }: { view: ModelView }) {
  const percent = percentOf(view.downloadedBytes, view.totalBytes);
  return (
    <>
      <p className="muted small">
        当前未下载或文件不完整
        {view.downloadedBytes > 0
          ? `（已有 ${formatBytes(view.downloadedBytes)}，支持断点续传）`
          : ""}
        。
      </p>
      <div className="progress">
        <div className="progress-head">
          <span className="progress-text settings-mono">
            {formatBytes(view.downloadedBytes)} / {formatBytes(view.totalBytes)}
          </span>
          <span className="progress-text settings-mono">{percent.toFixed(1)}%</span>
        </div>
        <div className="progress-track">
          <div className="progress-fill" style={{ width: `${Math.max(0, percent)}%` }} />
        </div>
      </div>
    </>
  );
}

function ProgressBody({ view }: { view: ModelView }) {
  const percent = percentOf(view.completedBytes, view.totalBytes);
  return (
    <>
      <div className="progress">
        <div className="progress-head">
          <span className="progress-text">
            {stepLabel(view.step)}
            {view.resource ? ` · ${view.resource}` : ""}
          </span>
          <span className="progress-text settings-mono">{percent.toFixed(1)}%</span>
        </div>
        <div className="progress-track">
          <div className="progress-fill active" style={{ width: `${Math.max(1, percent)}%` }} />
        </div>
        <div className="progress-foot">
          <span className="settings-mono">
            {formatBytes(view.completedBytes)} / {formatBytes(view.totalBytes)}
          </span>
          {view.source && <span>来源：{sourceLabel(view.source)}</span>}
        </div>
      </div>
      <p className="muted small">
        下载期间请保持网络连接。中断后重新点击按钮即可从断点续传。
      </p>
    </>
  );
}