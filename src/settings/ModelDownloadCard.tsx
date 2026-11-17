import {
  percentOf,
  formatBytes,
  sourceLabel,
  stepLabel,
  type ModelView,
} from "./modelTypes";

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
    <section className="card">
      <div className="card-head">
        <h2>语音模型</h2>
        {view.phase === "ready" && <span className="tag-ok">已就绪</span>}
      </div>

      <p className="muted small">
        本地 SenseVoice 语音识别模型（约 241MB），首次使用前必须下载。模型只存本机，
        不参与任何上传。
      </p>

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
          <button type="button" onClick={onCancel}>
            取消下载
          </button>
        ) : (
          <button
            type="button"
            className="primary"
            onClick={onDownload}
            disabled={busy || view.phase === "loading"}
          >
            {view.phase === "ready" ? "重新下载模型" : "下载模型"}
          </button>
        )}
        {view.phase === "ready" && (
          <button type="button" className="danger" onClick={onRemove} disabled={busy}>
            删除模型
          </button>
        )}
      </div>

      {notice && <p className="status">{notice}</p>}
      {view.error && <p className="error-text">{view.error}</p>}
    </section>
  );
}

function ReadyBody({ view }: { view: ModelView }) {
  return (
    <div className="model-ready">
      <p className="muted small">
        模型目录：<code className="model-path">{view.modelDir || "默认目录"}</code>
      </p>
      <p className="muted small">
        删除前请先结束正在进行的面试；删除后需要重新下载约 241MB 才能恢复语音识别。
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
          <span className="progress-text">
            {formatBytes(view.downloadedBytes)} / {formatBytes(view.totalBytes)}
          </span>
          <span className="progress-text">{percent.toFixed(0)}%</span>
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
          <span className="progress-text">{percent.toFixed(1)}%</span>
        </div>
        <div className="progress-track">
          <div className="progress-fill active" style={{ width: `${Math.max(1, percent)}%` }} />
        </div>
        <div className="progress-foot">
          <span>
            {formatBytes(view.completedBytes)} / {formatBytes(view.totalBytes)}
          </span>
          {view.source && <span>来源：{sourceLabel(view.source)}</span>}
        </div>
      </div>
      <p className="muted small">
        下载期间请保持网络连接。中断后可重新点击「下载模型」，会从断点续传。
      </p>
    </>
  );
}