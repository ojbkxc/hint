import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import BehaviorCard from "./BehaviorCard";
import { EMPTY_SETTINGS, type Settings } from "./types";

function makeForm(overrides: Partial<Settings> = {}): Settings {
  return { ...EMPTY_SETTINGS, ...overrides };
}

function renderCard(form: Settings = makeForm()) {
  const patch = vi.fn();
  const utils = render(<BehaviorCard form={form} patch={patch} />);
  return {
    ...utils,
    patch,
    patchLast: () => patch.mock.calls[patch.mock.calls.length - 1]?.[0],
  };
}

describe("BehaviorCard —— 答案风格", () => {
  it("默认选中口语短答，切换为提纲时 patch answerStyle", async () => {
    const user = userEvent.setup();
    const { patchLast } = renderCard();

    const select = screen.getByLabelText("答案风格") as HTMLSelectElement;
    expect(select.value).toBe("concise");
    expect(
      screen.getByRole("option", { name: "口语短答（可直接照念）" }),
    ).toBeInTheDocument();

    await user.selectOptions(select, "outline");
    expect(patchLast()).toEqual({ answerStyle: "outline" });
  });

  it("表单里是 outline 时下拉跟着显示提纲", () => {
    renderCard(makeForm({ answerStyle: "outline" }));
    expect((screen.getByLabelText("答案风格") as HTMLSelectElement).value).toBe(
      "outline",
    );
  });
});

describe("BehaviorCard —— 触发方式", () => {
  it("自动模式有专门说明；切手动后 patch triggerMode 并换说明文案", async () => {
    const user = userEvent.setup();
    const { patchLast } = renderCard();

    const select = screen.getByLabelText("触发方式") as HTMLSelectElement;
    expect(select.value).toBe("auto");
    expect(screen.getByText(/自动模式：面试官说完一句话/)).toBeInTheDocument();

    await user.selectOptions(select, "manual");
    expect(patchLast()).toEqual({ triggerMode: "manual" });
  });

  it("表单为 manual 时渲染手动模式说明与快捷键提示", () => {
    renderCard(makeForm({ triggerMode: "manual" }));
    expect(screen.getByText(/手动模式：不会自动生成答案/)).toBeInTheDocument();
    expect(screen.queryByText(/自动模式：面试官说完一句话/)).not.toBeInTheDocument();
  });
});

describe("BehaviorCard —— 不透明度滑块", () => {
  it("默认 88%，滑块值等于百分比且限制在 20–100", () => {
    renderCard();
    const slider = screen.getByLabelText(/浮层不透明度/) as HTMLInputElement;
    expect(slider.value).toBe("88");
    expect(slider.min).toBe("20");
    expect(slider.max).toBe("100");
    expect(slider.step).toBe("1");
    expect(screen.getByText("88%")).toBeInTheDocument();
  });

  it("拖动滑块把百分比换算成 0.2–1 的小数并 patch overlayOpacity", () => {
    const { patchLast } = renderCard();
    const slider = screen.getByLabelText(/浮层不透明度/);

    fireEvent.change(slider, { target: { value: "55" } });
    expect(patchLast()).toEqual({ overlayOpacity: 0.55 });

    fireEvent.change(slider, { target: { value: "100" } });
    expect(patchLast()).toEqual({ overlayOpacity: 1 });

    fireEvent.change(slider, { target: { value: "20" } });
    expect(patchLast()).toEqual({ overlayOpacity: 0.2 });
  });

  it("表单里存了越界或不合法的不透明度时按 clamp / 默认值展示", () => {
    // 现在有两个滑块（不透明度 + 字号），默认字号 100% 与不透明度 100% 会撞文本，
    // 必须按 id 精确定位到不透明度那一条的数值。
    const opacityValue = (root: HTMLElement) =>
      root.querySelector("label[for=\"behavior-opacity\"] .slider-value")?.textContent;

    const { unmount, container } = renderCard(makeForm({ overlayOpacity: 1.6 }));
    expect(opacityValue(container)).toBe("100%");
    unmount();

    const second = renderCard(makeForm({ overlayOpacity: 0.05 }));
    expect(opacityValue(second.container)).toBe("20%");
    second.unmount();

    const third = renderCard(makeForm({ overlayOpacity: Number.NaN }));
    expect(opacityValue(third.container)).toBe("88%");
  });

  it("字号滑块默认 100%，保存时按比例下发 overlayFontScale", () => {
    const { container, patchLast } = renderCard(makeForm());
    expect(container.querySelector("label[for=\"behavior-font-scale\"] .slider-value")?.textContent).toBe(
      "100%",
    );
    const slider = container.querySelector<HTMLInputElement>("#behavior-font-scale");
    expect(slider).not.toBeNull();
    // 150% → 1.5
    fireEvent.change(slider!, { target: { value: "150" } });
    expect(patchLast()).toEqual({ overlayFontScale: 1.5 });
  });
});

describe("BehaviorCard —— 热键清单", () => {
  it("列出全部快捷键，含 M5 截图预留", () => {
    renderCard();
    expect(screen.getByText("Ctrl+Shift+H")).toBeInTheDocument();
    expect(screen.getByText("Ctrl+Shift+A")).toBeInTheDocument();
    expect(screen.getByText("Ctrl+Shift+R")).toBeInTheDocument();
    expect(screen.getByText("Ctrl+Shift+S")).toBeInTheDocument();
    expect(screen.getByText("Esc")).toBeInTheDocument();
    expect(screen.getByText("截图（M5）")).toBeInTheDocument();
  });
});