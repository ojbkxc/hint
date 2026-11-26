import { Fragment, useCallback, useEffect, useState, type ReactNode } from "react";
import BehaviorCard from "./settings/BehaviorCard";
import DevicesCard from "./settings/DevicesCard";
import ModelCard from "./settings/ModelCard";
import ModelDownloadCard from "./settings/ModelDownloadCard";
import ProfileCard from "./settings/ProfileCard";
import SessionCard, { SESSION_STATE_LABEL } from "./settings/SessionCard";
import { useModelManager } from "./settings/useModelManager";
import { useSessionControl } from "./settings/useSessionControl";
import { useSettingsForm } from "./settings/useSettingsForm";
import { formatBytes, percentOf } from "./settings/modelTypes";
import { deriveStatusTone, statusToneClass } from "./settings/statusTone";
import { normalizeOpacity, type Settings } from "./settings/types";
import SessionReview from "./sessions/SessionReview";
import { useSessions } from "./sessions/useSessions";
import { isRecord, pickText } from "./lib/data";
import { safeInvoke, safeListen } from "./lib/tauri";
import { applyTheme } from "./lib/theme";

type Tab = "settings" | "review";

/** 设置页右栏当前渲染的区块；只有选中项可见，其余保持挂载（用 hidden 收起）。 */
type SectionId =
  | "model"
  | "voice"
  | "session"
  | "devices"
  | "profile"
  | "behavior"
  | "save";

type Tone = "ok" | "warn" | "error" | "busy" | "neutral";

type Badge = { text: string; tone: Tone };

const TABS: Array<{ id: Tab; label: string }> = [
  { id: "settings", label: "设置" },
  { id: "review", label: "会话复盘" },
];

/**
 * 左栏分组导航。分组标题与条目来自同一份 SECTION_LABEL，
 * 新增区块时只改这一处，rail 与 titlebar 不会说法不一致。
 */
const RAIL_GROUPS: Array<{
  title: string;
  items: Array<{ id: SectionId; glyph: string }>;
}> = [
  { title: "接口与模型", items: [{ id: "model", glyph: "接" }, { id: "voice", glyph: "声" }] },
  { title: "会话", items: [{ id: "session", glyph: "会" }, { id: "devices", glyph: "设" }] },
  {
    title: "资料与行为",
    items: [{ id: "profile", glyph: "简" }, { id: "behavior", glyph: "行" }],
  },
  { title: "其他", items: [{ id: "save", glyph: "存" }] },
];

const SECTION_LABEL: Record<SectionId, string> = {
  model: "模型接口",
  voice: "语音模型",
  session: "面试会话",
  devices: "设备",
  profile: "简历与项目资料",
  behavior: "回答行为",
  save: "保存与日志",
};

/**
 * 顶部提示条。
 *
 * `sticky` 表示**不自动消失**：需要用户动手处理的告警（例如配置文件损坏、
 * 简历与项目资料被重置）必须留到他看见并关闭为止——10 秒后自己消失的提示，
 * 对「你的数据没了」这种事实毫无用处。
 */
type Banner = { level: "info" | "error"; message: string; sticky?: boolean };

