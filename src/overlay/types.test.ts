import { describe, expect, it } from "vitest";
import { HINT_EVENTS } from "./types";

describe("HINT_EVENTS 事件契约", () => {
  it("与 integrator 对齐的全部事件名都在，且为 hint:// 前缀", () => {
    expect(HINT_EVENTS).toEqual({
      transcript: "hint://transcript",
      answerDelta: "hint://answer-delta",
      answerDone: "hint://answer-done",
      levels: "hint://levels",
      status: "hint://status",
      notice: "hint://notice",
      queue: "hint://queue",
      overlayOpacity: "hint://overlay-opacity",
      overlayFontScale: "hint://overlay-font-scale",
      // 新增事件：主题随保存实时下发给浮层（设置页与浮层共用一份配置，
      // 浮层是独立窗口，收不到主窗口的 DOM 变更，只能走事件）。
      overlayTheme: "hint://overlay-theme",
      speechEdge: "hint://speech-edge",
    });
  });

  it("事件名只含 Tauri 允许的字符（字母数字 - : / _）", () => {
    for (const name of Object.values(HINT_EVENTS)) {
      expect(name).toMatch(/^[a-zA-Z0-9:/_-]+$/);
    }
  });

  it("事件名两两互不相同", () => {
    const names = Object.values(HINT_EVENTS);
    expect(new Set(names).size).toBe(names.length);
  });
});
