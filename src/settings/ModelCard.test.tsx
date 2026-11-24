import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import ModelCard from "./ModelCard";
import { EMPTY_SETTINGS, type FormAction, type Settings } from "./types";

function makeForm(overrides: Partial<Settings> = {}): Settings {
  return {
    ...EMPTY_SETTINGS,
    provider: "openai",
    baseUrl: "https://api.deepseek.com/v1",
    model: "deepseek-chat",
    ...overrides,
  };
}

type CardProps = {
  form?: Settings;
  apiKey?: string;
  keySaved?: boolean;
  busy?: boolean;
  action?: FormAction;
  answer?: string;
  models?: string[];
  onApiKey?: (value: string) => void;
  onPatch?: (next: Partial<Settings>) => void;
  onTest?: () => Promise<{ text: string; model: string } | null>;
  onLoadModels?: () => void;
};

function renderCard(props: CardProps = {}) {
  const onApiKey = props.onApiKey ?? vi.fn();
  const onPatch = props.onPatch ?? vi.fn();
  const onTest = props.onTest ?? vi.fn(() => Promise.resolve(null));
  const onLoadModels = props.onLoadModels ?? vi.fn();
  const utils = render(
    <ModelCard
      form={props.form ?? makeForm()}
      apiKey={props.apiKey ?? ""}
      keySaved={props.keySaved ?? false}
      busy={props.busy ?? false}
      action={props.action ?? ""}
      answer={props.answer ?? ""}
      models={props.models ?? []}
      onApiKey={onApiKey}
      onPatch={onPatch}
      onTest={onTest}
      onLoadModels={onLoadModels}
    />,
  );
  return { ...utils, onApiKey, onPatch, onTest, onLoadModels };
}

describe("ModelCard —— 当前生效概览", () => {
  it("展示协议中文名、Base URL、模型名与 Key 状态；空值显示「未填写」", () => {
    const { container } = renderCard({
      form: makeForm({ provider: "anthropic", baseUrl: "", model: "" }),
      keySaved: true,
    });
    const kv = container.querySelector(".settings-kv") as HTMLElement;
    expect(within(kv).getByText("Anthropic")).toBeInTheDocument();
    expect(within(kv).getAllByText("未填写")).toHaveLength(2);
    expect(within(kv).getByText("已保存")).toHaveClass("badge-success");
  });

  it("Key 未保存时徽标为「未保存」", () => {
    const { container } = renderCard({ keySaved: false });
    const kv = container.querySelector(".settings-kv") as HTMLElement;
    expect(within(kv).getByText("未保存")).toHaveClass("badge-neutral");
    expect(within(kv).queryByText("已保存")).not.toBeInTheDocument();
  });

  it("协议中文名映射：openai → OpenAI 兼容，gemini → Gemini", () => {
    const { container, unmount } = renderCard({
      form: makeForm({ provider: "openai", baseUrl: "https://example.com/x", model: "x" }),
    });
    const kv = container.querySelector(".settings-kv") as HTMLElement;
    expect(within(kv).getByText("OpenAI 兼容")).toBeInTheDocument();
    unmount();

    const second = renderCard({ form: makeForm({ provider: "gemini" }) });
    const secondKv = second.container.querySelector(".settings-kv") as HTMLElement;
    expect(within(secondKv).getByText("Gemini")).toBeInTheDocument();
  });
});

