import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const backend = vi.hoisted(() => {
  const calls: Array<{ command: string; args: unknown }> = [];
  const state: {
    importText: (path: string) => unknown;
  } = { importText: () => "" };
  return { calls, state };
});

const dialog = vi.hoisted(() => {
  const state: { nextSelection: unknown } = { nextSelection: null };
  return { state };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((command: string, args: unknown) => {
    backend.calls.push({ command, args });
    if (command === "import_document_text") {
      return Promise.resolve().then(() =>
        backend.state.importText((args as { path: string }).path),
      );
    }
    return Promise.resolve(null);
  }),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(() => Promise.resolve(dialog.state.nextSelection)),
}));

import ProfileCard from "./ProfileCard";
import { EMPTY_SETTINGS, type Settings } from "./types";

const LIMIT = 6000;

function makeForm(overrides: Partial<Settings> = {}): Settings {
  return { ...EMPTY_SETTINGS, resume: "", projects: "", ...overrides };
}

function renderCard(form: Settings = makeForm()) {
  const patch = vi.fn();
  const utils = render(<ProfileCard form={form} patch={patch} />);
  return {
    ...utils,
    patch,
    patchLast: () => patch.mock.calls[patch.mock.calls.length - 1]?.[0],
  };
}

function importCalls() {
  return backend.calls.filter((call) => call.command === "import_document_text");
}

beforeEach(() => {
  backend.calls.length = 0;
  backend.state.importText = () => "";
  dialog.state.nextSelection = null;
});

describe("ProfileCard —— 两个 textarea 双向绑定", () => {
  it("简历与项目资料分别显示表单值，改动各自 patch 对应字段", () => {
    const { patch } = renderCard(
      makeForm({ resume: "五年后端经验", projects: "订单系统重构" }),
    );

    const resume = screen.getByLabelText("简历") as HTMLTextAreaElement;
    const projects = screen.getByLabelText("项目资料") as HTMLTextAreaElement;
    expect(resume.value).toBe("五年后端经验");
    expect(projects.value).toBe("订单系统重构");

    fireEvent.change(resume, { target: { value: "六年后端经验" } });
    expect(patch).toHaveBeenLastCalledWith({ resume: "六年后端经验" });

    fireEvent.change(projects, { target: { value: "支付网关迁移" } });
    expect(patch).toHaveBeenLastCalledWith({ projects: "支付网关迁移" });
  });

  it("字数提示按当前值与 6000 上限展示", () => {
    renderCard(makeForm({ resume: "abcdef", projects: "12" }));
    expect(screen.getByText("6 / 6000")).toBeInTheDocument();
    expect(screen.getByText("2 / 6000")).toBeInTheDocument();
  });
});

describe("ProfileCard —— 字数超限提示", () => {
  it("未超限时字数串不带 is-over", () => {
    const { container } = renderCard(makeForm({ resume: "a".repeat(LIMIT) }));
    expect(screen.getByText(`${LIMIT} / ${LIMIT}`)).not.toHaveClass("is-over");
    expect(container.querySelectorAll(".char-count.is-over")).toHaveLength(0);
  });

  it("超过 6000 字时两个计数字段各自变红（is-over）", () => {
    const { container } = renderCard(
      makeForm({ resume: "a".repeat(LIMIT + 1), projects: "b".repeat(LIMIT + 5) }),
    );
    expect(screen.getByText(`${LIMIT + 1} / ${LIMIT}`)).toHaveClass("is-over");
    expect(screen.getByText(`${LIMIT + 5} / ${LIMIT}`)).toHaveClass("is-over");
    expect(container.querySelectorAll(".char-count.is-over")).toHaveLength(2);
  });

  it("只有一侧超限时另一侧不受影响", () => {
    renderCard(makeForm({ resume: "a".repeat(LIMIT + 1), projects: "短" }));
    expect(screen.getByText(`${LIMIT + 1} / ${LIMIT}`)).toHaveClass("is-over");
    expect(screen.getByText(`1 / ${LIMIT}`)).not.toHaveClass("is-over");
  });
});

