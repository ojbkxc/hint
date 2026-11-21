import { useCallback, useEffect, useState } from "react";
import BehaviorCard from "./settings/BehaviorCard";
import DevicesCard from "./settings/DevicesCard";
import ModelCard from "./settings/ModelCard";
import ModelDownloadCard from "./settings/ModelDownloadCard";
import ProfileCard from "./settings/ProfileCard";
import SessionCard from "./settings/SessionCard";
import { useModelManager } from "./settings/useModelManager";
import { useSessionControl } from "./settings/useSessionControl";
import { useSettingsForm } from "./settings/useSettingsForm";
import SessionReview from "./sessions/SessionReview";
import { useSessions } from "./sessions/useSessions";
import { isRecord, pickText } from "./lib/data";
import { safeInvoke, safeListen } from "./lib/tauri";

type Tab = "settings" | "review";

const TABS: Array<{ id: Tab; label: string }> = [
  { id: "settings", label: "设置" },
  { id: "review", label: "会话复盘" },
];

type Banner = { level: "info" | "error"; message: string };

export default function Dashboard() {
  const [tab, setTab] = useState<Tab>("settings");
  const [testAnswer, setTestAnswer] = useState("");
  const [banner, setBanner] = useState<Banner | null>(null);
  const form = useSettingsForm();
  const sessions = useSessions(tab === "review");
  const model = useModelManager();
  const session = useSessionControl();

  // 主窗口同样订阅后端提示：关闭窗口收进托盘等提示在浮层隐藏时只能在这里看到。
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    void safeListen<unknown>("hint://notice", (payload) => {
      if (!active || !isRecord(payload)) return;
      const message = pickText(payload, ["message"]);
      if (!message) return;
      setBanner({
        level: pickText(payload, ["level"]) === "error" ? "error" : "info",
        message,
      });
    }).then((fn) => {
      if (!active) {
        fn?.();
        return;
      }
      unlisten = fn;
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!banner) return;
    const timer = window.setTimeout(
      () => setBanner(null),
      banner.level === "error" ? 10000 : 6000,
    );
    return () => window.clearTimeout(timer);
  }, [banner]);

  async function handleTest() {
    setTestAnswer("");
    const response = await form.testConnection();
    if (response) setTestAnswer(response.text);
    return response;
  }

  const openLogDir = useCallback(async () => {
    const result = await safeInvoke("open_log_dir");
    if (!result.ok) setBanner({ level: "error", message: `打开日志目录失败：${result.error}` });
  }, []);

  return (
    <div className="dashboard-root">
      <header className="dashboard-header">
        <div>
          <h1>hint</h1>
          <p className="subtitle">面试实时提词器</p>
        </div>
        <div className="hint-badge">Ctrl+Shift+Space 显示浮层</div>
      </header>

      {banner && (
        <button
          type="button"
          className={`dashboard-banner banner-${banner.level}`}
          onClick={() => setBanner(null)}
          title="点击关闭"
        >
          {banner.message}
        </button>
      )}

      <nav className="tab-bar" role="tablist" aria-label="主功能">
        {TABS.map((item) => (
          <button
            key={item.id}
            type="button"
            role="tab"
            aria-selected={tab === item.id}
            className={`tab${tab === item.id ? " active" : ""}`}
            onClick={() => setTab(item.id)}
          >
            {item.label}
          </button>
        ))}
      </nav>

      {tab === "settings" ? (
        !form.loaded ? (
          <p className="muted">正在读取配置…</p>
        ) : (
          <div className="dashboard-grid">
            <ModelCard
              form={form.form}
              apiKey={form.apiKey}
              keySaved={form.keySaved}
              busy={form.busy}
              action={form.action}
              answer={testAnswer}
              status={form.status}
              models={form.models}
              onApiKey={form.setApiKey}
              onPatch={form.patch}
              onTest={handleTest}
              onLoadModels={() => void form.loadModels()}
            />
            <SessionCard control={session} />
            <ModelDownloadCard
              view={model.view}
              busy={model.busy}
              notice={model.notice}
              onDownload={() => void model.download()}
              onCancel={() => void model.cancel()}
              onRemove={() => void model.remove()}
            />
            <ProfileCard form={form.form} patch={form.patch} />
            <BehaviorCard form={form.form} patch={form.patch} />
            <DevicesCard
              form={form.form}
              loaded={form.loaded}
              patch={form.patch}
              loadDevices={form.loadDevices}
            />
            <div className="card save-card">
              <h2>保存</h2>
              <p className="muted small">
                模型、资料、回答行为与设备选择都保存在同一份配置里，修改后请点下方按钮。
                遇到问题时可在日志目录的 <code>logs\hint.log</code> 里查看原因并反馈给开发者。
              </p>
              <div className="action-row">
                <button
                  type="button"
                  className="primary"
                  onClick={() => void form.save()}
                  disabled={form.busy}
                >
                  {form.action === "save" ? "保存中…" : "保存全部配置"}
                </button>
                <button type="button" className="ghost" onClick={() => void openLogDir()}>
                  打开日志目录
                </button>
              </div>
              {form.status && <p className="status">{form.status}</p>}
            </div>
          </div>
        )
      ) : (
        <SessionReview
          sessions={sessions.sessions}
          loading={sessions.loading}
          error={sessions.error}
          selectedId={sessions.selectedId}
          detail={sessions.detail}
          detailLoading={sessions.detailLoading}
          detailError={sessions.detailError}
          notice={sessions.notice}
          onRefresh={() => void sessions.refresh()}
          onOpen={(id) => void sessions.open(id)}
          onExport={() => void sessions.exportMarkdown()}
          onDelete={() => void sessions.remove()}
          onDeleteAll={() => void sessions.removeAll()}
        />
      )}
    </div>
  );
}