export default function Dashboard() {
  const [tab, setTab] = useState<Tab>("settings");
  const [section, setSection] = useState<SectionId>("model");
  const [testAnswer, setTestAnswer] = useState("");
  const [banner, setBanner] = useState<Banner | null>(null);
  // 环境自检：点按钮才跑（内部要探测模型、设备、日志尾部，不必每次进页面都跑）。
  const [doctorText, setDoctorText] = useState("");
  const [doctorBusy, setDoctorBusy] = useState(false);
  // 浮层可见性与切换中标志：徽章按钮据此显示「显示浮层 / 隐藏浮层」。
  // 初值 true 与 tauri.conf.json 的 overlay.visible 一致（1.0.10 起默认显示）。
  const [overlayVisible, setOverlayVisible] = useState(true);

  // 按钮状态要跟浮层的**真实**可见性一致。
  //
  // 不能靠 `useState(true)` 猜：用户上次收起浮层后，启动时它是隐藏的，
  // 按钮若仍显示「隐藏浮层」，点一下等于去隐藏一个已经隐藏的窗口——
  // 用户看到的是「点了没反应」。
  useEffect(() => {
    let active = true;
    void (async () => {
      const result = await safeInvoke<boolean>("overlay_is_visible");
      if (!active || !result.ok) return;
      if (typeof result.data === "boolean") setOverlayVisible(result.data);
    })();
    return () => {
      active = false;
    };
  }, []);

  // 后端才是「浮层现在到底可见吗」的唯一真相：按钮、Ctrl+Shift+H、托盘三条入口
  // 都会改它。只靠首屏拉一次不够——用快捷键或托盘切换后按钮文案会停在旧状态。
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    void safeListen<unknown>("hint://overlay-visible", (payload) => {
      if (!active) return;
      if (typeof payload === "boolean") setOverlayVisible(payload);
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
  const [overlayBusy, setOverlayBusy] = useState(false);
  const form = useSettingsForm();
  const sessions = useSessions(tab === "review");
  const model = useModelManager();
  const session = useSessionControl();

  // 配置文件损坏：用 banner 明确告诉用户。
  //
  // 后端在 `AppState::new` 阶段就读配置（那时日志还没初始化），损坏的文件会被
  // 改名成 `settings.json.bad` 并回退默认值——用户的简历、项目资料、接口配置
  // **全部回到空**。不提示的话他只会在下次保存时发现"我填的东西没了"。
  //
  // 这里**直接拉命令、不等 `form.loaded`**：等 `form.loaded` 会引入时序依赖，
  // 真机验证时新启动一轮就是「后端返回了 settings.json.bad、banner 却始终不出现」，
  // 而手动 reload 后才出现——说明首次挂载那次状态更新被丢掉了（StrictMode 双挂载
  // 时第一次 effect 的 `disposed` 会把 setState 拦掉）。命令本身没有副作用，
  // 多点一次也没代价。
  useEffect(() => {
    let active = true;
    void (async () => {
      const result = await safeInvoke<string>("config_corruption_notice");
      if (!active || !result.ok) return;
      if (typeof result.data !== "string" || !result.data) return;
      setBanner({
        level: "error",
        sticky: true,
        message: `配置文件损坏，已回退默认设置；原文件已备份为「${result.data}」。简历与项目资料需要重新填写。`,
      });
    })();
    return () => {
      active = false;
    };
  }, []);

  // 会话数据库降级到内存库：历史面试记录读不出来了，必须告诉用户。
  //
  // 与上面同一条教训——后端在日志初始化之前就发现了这件事，只能由前端主动拉。
  // 注意它和「配置损坏」可能同时发生（磁盘整个坏掉时），用两条独立 effect、
  // 后到的覆盖先到的：数据库这条更严重（历史记录不可恢复），排在后面加载。
  useEffect(() => {
    let active = true;
    void (async () => {
      const result = await safeInvoke<boolean>("store_degraded_notice");
      if (!active || !result.ok) return;
      if (result.data !== true) return;
      setBanner({
        level: "error",
        sticky: true,
        message:
          "会话数据库打不开，已临时改用内存记录：历史面试记录无法读取，本次关闭后新记录也会清空。请检查磁盘空间或权限后重启。",
      });
    })();
    return () => {
      active = false;
    };
  }, []);

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
    // sticky 的提示不排自动关闭：需要用户动手处理的告警（配置损坏、数据被重置）
    // 不能 10 秒后自己消失——那等于没提示。
    if (banner.sticky) return;
    const timer = window.setTimeout(
      () => setBanner(null),
      banner.level === "error" ? 10000 : 6000,
    );
    return () => window.clearTimeout(timer);
  }, [banner]);

  /**
   * 主题跟随表单值：读到已保存的配置、或用户在「回答行为」里改了主题，
   * 配置页立刻换肤；点「保存全部配置」把值落盘，浮层由后端事件同步。
   *
   * 写在这里而不是保存回调里：首屏 get_settings 回来后也要应用，
   * 否则重启后深色配置要等用户再动一次设置才生效。
   */
  useEffect(() => {
    applyTheme(form.form.theme);
  }, [form.form.theme]);

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

  /**
   * 环境自检：与 `--doctor` 走同一个后端 collect，屏幕上看到的和报告文件一致。
   * 用户在 GUI 里就能自查版本/模型/配置/会话库/音频设备/日志尾部告警。
   */
  const runDoctor = useCallback(async () => {
    setDoctorBusy(true);
    const result = await safeInvoke<string>("doctor_report");
    setDoctorBusy(false);
    if (!result.ok) {
      setBanner({ level: "error", message: `环境自检失败：${result.error}` });
      return;
    }
    setDoctorText(typeof result.data === "string" ? result.data : "");
  }, []);

  /**
   * 清理升级后残留的旧模型目录。
   *
   * 自检会指出「旧模型目录只剩空壳，可安全删除」，但用户拿到建议还得自己
   * 去文件管理器找。后端只在旧目录确实不含模型文件时才删（有内容直接拒绝），
   * 这里只负责触发与提示。
   */
  const cleanLegacy = useCallback(async () => {
    const result = await safeInvoke<string>("clean_legacy_model_dir");
    if (!result.ok) {
      setBanner({ level: "error", message: result.error });
      return;
    }
    setBanner({
      level: "info",
      message: typeof result.data === "string" ? result.data : "旧模型目录已清理",
    });
    // 清理后报告里那行提示就过期了，重新跑一次自检保持与磁盘一致。
    void runDoctor();
  }, [runDoctor]);


  /**
   * 浮层显示 / 隐藏切换（与 Ctrl+Shift+H 同一条后端命令）。
   *
   * `toggle_overlay` 返回切换后的可见状态，直接拿去更新本地指示，
   * 不用再单独查一次。
   */
  const toggleOverlay = useCallback(async () => {
    setOverlayBusy(true);
    const result = await safeInvoke<unknown>("toggle_overlay");
    setOverlayBusy(false);
    if (!result.ok) {
      setBanner({ level: "error", message: `切换浮层失败：${result.error}` });
      return;
    }
    if (typeof result.data === "boolean") setOverlayVisible(result.data);
  }, []);

  /**
   * rail 徽标全部由现有 props 派生，只是把散落在各卡里的状态前置到导航上，
   * 不新增任何状态，也不改 hook 的返回值。
   */
  /**
   * 开始面试还差什么。
   *
   * 后端 `start_session` 会拒掉其中三项（Key / 模型 / Base URL 与模型名），
   * 但用户在看到那条报错之前完全不知道还差几个条件——六个 rail 条目各显示
   * 自己的状态，没有一处告诉他「现在能不能开始」。这里把判据前置到界面上。
   *
   * 只做提示、不拦按钮：手动提问和截图问答在没有语音模型时也能用，
   * 拦死开始按钮会把这两条路一起堵掉。
   */
  const readiness = (() => {
    if (!form.loaded) return [];
    const missing: Array<{ label: string; section: SectionId }> = [];
    if (!form.form.baseUrl.trim() || !form.form.model.trim()) {
      missing.push({ label: "Base URL 与模型名", section: "model" });
    }
    if (!form.keySaved) {
      missing.push({ label: "API Key", section: "model" });
    }
    if (model.view.phase !== "ready") {
      missing.push({ label: "下载模型", section: "voice" });
    }
    return missing;
  })();

  function railBadge(id: SectionId): Badge | null {
    switch (id) {
      case "model":
        if (!form.loaded) return { text: "读取中", tone: "busy" };
        return form.keySaved
          ? { text: "Key 已保存", tone: "ok" }
          : { text: "Key 未保存", tone: "warn" };
      case "voice": {
        const phase = model.view.phase;
        if (phase === "ready") return { text: "就绪", tone: "ok" };
        if (phase === "downloading")
          return {
            text: `${percentOf(model.view.completedBytes, model.view.totalBytes).toFixed(0)}%`,
            tone: "busy",
          };
        if (phase === "loading") return { text: "读取中", tone: "busy" };
        if (phase === "error") return { text: "失败", tone: "error" };
        return { text: "未下载", tone: "warn" };
      }
      case "session":
        if (session.state === "listening") return { text: "进行中", tone: "ok" };
        if (session.state === "connecting") return { text: "启动中", tone: "busy" };
        if (session.state === "error") return { text: "失败", tone: "error" };
        return { text: "未开始", tone: "neutral" };
      case "devices":
        return form.form.micDevice || form.form.outputDevice
          ? { text: "已指定", tone: "ok" }
          : { text: "系统默认", tone: "neutral" };
      case "profile": {
        const total = form.form.resume.length + form.form.projects.length;
        return total === 0
          ? { text: "未填写", tone: "warn" }
          : { text: `${total} 字`, tone: "neutral" };
      }
      case "behavior":
        return { text: form.form.triggerMode === "auto" ? "自动" : "手动", tone: "neutral" };
      case "save": {
        if (!form.status) return null;
        const tone = deriveStatusTone(form.status);
        if (tone === "error") return { text: "失败", tone: "error" };
        if (tone === "warning") return { text: "待处理", tone: "warn" };
        if (tone === "success") return { text: "已保存", tone: "ok" };
        return { text: "提示", tone: "neutral" };
      }
      default:
        return null;
    }
  }

  /** titlebar 右侧的等宽技术值：只放当前区块的关键数字/标识，不放说明文案。 */
  function panelMeta(id: SectionId): ReactNode {
    switch (id) {
      case "model":
        return (
          <>
            {providerShort(form.form.provider)} ·{" "}
            <span className="settings-mono">{form.form.model || "未填模型"}</span>
          </>
        );
      case "voice": {
        const view = model.view;
        if (view.phase === "ready")
          return (
            <>
              已就绪 · <span className="settings-mono">{formatBytes(view.totalBytes)}</span>
            </>
          );
        if (view.phase === "downloading")
          return (
            <span className="settings-mono">
              {percentOf(view.completedBytes, view.totalBytes).toFixed(1)}% ·{" "}
              {formatBytes(view.completedBytes)} / {formatBytes(view.totalBytes)}
            </span>
          );
        return (
          <span className="settings-mono">{formatBytes(view.totalBytes)} · 需下载</span>
        );
      }
      case "session":
        return session.sessionId !== null
          ? `会话 ${session.sessionId}`
          : "未开始，快捷键 Ctrl+Shift+R";
      case "devices":
        return `麦克风 ${form.form.micDevice ? "指定" : "默认"} · 输出 ${
          form.form.outputDevice ? "指定" : "默认"
        }`;
      case "profile":
        return (
          <span className="settings-mono">
            简历 {form.form.resume.length} 字 · 项目 {form.form.projects.length} 字
          </span>
        );
      case "behavior":
        return `${form.form.answerStyle === "concise" ? "口语短答" : "要点提纲"} · ${
          form.form.triggerMode === "auto" ? "自动触发" : "手动触发"
        } · 浮层 ${Math.round(normalizeOpacity(form.form.overlayOpacity) * 100)}%`;
      case "save":
        return <span className="settings-mono">logs\hint.log</span>;
      default:
        return null;
    }
  }

  /** 只有「面试会话」需要把状态胶囊放到 titlebar 右侧；其余区块状态已在 rail 上。 */
  function panelHeadActions(): ReactNode {
    if (section !== "session") return null;
    return (
      <span className={`session-state state-${session.state}`}>
        {SESSION_STATE_LABEL[session.state] ?? session.state}
      </span>
    );
  }

  return (
    <div className="dashboard-root">
      <header className="dashboard-header">
        <div>
          <h1>hint</h1>
          <p className="subtitle">面试实时提词器</p>
        </div>
        {/* 徽章原本只是提示文字，现在做成真按钮：点一下等价于按 Ctrl+Shift+H，
            切换浮层显示 / 隐藏。热键文案保留，用户仍然知道有热键。 */}
        <button
          type="button"
          className="hint-badge hint-badge-button"
          onClick={() => void toggleOverlay()}
          disabled={overlayBusy}
          title="显示 / 隐藏浮层（等同按 Ctrl+Shift+H）"
        >
          <span aria-hidden="true">{overlayVisible ? "◉" : "◎"}</span>
          {overlayVisible ? "隐藏浮层" : "显示浮层"}
          <kbd>Ctrl+Shift+H</kbd>
        </button>
      </header>

      {/* 准备状态：缺项时列出，并可直接跳到对应区块。全齐时不占地方。 */}
      {readiness.length > 0 && (
        <div className="readiness-strip" role="status">
          <span className="readiness-title">
            开始面试前还差 {readiness.length} 项
          </span>
          {readiness.map((item) => (
            <button
              key={item.label}
              type="button"
              className="readiness-chip"
              onClick={() => setSection(item.section)}
              title={`去「${SECTION_LABEL[item.section]}」补齐`}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}

      {banner && (
        <button
          type="button"
          role="status"
          aria-live={banner.level === "error" ? "assertive" : "polite"}
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
            id={`tab-${item.id}`}
            type="button"
            role="tab"
            aria-selected={tab === item.id}
            aria-controls={`tabpanel-${item.id}`}
            className={`tab${tab === item.id ? " active" : ""}`}
            onClick={() => setTab(item.id)}
          >
            {item.label}
          </button>
        ))}
      </nav>

      {tab === "settings" ? (
        <div
          className="tabpanel"
          role="tabpanel"
          id="tabpanel-settings"
          aria-labelledby="tab-settings"
        >
          {!form.loaded ? (
            <div className="settings-shell" aria-busy="true">
              <span className="visually-hidden">正在读取配置</span>
              <div className="settings-rail settings-rail-skeleton" aria-hidden="true">
                {[0, 1, 2, 3, 4, 5, 6].map((key) => (
                  <div key={key} className="skeleton skeleton-text" />
                ))}
              </div>
              <div className="settings-panel" aria-hidden="true">
                <div className="settings-titlebar">
                  <div className="settings-title">
                    <h2 className="settings-title-name">正在读取配置</h2>
                  </div>
                </div>
                <div className="settings-body">
                  {[0, 1, 2].map((key) => (
                    <div key={key} className="skeleton" style={{ height: 120 }} />
                  ))}
                </div>
              </div>
            </div>
          ) : (
            <div className="settings-shell">
              <nav className="settings-rail" aria-label="设置分区">
                {RAIL_GROUPS.map((group) => (
                  <Fragment key={group.title}>
                    <div className="settings-rail-title">{group.title}</div>
                    {group.items.map((item) => {
                      const badge = railBadge(item.id);
                      const active = section === item.id;
                      return (
                        <button
                          key={item.id}
                          type="button"
                          className={`settings-rail-item${active ? " active" : ""}`}
                          aria-current={active ? "true" : undefined}
                          aria-controls={`section-${item.id}`}
                          onClick={() => setSection(item.id)}
                          title={SECTION_LABEL[item.id]}
                        >
                          <span className="settings-rail-glyph" aria-hidden="true">
                            {item.glyph}
                          </span>
                          <span className="settings-rail-name">{SECTION_LABEL[item.id]}</span>
                          {badge && (
                            <span className={`settings-rail-badge tone-${badge.tone}`}>
                              {badge.text}
                            </span>
                          )}
                        </button>
                      );
                    })}
                  </Fragment>
                ))}
              </nav>

              <div className="settings-panel">
                <div className="settings-titlebar">
                  <div className="settings-title">
                    <h2 className="settings-title-name">{SECTION_LABEL[section]}</h2>
                    <span className="settings-meta">{panelMeta(section)}</span>
                  </div>
                  <div className="settings-head-actions">{panelHeadActions()}</div>
                </div>

                <div className="settings-body">
                  <div className="settings-section" id="section-model" hidden={section !== "model"}>
                    <ModelCard
                      form={form.form}
                      apiKey={form.apiKey}
                      keySaved={form.keySaved}
                      busy={form.busy}
                      action={form.action}
                      answer={testAnswer}
                      models={form.models}
                      onApiKey={form.setApiKey}
                      onPatch={form.patch}
                      onTest={handleTest}
                      onLoadModels={() => void form.loadModels()}
                    />
                  </div>

                  <div className="settings-section" id="section-voice" hidden={section !== "voice"}>
                    <ModelDownloadCard
                      view={model.view}
                      busy={model.busy}
                      notice={model.notice}
                      onDownload={() => void model.download()}
                      onCancel={() => void model.cancel()}
                      onRemove={() => void model.remove()}
                    />
                  </div>

                  <div
                    className="settings-section"
                    id="section-session"
                    hidden={section !== "session"}
                  >
                    <SessionCard control={session} />
                  </div>

                  <div
                    className="settings-section"
                    id="section-devices"
                    hidden={section !== "devices"}
                  >
                    <DevicesCard
                      form={form.form}
                      loaded={form.loaded}
                      patch={form.patch}
                      loadDevices={form.loadDevices}
                    />
                  </div>

                  <div
                    className="settings-section"
                    id="section-profile"
                    hidden={section !== "profile"}
                  >
                    <ProfileCard form={form.form} patch={form.patch} />
                  </div>

                  <div
                    className="settings-section"
                    id="section-behavior"
                    hidden={section !== "behavior"}
                  >
                    <BehaviorCard form={form.form} patch={form.patch} />
                  </div>

                  <div className="settings-section" id="section-save" hidden={section !== "save"}>
                    <div className="settings-block">
                      <div className="field-group">
                        <p className="field-group-title">保存范围</p>
                        <p className="muted small">
                          模型接口、语音模型、面试会话、设备、简历与项目资料、回答行为都写在同一份配置里：
                          点下方固定栏的「保存全部配置」一次性落盘，没有逐项保存。改动未落盘时界面不给提示，
                          切换区块也不会丢，关闭窗口前记得保存。
                        </p>
                      </div>
                      <div className="field-group">
                        <p className="field-group-title">环境自检</p>
                        <p className="muted small">
                          跑一次本地诊断，把版本、语音模型、配置、会话数据库、音频设备与
                          日志尾部的告警汇总出来。和命令行 `--doctor` 的报告内容完全一致。
                          遇到问题时把这段贴出来，比截图更容易定位。
                        </p>
                        <div className="action-row">
                          <button
                            type="button"
                            className="secondary"
                            onClick={() => void runDoctor()}
                            disabled={doctorBusy}
                          >
                            {doctorBusy ? "自检中…" : "运行环境自检"}
                          </button>
                          {/* 自检报告提到旧模型目录时才给这个动作：诊断发现问题
                              却不提供动作，等于把活推回给用户。 */}
                          {doctorText.includes("旧模型目录") && (
                            <button
                              type="button"
                              className="ghost"
                              onClick={() => void cleanLegacy()}
                            >
                              清理旧模型目录
                            </button>
                          )}
                        </div>
                        {doctorText && (
                          <pre
                            className="settings-mono doctor-output"
                            aria-label="环境自检结果"
                          >
                            {doctorText}
                          </pre>
                        )}
                      </div>

                      <div className="field-group">
                        <p className="field-group-title">日志</p>
                        <div className="settings-kv">
                          <span className="settings-kv-key">日志文件</span>
                          <span className="settings-kv-val">
                            <code className="settings-mono">logs\hint.log</code>
                          </span>
                          <span className="settings-kv-key">打开方式</span>
                          <span className="settings-kv-val">下方固定栏的按钮，无需找到目录</span>
                        </div>
                        <p className="muted small">
                          遇到问题时把该文件末尾的报错贴给开发者，比截图更容易定位。
                        </p>
                      </div>
                    </div>
                  </div>
                </div>

                <footer className="settings-footer">
                  <div className="settings-footer-actions">
                    <button
                      type="button"
                      className="primary"
                      onClick={() => void form.save()}
                      disabled={form.busy}
                    >
                      {form.action === "save" ? "保存中…" : "保存全部配置"}
                    </button>
                    <button
                      type="button"
                      className="secondary"
                      onClick={() => void openLogDir()}
                    >
                      打开日志目录
                    </button>
                  </div>
                  {form.status && (
                    <p className={statusToneClass(deriveStatusTone(form.status))}>{form.status}</p>
                  )}
                </footer>
              </div>
            </div>
          )}
        </div>
      ) : (
        <div
          className="tabpanel"
          role="tabpanel"
          id="tabpanel-review"
          aria-labelledby="tab-review"
        >
        <SessionReview
          sessions={sessions.sessions}
          totalCount={sessions.totalCount}
          query={sessions.query}
          onQueryChange={sessions.setQuery}
          searching={sessions.searching}
          isSearching={sessions.isSearching}
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
        </div>
      )}
    </div>
  );
}

/** 协议短名；只用于 titlebar 的技术值，与设置页下拉文案保持同一套叫法。 */
function providerShort(provider: Settings["provider"]): string {
  switch (provider) {
    case "anthropic":
      return "Anthropic";
    case "gemini":
      return "Gemini";
    default:
      return "OpenAI 兼容";
  }
}