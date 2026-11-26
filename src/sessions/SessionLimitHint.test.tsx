import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import SessionReview from "./SessionReview";
import type { SessionSummary } from "./types";

function makeSummary(id: string): SessionSummary {
  return {
    id,
    startedAt: "2026-11-26 15:37:30",
    endedAt: "2026-11-26 15:38:00",
    model: "deepseek-v4.1-flash",
    note: "",
  };
}

function renderList(props: { limited: boolean; totalCount: number; count: number }) {
  const sessions = Array.from({ length: props.count }, (_, i) => makeSummary(String(i + 1)));
  render(
    <SessionReview
      sessions={sessions}
      totalCount={props.totalCount}
      query=""
      onQueryChange={vi.fn()}
      searching={false}
      isSearching={false}
      limited={props.limited}
      loading={false}
      error=""
      selectedId=""
      detail={null}
      detailLoading={false}
      detailError=""
      notice=""
      onRefresh={vi.fn()}
      onOpen={vi.fn()}
      onExport={vi.fn()}
      onDelete={vi.fn()}
      onDeleteAll={vi.fn()}
      onRename={vi.fn(async () => {})}
      onExportAll={vi.fn()}
    />,
  );
}

describe("SessionReview —— 条数上限提示", () => {
  it("被截断时提示「只显示最近 N 条」和真实总数", () => {
    // 列表接口上限 200：用户有 350 条时只能看到 200 条，界面必须说明。
    renderList({ limited: true, totalCount: 350, count: 200 });
    const hint = screen.getByText(/只显示最近/);
    expect(hint).toHaveTextContent("200");
    expect(hint).toHaveTextContent("350");
    // 还要告诉用户记录没丢
    expect(hint).toHaveTextContent(/仍在本机/);
  });

  it("没被截断时不显示提示", () => {
    renderList({ limited: false, totalCount: 5, count: 5 });
    expect(screen.queryByText(/只显示最近/)).toBeNull();
  });

  it("提示文案不说「丢了多少」——记录还在，只是没列出来", () => {
    // 说「丢了 150 条」会让人以为数据损坏；这里强调「仍在本机 + 可以导出」。
    renderList({ limited: true, totalCount: 350, count: 200 });
    const hint = screen.getByText(/只显示最近/);
    expect(hint.textContent).not.toMatch(/丢失|损坏|失败/);
  });
});