describe("ModelCard —— 三协议 segment 切换", () => {
  it("点 Anthropic 分段调 onPatch({provider:'anthropic'})", async () => {
    const user = userEvent.setup();
    const { onPatch } = renderCard();

    const anthropic = screen.getByRole("button", { name: "Anthropic" });
    expect(anthropic).toHaveAttribute("aria-pressed", "false");
    expect(screen.getByRole("button", { name: "OpenAI 兼容" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    await user.click(anthropic);
    expect(onPatch).toHaveBeenCalledWith({ provider: "anthropic" });
  });

  it("点 Gemini 分段同样只 patch provider，并显示对应说明", async () => {
    const user = userEvent.setup();
    const { onPatch } = renderCard({ form: makeForm({ provider: "gemini" }) });

    expect(screen.getByText("Gemini generateContent 协议。")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "OpenAI 兼容" }));
    expect(onPatch).toHaveBeenCalledWith({ provider: "openai" });
  });

  it("预设芯片点击时一次 patch 协议 + Base URL + 模型", async () => {
    const user = userEvent.setup();
    const { onPatch } = renderCard();

    await user.click(screen.getByRole("button", { name: "Claude" }));
    expect(onPatch).toHaveBeenCalledWith({
      provider: "anthropic",
      baseUrl: "https://api.anthropic.com",
      model: "claude-sonnet-5-5",
    });
  });

  it("表单与某个预设完全一致时该芯片 aria-pressed=true", () => {
    renderCard({
      form: makeForm({
        provider: "openai",
        baseUrl: "https://api.deepseek.com/v1",
        model: "deepseek-chat",
      }),
    });
    expect(screen.getByRole("button", { name: "DeepSeek" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByRole("button", { name: "OpenAI" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });
});

describe("ModelCard —— 刷新模型与测试连接", () => {
  it("刷新模型按钮触发 onLoadModels；action=models 时显示拉取中且禁用", async () => {
    const user = userEvent.setup();
    const { onLoadModels } = renderCard();

    await user.click(screen.getByRole("button", { name: "刷新模型" }));
    expect(onLoadModels).toHaveBeenCalledTimes(1);

    const loading = renderCard({ busy: true, action: "models" });
    const button = loading.getByRole("button", { name: /拉取中/ });
    expect(button).toBeDisabled();
  });

  it("测试连接触发 onTest，成功后渲染结果卡（模型名 + 耗时）", async () => {
    const user = userEvent.setup();
    const onTest = vi.fn(() =>
      Promise.resolve({ text: "我是模型", model: "deepseek-chat" }),
    );
    renderCard({ onTest, answer: "我是模型" });

    await user.click(screen.getByRole("button", { name: "测试连接" }));

    const result = await screen.findByRole("status");
    expect(result).toHaveClass("is-ok");
    expect(screen.getByText("连接成功")).toBeInTheDocument();
    expect(result).toHaveTextContent("模型 deepseek-chat");
    expect(result).toHaveTextContent(/耗时 \d+ms/);
    expect(screen.getByLabelText("测试连接返回内容")).toHaveTextContent("我是模型");
    expect(onTest).toHaveBeenCalledTimes(1);
  });

  it("测试连接返回 null 时结果卡标红并写「连接失败」", async () => {
    const user = userEvent.setup();
    renderCard({ onTest: vi.fn(() => Promise.resolve(null)) });

    await user.click(screen.getByRole("button", { name: "测试连接" }));

    const result = await screen.findByRole("status");
    expect(result).toHaveClass("is-error");
    expect(screen.getByText("连接失败")).toBeInTheDocument();
    expect(screen.queryByLabelText("测试连接返回内容")).not.toBeInTheDocument();
  });

  it("action=test 时按钮显示测试中且禁用", () => {
    renderCard({ busy: true, action: "test" });
    expect(screen.getByRole("button", { name: /测试中/ })).toBeDisabled();
  });

  it("卡片内不再渲染全局 status（避免与底部固定栏重复显示同一条提示）", () => {
    // 保存/拉取这类全局结果统一由面板底栏显示；卡片里再渲染一次会让用户
    // 看到两行一模一样的提示（真机实测确实出现过）。
    renderCard({});
    expect(screen.queryByText(/连接失败/)).not.toBeInTheDocument();
    expect(screen.queryByText(/保存失败/)).not.toBeInTheDocument();
  });
});

describe("ModelCard —— Key 输入与显隐", () => {
  it("Key 默认 password，点显隐按钮切到 text 并换 aria-label", async () => {
    const user = userEvent.setup();
    renderCard({ apiKey: "sk-secret" });

    const input = screen.getByLabelText("API Key") as HTMLInputElement;
    expect(input.type).toBe("password");
    expect(input.value).toBe("sk-secret");

    await user.click(screen.getByRole("button", { name: "显示 API Key" }));
    expect(input.type).toBe("text");
    expect(screen.getByRole("button", { name: "隐藏 API Key" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );

    await user.click(screen.getByRole("button", { name: "隐藏 API Key" }));
    expect(input.type).toBe("password");
  });

  it("Key 输入变化回调 onApiKey", () => {
    const onApiKey = vi.fn();
    renderCard({ onApiKey });
    fireEvent.change(screen.getByLabelText("API Key"), {
      target: { value: "sk-新输入" },
    });
    expect(onApiKey).toHaveBeenLastCalledWith("sk-新输入");
  });

  it("已保存 Key 时占位文案提示留空不修改", () => {
    renderCard({ keySaved: true });
    expect(screen.getByLabelText("API Key")).toHaveAttribute(
      "placeholder",
      "已保存，留空则不修改",
    );
  });
});

describe("ModelCard —— 模型下拉与 Base URL", () => {
  it("模型下拉 datalist 含接口返回的每个模型，同时保留手输", () => {
    const { container } = renderCard({
      models: ["deepseek-chat", "deepseek-reasoner"],
    });
    const input = screen.getByLabelText("模型名");
    expect(input).toHaveAttribute("list", "model-options");
    expect(input).toHaveAttribute("placeholder", "点「刷新模型」从接口拉取");

    const options = Array.from(container.querySelectorAll("#model-options option")).map(
      (option) => option.getAttribute("value"),
    );
    expect(options).toEqual(["deepseek-chat", "deepseek-reasoner"]);
    expect(screen.getByText("已拉取 2 个可选模型")).toBeInTheDocument();
  });

  it("没有模型列表时提示可手动填写", () => {
    renderCard({ models: [] });
    expect(screen.getByText("未拉取列表，可手动填写")).toBeInTheDocument();
  });

  it("手输模型名走 onPatch({model})", () => {
    const onPatch = vi.fn();
    renderCard({ onPatch });
    fireEvent.change(screen.getByLabelText("模型名"), {
      target: { value: "gpt-4o" },
    });
    expect(onPatch).toHaveBeenLastCalledWith({ model: "gpt-4o" });
  });

  it("手输 Base URL 逐字符走 onPatch({baseUrl})", () => {
    const onPatch = vi.fn();
    renderCard({ onPatch, form: makeForm({ baseUrl: "https://api." }) });
    fireEvent.change(screen.getByLabelText("Base URL"), {
      target: { value: "https://api.openai.com" },
    });
    expect(onPatch).toHaveBeenLastCalledWith({ baseUrl: "https://api.openai.com" });
  });

  it("Base URL 失焦时规范化并回写", () => {
    const onPatch = vi.fn();
    renderCard({ onPatch, form: makeForm({ baseUrl: "api.openai.com" }) });

    fireEvent.blur(screen.getByLabelText("Base URL"));
    expect(onPatch).toHaveBeenCalledWith({ baseUrl: "https://api.openai.com/v1" });
  });

  it("粘贴 Anthropic 域名失焦时同时切协议并清掉端点后缀", () => {
    const onPatch = vi.fn();
    renderCard({
      onPatch,
      form: makeForm({ baseUrl: "https://api.anthropic.com/messages" }),
    });

    fireEvent.blur(screen.getByLabelText("Base URL"));
    expect(onPatch).toHaveBeenCalledWith({
      baseUrl: "https://api.anthropic.com",
      provider: "anthropic",
    });
  });

  it("带版本段 + 端点的 Anthropic 写法只剥端点，保留版本段", () => {
    const onPatch = vi.fn();
    renderCard({
      onPatch,
      form: makeForm({ baseUrl: "https://api.anthropic.com/v1/messages" }),
    });

    fireEvent.blur(screen.getByLabelText("Base URL"));
    expect(onPatch).toHaveBeenCalledWith({
      baseUrl: "https://api.anthropic.com/v1",
      provider: "anthropic",
    });
  });

  it("已规范且协议一致时失焦不产生多余 patch", () => {
    const onPatch = vi.fn();
    renderCard({ onPatch, form: makeForm({ baseUrl: "https://api.deepseek.com/v1" }) });
    fireEvent.blur(screen.getByLabelText("Base URL"));
    expect(onPatch).not.toHaveBeenCalled();
  });
});

describe("ModelCard —— M5 截图提示", () => {
  it("点「截图问答」后出现 M5 说明，且带上当前模型名", async () => {
    const user = userEvent.setup();
    renderCard({ form: makeForm({ model: "qwen-plus" }) });

    expect(screen.queryByText(/截图问答尚未启用/)).not.toBeInTheDocument();
    expect(screen.getByText("M5 预留")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /截图问答/ }));
    await waitFor(() => {
      expect(screen.getByText(/截图问答尚未启用/)).toBeInTheDocument();
    });
    expect(screen.getByText(/当前模型「qwen-plus」/)).toBeInTheDocument();
  });

  it("模型名为空时 M5 说明不带模型推断那句", async () => {
    const user = userEvent.setup();
    renderCard({ form: makeForm({ model: "" }) });

    await user.click(screen.getByRole("button", { name: /截图问答/ }));
    const hint = await screen.findByText(/截图问答尚未启用/);
    expect(hint.textContent).not.toContain("当前模型");
  });
});