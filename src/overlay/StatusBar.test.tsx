import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import StatusBar from "./StatusBar";
import type { LevelsState, SpeechState } from "./types";

type BarProps = {
  levels?: LevelsState;
  model?: string;
  ttftMs?: number | null;
  streaming?: boolean;
  speech?: SpeechState;
};

function renderBar(props: BarProps = {}) {
  return render(
    <StatusBar
      levels={props.levels ?? { mic: 0, system: 0 }}
      model={props.model ?? ""}
      ttftMs={props.ttftMs ?? null}
      streaming={props.streaming ?? false}
      speech={props.speech ?? { joint: "neither" }}
    />,
  );
}

function fillWidth(tone: "mic" | "system", container: HTMLElement): string {
  const fill = container.querySelector(`.level-${tone} .level-fill`) as HTMLElement;
  return fill.style.width;
}

describe("StatusBar —— 电平条", () => {
  it("麦克风与系统声两条电平条都渲染，标签与 title 带百分比", () => {
    const { container } = renderBar({ levels: { mic: 0.5, system: 0.25 } });

    expect(container.querySelectorAll(".level")).toHaveLength(2);
    expect(container.querySelector(".level-mic .level-label")?.textContent).toBe("麦克风");
    expect(container.querySelector(".level-system .level-label")?.textContent).toBe("系统声");
    expect(container.querySelector(".level-mic")).toHaveAttribute("title", "麦克风电平 50%");
    expect(container.querySelector(".level-system")).toHaveAttribute("title", "系统声电平 25%");
  });

  it("宽度按 0–100% 换算，四舍五入", () => {
    const { container } = renderBar({ levels: { mic: 0.333, system: 0.666 } });
    expect(fillWidth("mic", container)).toBe("33%");
    expect(fillWidth("system", container)).toBe("67%");
  });

  it("clamp 到 0–1：负数按 0、超过 1 按 100%，且填充宽度最小 3%", () => {
    const { unmount } = renderBar({ levels: { mic: -0.4, system: 1.8 } });
    const container = document.body;
    expect(fillWidth("mic", container)).toBe("3%");
    expect(fillWidth("system", container)).toBe("100%");
    expect(container.querySelector(".level-mic")).toHaveAttribute("title", "麦克风电平 0%");
    expect(container.querySelector(".level-system")).toHaveAttribute("title", "系统声电平 100%");
    unmount();

    const second = renderBar({ levels: { mic: 0, system: 2 } });
    expect(fillWidth("mic", second.container)).toBe("3%");
    expect(fillWidth("system", second.container)).toBe("100%");
  });

  it("宽度地板 3%：低于它的电平被抬到 3%，高于则用真实百分比", () => {
    const bar = renderBar({ levels: { mic: 0.04, system: 0.02 } });
    expect(fillWidth("mic", bar.container)).toBe("4%");
    expect(fillWidth("system", bar.container)).toBe("3%");
    bar.unmount();
  });

  it("NaN 电平不会写出 NaN%，宽度落空由样式表兜底", () => {
    const second = renderBar({ levels: { mic: Number.NaN, system: 0.4 } });
    // NaN 会让 Math.max(3, NaN) 仍是 NaN，jsdom 拒绝非法宽度 → 空字符串（不写 NaN%）
    expect(fillWidth("mic", second.container)).toBe("");
    expect(second.container.querySelector(".level-fill")).not.toHaveStyle({
      width: "NaN%",
    });
    expect(fillWidth("system", second.container)).toBe("40%");
  });
});

describe("StatusBar —— 连接状态与模型名", () => {
  it("没有模型时显示「未配置模型」并给出 title 兜底（不是「未连接」）", () => {
    renderBar({ model: "" });
    const item = screen.getByText("未配置模型");
    expect(item).toHaveAttribute("title", "尚未在设置里选择模型");
  });

  it("有模型时显示模型名，title 为模型名本身", () => {
    renderBar({ model: "deepseek-chat" });
    const item = screen.getByText("deepseek-chat");
    expect(item).toHaveAttribute("title", "deepseek-chat");
    expect(screen.queryByText("未配置模型")).not.toBeInTheDocument();
  });
});

describe("StatusBar —— 首字延迟", () => {
  it("有 ttft 值时按秒显示（保留一位小数）", () => {
    renderBar({ ttftMs: 1250 });
    expect(screen.getByText("首字 1.3s")).toBeInTheDocument();
  });

  it("ttft 为 0 时显示 0.0s，而不是走流式兜底", () => {
    renderBar({ ttftMs: 0 });
    expect(screen.getByText("首字 0.0s")).toBeInTheDocument();
  });

  it("无 ttft 且流式中显示「首字 …」，空闲时显示「首字 —」", () => {
    const streaming = renderBar({ ttftMs: null, streaming: true });
    expect(streaming.getByText("首字 …")).toBeInTheDocument();
    streaming.unmount();

    renderBar({ ttftMs: null, streaming: false });
    expect(screen.getByText("首字 —")).toBeInTheDocument();
  });
});

describe("StatusBar —— 说话人徽标", () => {
  it("joint=neither 时不渲染徽标", () => {
    const { container } = renderBar({ speech: { joint: "neither" } });
    expect(container.querySelector(".speaking-badge")).toBeNull();
  });

  it.each([
    ["interviewer", "对方在说"],
    ["me", "你在说"],
    ["both", "双方同时"],
  ] as const)("joint=%s 时显示「%s」并带对应类名", (joint, text) => {
    const { container } = renderBar({ speech: { joint } });
    const badge = container.querySelector(".speaking-badge") as HTMLElement;
    expect(badge).toHaveClass(`speaking-${joint}`);
    expect(badge.textContent).toBe(text);
  });
});