describe("ProfileCard —— 从文件导入", () => {
  it("选文件后调 import_document_text 并把文本 patch 进简历", async () => {
    const user = userEvent.setup();
    dialog.state.nextSelection = "D:\\docs\\我的简历.docx";
    backend.state.importText = () => "  导入的简历正文  ";
    const { patchLast } = renderCard();

    await user.click(screen.getAllByRole("button", { name: "从文件导入" })[0]);

    await waitFor(() => {
      expect(importCalls()).toHaveLength(1);
    });
    expect(importCalls()[0].args).toEqual({ path: "D:\\docs\\我的简历.docx" });
    await waitFor(() => {
      expect(patchLast()).toEqual({ resume: "导入的简历正文" });
    });
    expect(
      screen.getByText("已导入 我的简历.docx（7 字）"),
    ).toBeInTheDocument();
  });

  it("第二个导入按钮写入 projects 字段", async () => {
    const user = userEvent.setup();
    dialog.state.nextSelection = "D:\\docs\\项目.md";
    backend.state.importText = () => "项目正文";
    const { patchLast } = renderCard();

    await user.click(screen.getAllByRole("button", { name: "从文件导入" })[1]);

    await waitFor(() => {
      expect(patchLast()).toEqual({ projects: "项目正文" });
    });
    expect(screen.getByText("已导入 项目.md（4 字）")).toBeInTheDocument();
  });

  it("用户取消选择时不发后��命令、不改内容", async () => {
    const user = userEvent.setup();
    dialog.state.nextSelection = null;
    const { patch } = renderCard();

    await user.click(screen.getAllByRole("button", { name: "从文件导入" })[0]);

    await waitFor(() => {
      expect(screen.getAllByRole("button", { name: "从文件导入" })).toHaveLength(2);
    });
    expect(importCalls()).toHaveLength(0);
    expect(patch).not.toHaveBeenCalled();
  });

  it("导入超过 6000 字时截断并附提示，状态里写明原文字数", async () => {
    const user = userEvent.setup();
    dialog.state.nextSelection = "D:\\docs\\长简历.txt";
    backend.state.importText = () => "字".repeat(LIMIT + 500);
    const { patchLast } = renderCard();

    await user.click(screen.getAllByRole("button", { name: "从文件导入" })[0]);

    await waitFor(() => {
      expect(patchLast()).toBeDefined();
    });
    const patched = patchLast() as { resume: string };
    expect(patched.resume.startsWith("字".repeat(LIMIT))).toBe(true);
    expect(patched.resume).toContain(`（导入内容超过 ${LIMIT} 字，已截断）`);
    expect(patched.resume.length).toBe(LIMIT + `\n（导入内容超过 ${LIMIT} 字，已截断）`.length);
    expect(
      screen.getByText(`已导入 长简历.txt：原文 ${LIMIT + 500} 字，已截取前 ${LIMIT} 字`),
    ).toBeInTheDocument();
  });

  it("恰好 6000 字不触发截断", async () => {
    const user = userEvent.setup();
    dialog.state.nextSelection = "C:\\a.txt";
    backend.state.importText = () => "字".repeat(LIMIT);
    const { patchLast } = renderCard();

    await user.click(screen.getAllByRole("button", { name: "从文件导入" })[0]);

    await waitFor(() => {
      expect(patchLast()).toEqual({ resume: "字".repeat(LIMIT) });
    });
    expect(screen.getByText(`已导入 a.txt（${LIMIT} 字）`)).toBeInTheDocument();
  });

  it("后端解析失败：状态带「导入失败：」前缀，不改内容", async () => {
    const user = userEvent.setup();
    dialog.state.nextSelection = "D:\\docs\\坏文件.pdf";
    backend.state.importText = () => Promise.reject("PDF 解析失败");
    const { patch } = renderCard();

    await user.click(screen.getAllByRole("button", { name: "从文件导入" })[0]);

    await waitFor(() => {
      expect(screen.getByText("导入失败：PDF 解析失败")).toBeInTheDocument();
    });
    expect(patch).not.toHaveBeenCalled();
  });

  it("对话框本身抛错时也落到同一状态行", async () => {
    const user = userEvent.setup();
    const { open } = await import("@tauri-apps/plugin-dialog");
    const mocked = vi.mocked(open);
    mocked.mockImplementationOnce(() => Promise.reject(new Error("没有权限打开对话框")));
    const { patch } = renderCard();

    await user.click(screen.getAllByRole("button", { name: "从文件导入" })[0]);

    await waitFor(() => {
      expect(screen.getByText("导入失败：没有权限打开对话框")).toBeInTheDocument();
    });
    expect(patch).not.toHaveBeenCalled();
  });

  it("非 Tauri 环境：点导入直接提示无法打开对话框，不发命令", async () => {
    const user = userEvent.setup();
    const descriptor = Object.getOwnPropertyDescriptor(window, "__TAURI_INTERNALS__");
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    try {
      renderCard();
      await user.click(screen.getAllByRole("button", { name: "从文件导入" })[0]);
      expect(
        screen.getByText("纯浏览器预览无法打开文件对话框；安装版可直接导入。"),
      ).toBeInTheDocument();
      expect(importCalls()).toHaveLength(0);
    } finally {
      if (descriptor) Object.defineProperty(window, "__TAURI_INTERNALS__", descriptor);
    }
  });

  it("导入期间按钮禁用并显示「导入中…」", async () => {
    const user = userEvent.setup();
    let release!: (value: unknown) => void;
    dialog.state.nextSelection = "D:\\docs\\a.txt";
    backend.state.importText = () =>
      new Promise((resolve) => {
        release = resolve;
      });
    renderCard();

    await user.click(screen.getAllByRole("button", { name: "从文件导入" })[0]);
    // 正在导入的那一侧显示「导入中…」，另一侧保持原文案但同样禁用
    const busyButton = screen.getByRole("button", { name: "导入中…" });
    expect(busyButton).toBeDisabled();
    const idleButton = screen.getByRole("button", { name: "从文件导入" });
    expect(idleButton).toBeDisabled();

    release("文本");
    await waitFor(() => {
      expect(screen.getAllByRole("button", { name: "从文件导入" })).toHaveLength(2);
    });
  });
});

