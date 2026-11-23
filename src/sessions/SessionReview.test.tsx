import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import SessionReview from "./SessionReview";
import type { SessionDetail, SessionSummary } from "./types";

function makeSummary(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    id: "s1",
    startedAt: "2026-01-01 10:00:00",
    endedAt: "2026-01-01 10:02:30",
    model: "paraformer",
    note: "",
    ...overrides,
  };
}

function makeDetail(overrides: Partial<SessionDetail> = {}): SessionDetail {
  return {
    id: "s1",
    summary: makeSummary(),
    segments: [],
    answers: [],
    ...overrides,
  };
}

type Props = Partial<React.ComponentProps<typeof SessionReview>>;

function renderReview(props: Props = {}) {
  const handlers = {
    onRefresh: vi.fn(),
    onOpen: vi.fn(),
    onExport: vi.fn(),
    onDelete: vi.fn(),
    onDeleteAll: vi.fn(),
  };
  const utils = render(
    <SessionReview
      sessions={[]}
      loading={false}
      error=""
      selectedId=""
      detail={null}
      detailLoading={false}
      detailError=""
      notice=""
      {...handlers}
      {...props}
    />,
  );
  return { ...utils, ...handlers };
}

describe("SessionReview —— 左侧列表分支", () => {
  it("加载中显示三个骨架、刷新按钮禁用且文案变成「加载中…」", () => {
    const { container } = renderReview({ loading: true });
    expect(screen.getByRole("button", { name: "加载中…" })).toBeDisabled();
    expect(container.querySelectorAll(".skeleton").length).toBeGreaterThanOrEqual(3);
    expect(
      screen.getByText("正在读取会话列表…"),
    ).toBeInTheDocument();
    // 加载中不显示空态
    expect(screen.queryByText(/还没有会话记录/)).not.toBeInTheDocument();
  });

  it("加载完成后刷新按钮可点并回调 onRefresh", async () => {
    const user = userEvent.setup();
    const { onRefresh } = renderReview({ sessions: [makeSummary()] });
    const button = screen.getByRole("button", { name: "刷新" });
    expect(button).toBeEnabled();
    await user.click(button);
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it("列表错误优先于加载态与空态，显示 role=alert", () => {
    const { container } = renderReview({ error: "读取会话失败：磁盘只读", loading: true });
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("读取会话失败：磁盘只读");
    expect(container.querySelectorAll(".skeleton")).toHaveLength(0);
    expect(screen.queryByText(/还没有会话记录/)).not.toBeInTheDocument();
  });

  it("无记录时给空态文案，清空全部按钮禁用且不带条数", () => {
    renderReview({ sessions: [] });
    expect(
      screen.getByText(/还没有会话记录。完成一次面试后，这里会列出可回看的转录与问答/),
    ).toBeInTheDocument();
    const clearAll = screen.getByRole("button", { name: "清空全部" });
    expect(clearAll).toBeDisabled();
    expect(clearAll).toHaveAttribute("title", "永久删除本机全部会话记录");
  });

  it("有条目时清空全部带条数并可点击", async () => {
    const user = userEvent.setup();
    const { onDeleteAll } = renderReview({
      sessions: [makeSummary({ id: "a" }), makeSummary({ id: "b" })],
    });
    const clearAll = screen.getByRole("button", { name: "清空全部（2）" });
    expect(clearAll).toBeEnabled();
    expect(clearAll).toHaveAttribute("title", "永久删除本机全部 2 条会话记录");
    await user.click(clearAll);
    expect(onDeleteAll).toHaveBeenCalledTimes(1);
  });

  it("列表项标题优先级：note > startedAt > 会话 id", () => {
    renderReview({
      sessions: [
        makeSummary({ id: "a", note: "字节二面", startedAt: "2026-01-01 10:00:00" }),
        makeSummary({ id: "b", note: "", startedAt: "2026-01-02 09:00:00" }),
        makeSummary({ id: "c", note: "", startedAt: "" }),
      ],
    });
    expect(screen.getByText("字节二面")).toBeInTheDocument();
    expect(screen.getByText("2026-01-02 09:00:00")).toBeInTheDocument();
    expect(screen.getByText("会话 c")).toBeInTheDocument();
  });

  it("列表项 meta 显示模型与时长，模型缺失时回落「模型未知」", () => {
    renderReview({
      sessions: [
        makeSummary({ id: "a", model: "paraformer" }),
        makeSummary({
          id: "b",
          model: "",
          startedAt: "",
          endedAt: "",
        }),
      ],
    });
    expect(screen.getByText("paraformer · 2 分 30 秒")).toBeInTheDocument();
    // 模型回落成「模型未知」后 meta 永不为空，源码里的「时间未知」兜底走不到
    expect(screen.getByText("模型未知")).toBeInTheDocument();
    expect(screen.queryByText("时间未知")).not.toBeInTheDocument();
  });

  it("选中项带 active 类与 aria-current，点击另一项回调 onOpen", async () => {
    const user = userEvent.setup();
    const { onOpen } = renderReview({
      sessions: [makeSummary({ id: "a", note: "第一场" }), makeSummary({ id: "b", note: "第二场" })],
      selectedId: "a",
    });
    const active = screen.getByRole("button", { name: /第一场/ });
    expect(active).toHaveClass("session-item", "active");
    expect(active).toHaveAttribute("aria-current", "true");

    const other = screen.getByRole("button", { name: /第二场/ });
    expect(other).not.toHaveClass("active");
    expect(other).not.toHaveAttribute("aria-current");

    await user.click(other);
    expect(onOpen).toHaveBeenCalledWith("b");
  });
});

describe("SessionReview —— 右侧详情分支", () => {
  it("未选中会话时提示从左侧选择，导出/删除都禁用", () => {
    renderReview();
    expect(screen.getByText("从左侧选择一条会话查看详情。")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "导出 Markdown" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "删除" })).toBeDisabled();
  });

  it("选中后导出与删除可点并回调", async () => {
    const user = userEvent.setup();
    const { onExport, onDelete } = renderReview({
      selectedId: "s1",
      detail: makeDetail(),
    });
    await user.click(screen.getByRole("button", { name: "导出 Markdown" }));
    await user.click(screen.getByRole("button", { name: "删除" }));
    expect(onExport).toHaveBeenCalledTimes(1);
    expect(onDelete).toHaveBeenCalledTimes(1);
  });

  it("详情加载中两个按钮都禁用，只显示骨架", () => {
    const { container } = renderReview({ selectedId: "s1", detailLoading: true });
    expect(screen.getByRole("button", { name: "导出 Markdown" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "删除" })).toBeDisabled();
    expect(screen.getByText("正在加载会话…")).toBeInTheDocument();
    expect(container.querySelectorAll(".skeleton-text").length).toBe(3);
  });

  it("notice 按语义色挂类名，detailError 用 alert 展示", () => {
    const { container } = renderReview({
      selectedId: "s1",
      notice: "导出失败：磁盘只读",
      detailError: "加载会话失败：文件不存在",
    });
    const notice = screen.getByRole("status");
    expect(notice).toHaveClass("error-text");
    expect(notice).toHaveTextContent("导出失败：磁盘只读");
    expect(screen.getByRole("alert")).toHaveTextContent("加载会话失败：文件不存在");
    // 有错误且没有详情时不再叠「没有可显示的内容」
    expect(screen.queryByText("没有可显示的内容。")).not.toBeInTheDocument();
    expect(container.querySelector(".review-body")).toBeNull();
  });

  it("成功提示用绿色 status 类", () => {
    renderReview({ selectedId: "s1", notice: "已导出 Markdown 文件", detail: makeDetail() });
    expect(screen.getByRole("status")).toHaveClass("status");
  });

  it("详情为空且无错误时提示「没有可显示的内容」", () => {
    renderReview({ selectedId: "s1", detail: null });
    expect(screen.getByText("没有可显示的内容。")).toBeInTheDocument();
  });
});

