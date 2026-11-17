import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import CaptureOverlay from "./CaptureOverlay";
import Dashboard from "./Dashboard";
import Overlay from "./Overlay";
import "./App.css";

function currentLabel(): string {
  // 开发预览：浏览器里用 ?window=overlay / ?window=capture 单独调试某个窗口。
  const preview = new URLSearchParams(window.location.search).get("window");
  if (preview === "overlay" || preview === "capture") return preview;
  try {
    return getCurrentWindow().label;
  } catch {
    // 纯浏览器预览时没有 Tauri 窗口，按设置页渲染。
    return "main";
  }
}

const label = currentLabel();

function Root() {
  if (label === "overlay") return <Overlay />;
  if (label === "capture") return <CaptureOverlay />;
  return <Dashboard />;
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);