describe("ProfileCard —— 档案事实的编辑与删除", () => {
  const profile: Settings["profile"] = {
    organizedAt: 1,
    sources: ["简历.md"],
    facts: [
      { kind: "basics", text: "张三 5年测试开发", tags: ["张三"] },
      { kind: "project", text: "[P1] 自动化平台 回归3天到4小时", tags: ["自动化"] },
      { kind: "qa", text: "离职原因 想做工程化", tags: ["离职"] },
    ],
  };

  it("逐条列出全部事实（不再只显示前 8 条）", () => {
    const { container } = renderCard(makeForm({ profile }));
    const items = container.querySelectorAll(".profile-fact-list li");
    expect(items).toHaveLength(3);
    expect(screen.getByText(/张三 5年测试开发/)).toBeInTheDocument();
  });

  it("点 × 删掉一条，profile 里少一条", async () => {
    const { patchLast } = renderCard(makeForm({ profile }));
    const removes = screen.getAllByLabelText(/删除第 \d+ 条事实/);
    await userEvent.click(removes[1]);

    const next = patchLast() as { profile: Settings["profile"] };
    expect(next.profile?.facts).toHaveLength(2);
    expect(next.profile?.facts.map((f) => f.kind)).toEqual(["basics", "qa"]);
  });

  it("删光后档案置空，退回整段发送", async () => {
    const single = { ...profile, facts: [profile.facts[0]] };
    const { patchLast } = renderCard(makeForm({ profile: single }));
    await userEvent.click(screen.getByLabelText("删除第 1 条事实"));

    const next = patchLast() as { profile: Settings["profile"] };
    expect(next.profile).toBeNull();
  });

  it("点正文进入编辑，保存后 text 更新且 tags 保留", async () => {
    const { patchLast } = renderCard(makeForm({ profile }));
    await userEvent.click(screen.getByText(/回归3天到4小时/));

    const box = screen.getByLabelText("编辑第 2 条事实");
    fireEvent.change(box, { target: { value: "[P1] 自动化平台 回归3天到2小时" } });
    await userEvent.click(screen.getByText("保存"));

    const next = patchLast() as { profile: Settings["profile"] };
    expect(next.profile?.facts[1].text).toBe("[P1] 自动化平台 回归3天到2小时");
    // 只改正文，标签不该被顺手清掉。
    expect(next.profile?.facts[1].tags).toEqual(["自动化"]);
  });

  it("取消编辑不改动任何数据", async () => {
    const { patch } = renderCard(makeForm({ profile }));
    await userEvent.click(screen.getByText(/回归3天到4小时/));
    fireEvent.change(screen.getByLabelText("编辑第 2 条事实"), {
      target: { value: "改了但不保存" },
    });
    await userEvent.click(screen.getByText("取消"));

    expect(patch).not.toHaveBeenCalled();
  });

  it("正文清空时保存按钮不可用（空事实没有意义）", async () => {
    renderCard(makeForm({ profile }));
    await userEvent.click(screen.getByText(/回归3天到4小时/));
    fireEvent.change(screen.getByLabelText("编辑第 2 条事实"), {
      target: { value: "   " },
    });
    expect(screen.getByText("保存")).toBeDisabled();
  });
});

