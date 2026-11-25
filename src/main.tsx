import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import CaptureOverlay from "./CaptureOverlay";
import ErrorBoundary from "./ErrorBoundary";
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
    // 纯浏览器预览时没有 Tauri 窗口；未知 label 也走这条回落。
    // 落到 "main" 即渲染设置页（Dashboard）——不是"按设置页渲染"之外的窗口。
    return "main";
  }
}

const label = currentLabel();

function Root() {
  // 每个窗口各自包一层：一个窗口崩了不该影响另一个，且要能提示「出了什么事」。
  if (label === "overlay")
    return (
      <ErrorBoundary surface="overlay">
        <Overlay />
      </ErrorBoundary>
    );
  if (label === "capture")
    return (
      <ErrorBoundary surface="capture">
        <CaptureOverlay />
      </ErrorBoundary>
    );
  return (
    <ErrorBoundary surface="dashboard">
      <Dashboard />
    </ErrorBoundary>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);