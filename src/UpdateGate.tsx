import { useCallback, useEffect, useState } from "react";
import { safeInvoke } from "./lib/tauri";

/**
 * 启动时强制更新遮罩。
 *
 * ## 为什么是「强制」
 *
 * 产品会持续改判定逻辑与提示词，新旧版本混用在面试中可能给出不一致的答案；
 * 而用户不会主动去下载新版。启动时直接拦住、装完自动继续，是唯一能保证
 * 「跑的就是最新版」的做法。
 *
 * ## 但失败必须放行
 *
 * 这个组件挂在启动路径上——如果网络不通就永远打不开软件，那比版本旧严重得多。
 * 所以：
 * - 检查失败（网络/GitHub 不可达/解析异常）→ 直接放行，不打扰用户；
 * - 下载失败 → 显示原因 + 「重试」与「跳过本次」（跳过只对本次生效，下次启动
 *   仍会再检查）；
 * - 只有「确实发现新版本、且用户选择了更新」才拦住界面。
 */
type Phase = "checking" | "updating" | "error" | "done";

type UpdateInfo = {
  version: string;
  installerUrl: string;
  installerName: string;
  notes: string;
};

export default function UpdateGate({ onDone }: { onDone: () => void }) {
  const [phase, setPhase] = useState<Phase>("checking");
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [message, setMessage] = useState("");

  const run = useCallback(async () => {
    setPhase("checking");
    setMessage("");
    const result = await safeInvoke<UpdateInfo | null>("check_update");
    if (!result.ok) {
      // 命令层已经把所有失败折成 Ok(None)，走到这里说明是 IPC 异常：
      // 同样放行，不能让更新把应用卡死。
      onDone();
      return;
    }
    if (!result.data) {
      onDone();
      return;
    }
    // 有新版本：进入强制更新流程。
    setInfo(result.data);
    setPhase("updating");
    setMessage(`正在下载 ${result.data.version} 版安装包…`);

    const downloaded = await safeInvoke<string>("download_update", {
      info: result.data,
    });
    if (!downloaded.ok) {
      setPhase("error");
      setMessage(`下载失败：${downloaded.error}`);
      return;
    }
    setMessage("下载完成，正在安装…");
    const installed = await safeInvoke("install_update", {
      path: downloaded.data,
    });
    if (!installed.ok) {
      setPhase("error");
      setMessage(`安装失败：${installed.error}`);
      return;
    }
    // install_update 会让本进程退出，正常情况走不到这里。
    setPhase("done");
    setMessage("安装程序已启动，本窗口即将关闭。");
  }, [onDone]);

  useEffect(() => {
    void run();
  }, [run]);

  if (phase === "done") {
    return (
      <div className="update-gate">
        <div className="update-card">
          <h1>正在安装更新</h1>
          <p className="muted">{message}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="update-gate">
      <div className="update-card">
        <h1>{phase === "error" ? "更新失败" : "正在更新 hint"}</h1>
        {info && (
          <p className="update-version">
            当前版本之外的可用版本：
            <strong> {info.version}</strong>
          </p>
        )}
        <p className="muted update-message">{message || "正在检查更新…"}</p>

        {phase === "updating" && (
          <div className="update-progress" role="progressbar" aria-label="更新进度">
            <span className="update-progress-bar" />
          </div>
        )}

        {phase === "error" && (
          <div className="update-actions">
            <button type="button" className="primary" onClick={() => void run()}>
              重试
            </button>
            <button type="button" className="ghost" onClick={onDone}>
              跳过本次
            </button>
          </div>
        )}

        {phase === "checking" && <p className="muted small">检查更新通常只需一两秒。</p>}
      </div>
    </div>
  );
}
