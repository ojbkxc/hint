import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import SessionReview from "./SessionReview";
import type { SessionDetail, SessionSummary } from "./types";

function makeSummary(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    id: "1",
    startedAt: "2026-11-26 15:37:30",
    endedAt: "2026-11-26 15:38:00",
    model: "deepseek-v4.1-flash",
    note: "",
    ...overrides,
  };
}

function makeDetail(note = ""): SessionDetail {
  return {
    id: "1",
    summary: makeSummary({ note }),
    segments: [],
    answers: [],
  };
}

function renderWithNote(note: string, onRename = vi.fn(async () => {})) {
  render(
    <SessionReview
      sessions={[makeSummary({ note })]}
      totalCount={1}
      query=""
      onQueryChange={vi.fn()}
      searching={false}
      isSearching={false}
      loading={false}
      error=""
      selectedId="1"
      detail={makeDetail(note)}
      detailLoading={false}
      detailError=""
      notice=""
      onRefresh={vi.fn()}
      onOpen={vi.fn()}
      onExport={vi.fn()}
      onDelete={vi.fn()}
      onDeleteAll={vi.fn()}
      onRename={onRename}
    />,
  );
  return { onRename };
}

describe("SessionReview —— 会话备注", () => {
  it("输入框显示已有备注", () => {
    renderWithNote("字节二面");
    expect(screen.getByLabelText("会话备注")).toHaveValue("字节二面");
  });

  it("没有备注时显示引导占位文案", () => {
    renderWithNote("");
    expect(screen.getByLabelText("会话备注")).toHaveAttribute(
      "placeholder",
      expect.stringContaining("字节二面"),
    );
  });

  it("失去焦点时保存备注", async () => {
    const user = userEvent.setup();
    const { onRename } = renderWithNote("");
    const input = screen.getByLabelText("会话备注");
    await user.type(input, "腾讯一面");
    await user.tab();
    await waitFor(() => expect(onRename).toHaveBeenCalledWith("1", "腾讯一面"));
  });

  it("回车也会保存（不必先点别处）", async () => {
    const user = userEvent.setup();
    const { onRename } = renderWithNote("");
    const input = screen.getByLabelText("会话备注");
    await user.type(input, "阿里三面{Enter}");
    await waitFor(() => expect(onRename).toHaveBeenCalledWith("1", "阿里三面"));
  });

  it("内容没变就不调用后端（避免无意义的写盘）", async () => {
    const user = userEvent.setup();
    const { onRename } = renderWithNote("字节二面");
    const input = screen.getByLabelText("会话备注");
    await user.click(input);
    await user.tab();
    expect(onRename).not.toHaveBeenCalled();
  });

  it("保存失败时显示原因，不静默回滚", async () => {
    const user = userEvent.setup();
    const onRename = vi.fn(async () => {
      throw new Error("该会话已不存在（可能刚被删除）");
    });
    renderWithNote("", onRename);
    const input = screen.getByLabelText("会话备注");
    await user.type(input, "字节二面");
    await user.tab();
    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("该会话已不存在"),
    );
  });

  it("Esc 放弃编辑，不保存", async () => {
    const user = userEvent.setup();
    const { onRename } = renderWithNote("字节二面");
    const input = screen.getByLabelText("会话备注");
    await user.clear(input);
    await user.type(input, "改了一半{Escape}");
    expect(onRename).not.toHaveBeenCalled();
  });
});