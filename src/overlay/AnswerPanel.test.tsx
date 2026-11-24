import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import AnswerPanel from "./AnswerPanel";
import type { AnswerHistoryEntry, AnswerState } from "./types";

function makeAnswer(overrides: Partial<AnswerState> = {}): AnswerState {
  return {
    text: "",
    streaming: false,
    done: false,
    model: "",
  configuredModel: "",
    ttftMs: null,
    question: "",
    pending: 0,
    error: "",
    retryable: false,
    muted: false,
    ...overrides,
  };
}

function makeHistory(): AnswerHistoryEntry[] {
  return [
    { id: 1, question: "问题一", text: "答案一", model: "m1" },
    { id: 2, question: "问题二", text: "", model: "m2" },
    { id: 3, question: "", text: "答案三", model: "m3" },
  ];
}

type PanelProps = {
  answer?: AnswerState;
  history?: AnswerHistoryEntry[];
  answeredTotal?: number;
  onRegenerate?: () => void;
  onClear?: () => void;
  onDismissError?: () => void;
};

function renderPanel(props: PanelProps = {}) {
  const onRegenerate = props.onRegenerate ?? vi.fn();
  const onClear = props.onClear ?? vi.fn();
  const onDismissError = props.onDismissError ?? vi.fn();
  const utils = render(
    <AnswerPanel
      answer={props.answer ?? makeAnswer()}
      history={props.history ?? []}
      answeredTotal={props.answeredTotal ?? 0}
      onRegenerate={onRegenerate}
      onClear={onClear}
      onDismissError={onDismissError}
    />,
  );
  return { ...utils, onRegenerate, onClear, onDismissError };
}

/** 让 jsdom 里的滚动容器像真的可滚动：伪造 scrollHeight/clientHeight。 */
function primeScrollable(
  node: HTMLElement,
  { scrollHeight = 1000, clientHeight = 100 } = {},
) {
  Object.defineProperty(node, "scrollHeight", {
    configurable: true,
    value: scrollHeight,
  });
  Object.defineProperty(node, "clientHeight", {
    configurable: true,
    value: clientHeight,
  });
}

function pressKey(target: EventTarget, key: string): boolean {
  const event = new KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
  });
  return target.dispatchEvent(event);
}