describe("SessionReview —— 详情正文", () => {
  it("头部显示起止时间、模型与时长", () => {
    const { container } = renderReview({
      selectedId: "s1",
      detail: makeDetail({
        summary: makeSummary({
          startedAt: "2026-01-01 10:00:00",
          endedAt: "2026-01-01 11:05:30",
          model: "paraformer",
        }),
      }),
    });
    const head = container.querySelector(".review-body .muted");
    expect(head?.textContent).toBe(
      "2026-01-01 10:00:00 → 2026-01-01 11:05:30 · paraformer · 时长 1 小时 5 分",
    );
  });

  it("缺少时间与模型时回落文案，且没有结束时间就不显示箭头", () => {
    const { container } = renderReview({
      selectedId: "s1",
      detail: makeDetail({
        summary: makeSummary({ startedAt: "", endedAt: "", model: "" }),
      }),
    });
    const head = container.querySelector(".review-body .muted");
    expect(head?.textContent).toBe("开始时间未知");
    expect(head?.textContent).not.toContain("→");
  });

  it("转录为空与回答为空分别提示", () => {
    renderReview({ selectedId: "s1", detail: makeDetail() });
    expect(screen.getByText("该会话没有转录记录。")).toBeInTheDocument();
    expect(screen.getByText("该会话没有生成过答案。")).toBeInTheDocument();
    // 两者都空时额外给一条排查提示
    expect(screen.getByText(/该会话内容为空/)).toBeInTheDocument();
    // 没有转录就不显示颜色图例
    expect(screen.queryByLabelText("转录颜色图例")).not.toBeInTheDocument();
  });

  it("有转录时显示图例与双通道行，回声行带 echo 类与标签", () => {
    const { container } = renderReview({
      selectedId: "s1",
      detail: makeDetail({
        segments: [
          { id: "seg1", channel: "interviewer", text: "自我介绍", isEcho: false, startMs: 0 },
          { id: "seg2", channel: "me", text: "我来自……", isEcho: false, startMs: 1000 },
          { id: "seg3", channel: "me", text: "（重复）", isEcho: true, startMs: 2000 },
        ],
      }),
    });
    expect(screen.getByLabelText("转录颜色图例")).toBeInTheDocument();
    const rows = Array.from(container.querySelectorAll(".review-transcript .line"));
    expect(rows).toHaveLength(3);
    expect(rows[0]).toHaveClass("interviewer");
    expect(within(rows[0] as HTMLElement).getByText("面试官")).toBeInTheDocument();
    expect(rows[1]).toHaveClass("me");
    expect(within(rows[1] as HTMLElement).getByText("我")).toBeInTheDocument();
    expect(rows[2]).toHaveClass("echo");
    expect(within(rows[2] as HTMLElement).getByText("回声")).toBeInTheDocument();
    // 有转录时不再提示空转录
    expect(screen.queryByText("该会话没有转录记录。")).not.toBeInTheDocument();
  });

  it("回答区渲染正文、模型与首字延迟，空答案有占位", () => {
    const { container } = renderReview({
      selectedId: "s1",
      detail: makeDetail({
        answers: [
          { id: "a1", text: "第一段回答", model: "paraformer", ttftMs: 1200 },
          { id: "a2", text: "", model: "", ttftMs: null },
        ],
      }),
    });
    const boxes = Array.from(container.querySelectorAll(".answer-box")).map(
      (node) => node.textContent,
    );
    expect(boxes).toEqual(["第一段回答", "（空答案）"]);
    expect(screen.getByText("paraformer · 首字 1.2s")).toBeInTheDocument();
    expect(screen.getByText("模型未知")).toBeInTheDocument();
  });

  it("只有回答没有转录时：不显示图例，也不显示「内容为空」排查句", () => {
    renderReview({
      selectedId: "s1",
      detail: makeDetail({
        answers: [{ id: "a1", text: "有答案", model: "m", ttftMs: null }],
      }),
    });
    expect(screen.queryByLabelText("转录颜色图例")).not.toBeInTheDocument();
    expect(screen.queryByText(/该会话内容为空/)).not.toBeInTheDocument();
  });
});