import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import ModelDownloadCard from "./ModelDownloadCard";
import { INITIAL_MODEL_VIEW, type ModelView } from "./modelTypes";

function view(overrides: Partial<ModelView> = {}): ModelView {
  return { ...INITIAL_MODEL_VIEW, ...overrides };
}

type CardProps = {
  view?: ModelView;
  busy?: boolean;
  notice?: string;
  onDownload?: () => void;
  onCancel?: () => void;
  onRemove?: () => void;
};

function renderCard(props: CardProps = {}) {
  const onDownload = props.onDownload ?? vi.fn();
  const onCancel = props.onCancel ?? vi.fn();
  const onRemove = props.onRemove ?? vi.fn();
  const utils = render(
    <ModelDownloadCard
      view={props.view ?? view()}
      busy={props.busy ?? false}
      notice={props.notice ?? ""}
      onDownload={onDownload}
      onCancel={onCancel}
      onRemove={onRemove}
    />,
  );
  return { ...utils, onDownload, onCancel, onRemove };
}

/** 进度条宽度写在 style 上，只能从 DOM 读。 */
function fillWidth(container: HTMLElement): string {
  const fill = container.querySelector(".progress-fill") as HTMLElement;
  return fill.style.width;
}

describe("ModelDownloadCard —— loading 态", () => {
  it("显示读取中与占位目录，下载按钮禁用", () => {
    renderCard({ view: view({ phase: "loading" }) });
    expect(screen.getByText("读取中…")).toBeInTheDocument();
    expect(screen.getByText("正在读取模型状态…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "下载模型" })).toBeDisabled();
    expect(screen.queryByText("已就绪")).not.toBeInTheDocument();
  });
});

describe("ModelDownloadCard —— unprepared 态", () => {
  it("未下载：按钮为「下载模型」，进度显示 0 与 0.0%", async () => {
    const user = userEvent.setup();
    const { container, onDownload } = renderCard({
      view: view({ phase: "unprepared", downloadedBytes: 0, totalBytes: 241357257 }),
    });

    expect(screen.getByText("未下载")).toBeInTheDocument();
    expect(screen.getByText(/当前未下载或文件不完整/)).toBeInTheDocument();
    expect(screen.queryByText(/支持断点续传/)).not.toBeInTheDocument();
    expect(screen.getByText("0.0%")).toBeInTheDocument();
    expect(screen.getByText("0 MB / 230.2 MB")).toBeInTheDocument();
    expect(fillWidth(container)).toBe("0%");

    await user.click(screen.getByRole("button", { name: "下载模型" }));
    expect(onDownload).toHaveBeenCalledTimes(1);
  });

  it("已有部分文件：提示断点续传并按比例算进度与宽度", () => {
    const { container } = renderCard({
      view: view({
        phase: "unprepared",
        downloadedBytes: 24135725,
        totalBytes: 241357257,
      }),
    });
    expect(screen.getByText(/已有 23.0 MB，支持断点续传/)).toBeInTheDocument();
    expect(screen.getByText("10.0%")).toBeInTheDocument();
    expect(fillWidth(container)).toBe("10%");
  });

  it("error 态复用未下载面板，并额外显示错误文案", () => {
    renderCard({
      view: view({ phase: "error", error: "模型目录不可写" }),
    });
    expect(screen.getByText("读取失败")).toBeInTheDocument();
    expect(screen.getByText("模型目录不可写")).toHaveClass("error-text");
    expect(screen.getByRole("button", { name: "下载模型" })).toBeEnabled();
  });

  it("busy 时下载按钮禁用", () => {
    renderCard({ busy: true, view: view({ phase: "unprepared" }) });
    expect(screen.getByRole("button", { name: "下载模型" })).toBeDisabled();
  });
});

describe("ModelDownloadCard —— downloading 态", () => {
  it("显示阶段文案、来源中文名、百分比与进度宽度（最小 1%）", async () => {
    const user = userEvent.setup();
    const { container, onCancel } = renderCard({
      view: view({
        phase: "downloading",
        step: "verifying",
        resource: "model.int8.onnx",
        completedBytes: 120678628,
        totalBytes: 241357257,
        source: "https://hf-mirror.com",
      }),
    });

    expect(screen.getByText("下载中")).toBeInTheDocument();
    expect(screen.getByText("校验文件 · model.int8.onnx")).toBeInTheDocument();
    expect(screen.getByText("50.0%")).toBeInTheDocument();
    expect(screen.getByText("115.1 MB / 230.2 MB")).toBeInTheDocument();
    expect(screen.getByText("来源：镜像源")).toBeInTheDocument();
    expect(fillWidth(container)).toBe("50%");

    await user.click(screen.getByRole("button", { name: "取消下载" }));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("刚开始下载（0%）时填充宽度仍有 1%，且不显示删除按钮", () => {
    const { container } = renderCard({
      view: view({
        phase: "downloading",
        step: "model",
        completedBytes: 0,
        totalBytes: 241357257,
      }),
    });
    expect(fillWidth(container)).toBe("1%");
    expect(screen.getByText("0.0%")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "删除模型" })).not.toBeInTheDocument();
    expect(screen.queryByText(/来源：/)).not.toBeInTheDocument();
  });

  it("未知 step 原样显示，未知来源原样展示（不映射成官方/镜像）", () => {
    renderCard({
      view: view({
        phase: "downloading",
        step: "warmup" as ModelView["step"],
        completedBytes: 1,
        totalBytes: 100,
        source: "https://example.com/dl",
      }),
    });
    expect(screen.getByText("warmup")).toBeInTheDocument();
    expect(screen.getByText("来源：https://example.com/dl")).toBeInTheDocument();
  });

  it("官方源写法映射成「官方源」", () => {
    renderCard({
      view: view({
        phase: "downloading",
        completedBytes: 1,
        totalBytes: 100,
        source: "official",
      }),
    });
    expect(screen.getByText("来源：官方源")).toBeInTheDocument();
  });
});

describe("ModelDownloadCard —— ready 态", () => {
  it("按钮变「检查模型文件」并出现删除按钮，点击分别回调", async () => {
    const user = userEvent.setup();
    const { onDownload, onRemove } = renderCard({
      view: view({ phase: "ready", modelDir: "C:\\models\\sensevoice", totalBytes: 241357257 }),
    });

    expect(screen.getByText("已就绪")).toHaveClass("tag-ok");
    expect(screen.getByText("C:\\models\\sensevoice")).toBeInTheDocument();
    expect(screen.getByText(/删除前请先结束正在进行的面试/)).toBeInTheDocument();

    const check = screen.getByRole("button", { name: "检查模型文件" });
    expect(check).toHaveAttribute("title", "只核对文件是否存在且大小一致，不会重新下载或覆盖");
    await user.click(check);
    expect(onDownload).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "删除模型" }));
    expect(onRemove).toHaveBeenCalledTimes(1);
  });

  it("就绪但没拿到目录时显示「默认目录」", () => {
    renderCard({ view: view({ phase: "ready", modelDir: "" }) });
    expect(screen.getByText("默认目录")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "取消下载" })).not.toBeInTheDocument();
  });

  it("busy 时删除按钮禁用", () => {
    renderCard({ busy: true, view: view({ phase: "ready", modelDir: "D:\\m" }) });
    expect(screen.getByRole("button", { name: "删除模型" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "检查模型文件" })).toBeDisabled();
  });

  it("未就绪且无目录时目录列显示破折号", () => {
    const { container } = renderCard({ view: view({ phase: "unprepared" }) });
    const kv = container.querySelector(".settings-kv") as HTMLElement;
    expect(kv.textContent).toContain("—");
  });
});

describe("ModelDownloadCard —— notice", () => {
  it("成功类 notice 走 status 类，失败类走 error-text", () => {
    const ok = renderCard({ notice: "模型已删除" });
    expect(ok.getByText("模型已删除")).toHaveClass("status");
    ok.unmount();

    const bad = renderCard({ notice: "下载失败：网络中断" });
    expect(bad.getByText("下载失败：网络中断")).toHaveClass("error-text");
  });

  it("没有 notice 时不渲染提示段", () => {
    const { container } = renderCard();
    expect(container.querySelector("p.status")).toBeNull();
  });
});