describe("AnswerPanel —— 空态与占位文案分支", () => {
  it("没有答案、不在流式、没有错误时给默认引导文案", () => {
    renderPanel();
    expect(
      screen.getByText(/检测到面试官提问后，答案会流式出现在这里/),
    ).toBeInTheDocument();
    // 无文本时「清空」按钮不该出现
    expect(screen.queryByRole("button", { name: "清空" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重新生成" })).toBeInTheDocument();
  });

  it("流式但没有文本时显示「正在等待模型输出」而不是引导文案", () => {
    renderPanel({ answer: makeAnswer({ streaming: true }) });
    expect(screen.getByText("正在等待模型输出…")).toBeInTheDocument();
    expect(
      screen.queryByText(/检测到面试官提问后，答案会流式出现在这里/),
    ).not.toBeInTheDocument();
  });

  it("muted 且无文本时显示「本轮答案已清空」的两行占位", () => {
    const { container } = renderPanel({
      // 同时带 error：muted 分支仍优先，不该退回「本次没有生成内容」那句
      answer: makeAnswer({ muted: true, done: true, error: "超时" }),
    });
    const placeholder = container.querySelector(".placeholder");
    expect(placeholder?.textContent).toContain(
      "本轮答案已清空，模型后续生成的内容不再显示。",
    );
    expect(placeholder?.textContent).toContain("需要完整答案时点「重新生成」。");
    // muted 分支优先于「错误后无内容」占位：即便同时有 error 也不显示那句
    expect(screen.queryByText(/本次没有生成内容/)).not.toBeInTheDocument();
  });

  it("有错误但没有文本时给出「没有生成内容」的占位", () => {
    renderPanel({ answer: makeAnswer({ error: "网络中断", done: true }) });
    expect(
      screen.getByText("本次没有生成内容，可按上方提示处理后重试。"),
    ).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("网络中断");
  });
});

describe("AnswerPanel —— 流式与正文", () => {
  it("流式中显示 caret、隐藏的「正在生成」与当前正文", () => {
    const { container } = renderPanel({
      answer: makeAnswer({ text: "第一句", streaming: true, question: "介绍项目" }),
    });
    const body = container.querySelector(".answer-text");
    expect(body).toHaveTextContent("第一句");
    expect(container.querySelector(".caret")).not.toBeNull();
    expect(container.querySelector(".streaming-dot")).not.toBeNull();
    expect(screen.getByText("正在生成")).toHaveClass("visually-hidden");
  });

  it("非流式时不渲染 caret 与流式指示点", () => {
    const { container } = renderPanel({
      answer: makeAnswer({ text: "完整答案", done: true }),
    });
    expect(container.querySelector(".caret")).toBeNull();
    expect(container.querySelector(".streaming-dot")).toBeNull();
    expect(screen.queryByText("正在生成")).not.toBeInTheDocument();
  });

  it("有文本时出现「清空」按钮，点击回到 onClear", async () => {
    const user = userEvent.setup();
    const { onClear } = renderPanel({ answer: makeAnswer({ text: "有内容" }) });
    await user.click(screen.getByRole("button", { name: "清空" }));
    expect(onClear).toHaveBeenCalledTimes(1);
  });

  it("「重新生成」始终可点并回调 onRegenerate", async () => {
    const user = userEvent.setup();
    const { onRegenerate } = renderPanel({ answeredTotal: 2 });
    await user.click(screen.getByRole("button", { name: "重新生成" }));
    expect(onRegenerate).toHaveBeenCalledTimes(1);
  });
});

describe("AnswerPanel —— 错误条", () => {
  it("retryable=true 时同时有「重试」与「关闭」，各自回调", async () => {
    const user = userEvent.setup();
    const { onRegenerate, onDismissError } = renderPanel({
      answer: makeAnswer({ error: "模型超时", retryable: true, done: true }),
    });
    const alert = screen.getByRole("alert");
    expect(alert).toHaveAttribute("title", "模型超时");
    expect(alert).toHaveTextContent("模型超时");

    await user.click(screen.getByRole("button", { name: "重试" }));
    expect(onRegenerate).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "关闭" }));
    expect(onDismissError).toHaveBeenCalledTimes(1);
  });

  it("retryable=false 时只有「关闭」，没有「重试」", () => {
    renderPanel({
      answer: makeAnswer({ error: "未配置模型", retryable: false }),
    });
    expect(screen.getByRole("button", { name: "关闭" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "重试" })).not.toBeInTheDocument();
  });

  it("没有错误时不渲染错误条", () => {
    renderPanel({ answer: makeAnswer({ text: "答案" }) });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});

describe("AnswerPanel —— 问题行与排队徽标", () => {
  it("有问题时显示「问：…」，pending>0 显示排队数", () => {
    const { container } = renderPanel({
      answer: makeAnswer({ question: "说说你的项目", pending: 2 }),
    });
    const question = container.querySelector(".overlay-question");
    expect(question).toHaveTextContent("问：说说你的项目");
    expect(question).toHaveAttribute("title", "说说你的项目");
    const badge = container.querySelector(".pending-badge");
    expect(badge).toHaveTextContent("排队 2");
    expect(badge).toHaveAttribute("title", "还有 2 个问题等待回答");
  });

  it("问题未知但有待答时用「当前问题」占位", () => {
    const { container } = renderPanel({
      answer: makeAnswer({ question: "", pending: 1 }),
    });
    expect(container.querySelector(".overlay-question")).toHaveTextContent("当前问题");
    expect(container.querySelector(".pending-badge")).toHaveTextContent("排队 1");
  });

  it("既无问题也无排队时不渲染问题行", () => {
    const { container } = renderPanel({ answer: makeAnswer({ text: "答案" }) });
    expect(container.querySelector(".overlay-question-row")).toBeNull();
  });
});

describe("AnswerPanel —— 本场已答计数", () => {
  it("answeredTotal 加上当前这一轮有文本的结果", () => {
    renderPanel({ answeredTotal: 3, answer: makeAnswer({ text: "当前答案" }) });
    expect(screen.getByText("本场已答 4 题")).toBeInTheDocument();
  });

  it("当前没有文本时只算 answeredTotal", () => {
    renderPanel({ answeredTotal: 3 });
    expect(screen.getByText("本场已答 3 题")).toBeInTheDocument();
  });

  it("answeredTotal=0 且无文本时整个计数不显示", () => {
    renderPanel({ answeredTotal: 0 });
    expect(screen.queryByText(/本场已答/)).not.toBeInTheDocument();
  });

  it("只有空白的当前文本不算已答（trim 后为空）", () => {
    renderPanel({ answeredTotal: 0, answer: makeAnswer({ text: "   \n " }) });
    expect(screen.queryByText(/本场已答/)).not.toBeInTheDocument();
    // 空白文本也不触发「清空」按钮
    expect(screen.queryByRole("button", { name: "清空" })).not.toBeInTheDocument();
  });
});

describe("AnswerPanel —— 历史时间线", () => {
  it("历史倒序渲染：最新的在最上面", () => {
    const { container } = renderPanel({ history: makeHistory() });
    const questions = Array.from(container.querySelectorAll(".block-question")).map(
      (node) => node.textContent,
    );
    // history 按时间先后追加，渲染时整体反转：最后归档的一条排最上面
    expect(questions).toEqual(["（未记录问题）", "问：问题二", "问：问题一"]);
    const answers = Array.from(container.querySelectorAll(".block-answer")).map(
      (node) => node.textContent,
    );
    expect(answers).toEqual(["答案三", "（空答案）", "答案一"]);
    expect(screen.getByText("更早的回答")).toBeInTheDocument();
  });

  it("每条历史区块的 title 带上原文，便于悬停回看", () => {
    const { container } = renderPanel({ history: makeHistory() });
    const blocks = Array.from(container.querySelectorAll(".answer-block.past"));
    // 最上面是最新归档的第 3 条（问题为空、有答案）
    expect(blocks[0].querySelector(".block-question")).toHaveAttribute("title", "");
    expect(blocks[0].querySelector(".block-answer")).toHaveAttribute("title", "答案三");
    // 第二条：有 question、空 text
    expect(blocks[1].querySelector(".block-question")).toHaveAttribute(
      "title",
      "问题二",
    );
    expect(blocks[1].querySelector(".block-answer")).toHaveAttribute("title", "");
  });

  it("没有历史时不渲染时间线", () => {
    const { container } = renderPanel();
    expect(container.querySelector(".answer-history-timeline")).toBeNull();
    expect(screen.queryByText("更早的回答")).not.toBeInTheDocument();
  });
});

describe("AnswerPanel —— 方向键滚动", () => {
  it("ArrowUp / ArrowDown 在答案区滚动 64px 并阻止默认行为", () => {
    const { container } = renderPanel({ answer: makeAnswer({ text: "长答案" }) });
    const body = container.querySelector(".overlay-answer-body") as HTMLElement;
    primeScrollable(body);
    body.scrollTop = 500;

    const upConsumed = pressKey(document.body, "ArrowUp");
    expect(body.scrollTop).toBe(436);
    // dispatchEvent 返回 false 说明 preventDefault 被调用过
    expect(upConsumed).toBe(false);

    const downConsumed = pressKey(document.body, "ArrowDown");
    expect(body.scrollTop).toBe(500);
    expect(downConsumed).toBe(false);
  });

  it("滚到边界后不再阻止默认行为（回到顶部/到达底部）", () => {
    const { container } = renderPanel({ answer: makeAnswer({ text: "长答案" }) });
    const body = container.querySelector(".overlay-answer-body") as HTMLElement;
    primeScrollable(body);

    body.scrollTop = 0;
    expect(pressKey(document.body, "ArrowUp")).toBe(true);
    expect(body.scrollTop).toBe(0);

    body.scrollTop = 900; // max = 1000 - 100
    expect(pressKey(document.body, "ArrowDown")).toBe(true);
    expect(body.scrollTop).toBe(900);
  });

  it("内容不足一屏（max<=0）时方向键完全不处理", () => {
    const { container } = renderPanel();
    const body = container.querySelector(".overlay-answer-body") as HTMLElement;
    primeScrollable(body, { scrollHeight: 100, clientHeight: 100 });
    body.scrollTop = 0;
    expect(pressKey(document.body, "ArrowDown")).toBe(true);
    expect(body.scrollTop).toBe(0);
  });

  it("焦点在转录面板内时不劫持方向键", () => {
    const { container } = renderPanel({ answer: makeAnswer({ text: "答案" }) });
    const body = container.querySelector(".overlay-answer-body") as HTMLElement;
    primeScrollable(body);
    body.scrollTop = 500;

    const transcript = document.createElement("div");
    transcript.className = "overlay-transcript";
    const inner = document.createElement("span");
    transcript.appendChild(inner);
    document.body.appendChild(transcript);

    const consumed = pressKey(inner, "ArrowDown");
    // 事件目标在 .overlay-transcript 内：滚动位置保持不动，也不阻止默认
    expect(body.scrollTop).toBe(500);
    expect(consumed).toBe(true);

    document.body.removeChild(transcript);
  });

  it("非方向键（如 Escape）不做滚动处理", () => {
    const { container } = renderPanel({ answer: makeAnswer({ text: "答案" }) });
    const body = container.querySelector(".overlay-answer-body") as HTMLElement;
    primeScrollable(body);
    body.scrollTop = 500;
    expect(pressKey(document.body, "Escape")).toBe(true);
    expect(body.scrollTop).toBe(500);
  });
});