describe("ProfileCard —— 检索标签编辑", () => {
  const profile: Settings["profile"] = {
    organizedAt: 1,
    sources: ["简历.md"],
    facts: [
      { kind: "basics", text: "李四 3年测试", tags: [] },
      { kind: "project", text: "[X1] 内部系统 负责中间件选型", tags: ["kafka"] },
    ],
  };

  it("非编辑态显示标签（可核对自己加了什么）", () => {
    renderCard(makeForm({ profile }));
    expect(screen.getByText(/kafka/)).toBeInTheDocument();
  });

  it("进入编辑时标签输入框带原值", async () => {
    renderCard(makeForm({ profile }));
    await userEvent.click(screen.getByText(/负责中间件选型/));
    const input = screen.getByLabelText("编辑第 2 条事实的标签") as HTMLInputElement;
    expect(input.value).toBe("kafka");
  });

  it("改标签后保存，tags 更新", async () => {
    const { patchLast } = renderCard(makeForm({ profile }));
    await userEvent.click(screen.getByText(/负责中间件选型/));
    const input = screen.getByLabelText("编辑第 2 条事实的标签");
    fireEvent.change(input, { target: { value: "kafka, 消息队列" } });
    await userEvent.click(screen.getByText("保存"));

    const next = patchLast() as { profile: Settings["profile"] };
    expect(next.profile?.facts[1].tags).toEqual(["kafka", "消息队列"]);
  });

  it("中文逗号/顿号/空格都当分隔符", async () => {
    const { patchLast } = renderCard(makeForm({ profile }));
    await userEvent.click(screen.getByText(/负责中间件选型/));
    fireEvent.change(screen.getByLabelText("编辑第 2 条事实的标签"), {
      target: { value: "中间件、选型，压测 kafka" },
    });
    await userEvent.click(screen.getByText("保存"));

    const next = patchLast() as { profile: Settings["profile"] };
    expect(next.profile?.facts[1].tags).toEqual(["中间件", "选型", "压测", "kafka"]);
  });

  it("清空标签得到空数组（不是保留旧值）", async () => {
    const { patchLast } = renderCard(makeForm({ profile }));
    await userEvent.click(screen.getByText(/负责中间件选型/));
    fireEvent.change(screen.getByLabelText("编辑第 2 条事实的标签"), {
      target: { value: "   " },
    });
    await userEvent.click(screen.getByText("保存"));

    const next = patchLast() as { profile: Settings["profile"] };
    expect(next.profile?.facts[1].tags).toEqual([]);
  });

  it("只改标签不动正文", async () => {
    const { patchLast } = renderCard(makeForm({ profile }));
    await userEvent.click(screen.getByText(/负责中间件选型/));
    fireEvent.change(screen.getByLabelText("编辑第 2 条事实的标签"), {
      target: { value: "kafka, mq" },
    });
    await userEvent.click(screen.getByText("保存"));

    const next = patchLast() as { profile: Settings["profile"] };
    expect(next.profile?.facts[1].text).toBe("[X1] 内部系统 负责中间件选型");
    expect(next.profile?.facts[1].tags).toEqual(["kafka", "mq"]);
  });
});
