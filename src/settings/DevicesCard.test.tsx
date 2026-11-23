import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import DevicesCard from "./DevicesCard";
import { EMPTY_SETTINGS, type AudioDevice, type Settings } from "./types";

type Devices = { inputs: AudioDevice[]; outputs: AudioDevice[]; error: string };

function devices(overrides: Partial<Devices> = {}): Devices {
  return {
    inputs: [
      { id: "mic-1", name: "内置麦克风" },
      { id: "mic-2", name: "USB 麦克风" },
    ],
    outputs: [{ id: "spk-1", name: "扬声器" }],
    error: "",
    ...overrides,
  };
}

function makeForm(overrides: Partial<Settings> = {}): Settings {
  return { ...EMPTY_SETTINGS, ...overrides };
}

type CardProps = {
  form?: Settings;
  loaded?: boolean;
  loadDevices?: () => Promise<Devices>;
};

function renderCard(props: CardProps = {}) {
  const patch = vi.fn();
  const loadDevices = props.loadDevices ?? vi.fn(() => Promise.resolve(devices()));
  const utils = render(
    <DevicesCard
      form={props.form ?? makeForm()}
      loaded={props.loaded ?? true}
      patch={patch}
      loadDevices={loadDevices}
    />,
  );
  return { ...utils, patch, loadDevices };
}

function micSelect(): HTMLSelectElement {
  return screen.getByLabelText("麦克风（我的声音）") as HTMLSelectElement;
}

function outputSelect(): HTMLSelectElement {
  return screen.getByLabelText("系统输出（面试官声音）") as HTMLSelectElement;
}

describe("DevicesCard —— 首次自动加载", () => {
  it("loaded 为 false 时不拉设备", () => {
    const { loadDevices } = renderCard({ loaded: false });
    expect(loadDevices).not.toHaveBeenCalled();
  });

  it("loaded 变 true 后只自动拉一次（attempted ref 守卫）", async () => {
    const loadDevices = vi.fn(() => Promise.resolve(devices()));
    const { rerender } = renderCard({ loaded: false, loadDevices });
    expect(loadDevices).not.toHaveBeenCalled();

    const form = makeForm();
    rerender(<DevicesCard form={form} loaded={true} patch={vi.fn()} loadDevices={loadDevices} />);
    await waitFor(() => {
      expect(loadDevices).toHaveBeenCalledTimes(1);
    });

    // 重新渲染、甚至 loaded 再翻转一次，也不会重复自动拉取
    rerender(<DevicesCard form={form} loaded={false} patch={vi.fn()} loadDevices={loadDevices} />);
    rerender(<DevicesCard form={form} loaded={true} patch={vi.fn()} loadDevices={loadDevices} />);
    await waitFor(() => {
      expect(loadDevices).toHaveBeenCalledTimes(1);
    });
  });

  it("自动加载完成后填入设备下拉与通道计数", async () => {
    renderCard();
    await waitFor(() => {
      expect(within(micSelect()).getByText("USB 麦克风")).toBeInTheDocument();
    });
    expect(within(outputSelect()).getByText("扬声器")).toBeInTheDocument();

    const kv = screen.getByText("输入通道").parentElement as HTMLElement;
    expect(within(kv).getByText("2")).toBeInTheDocument();
    expect(within(kv).getByText("1")).toBeInTheDocument();
    expect(screen.getByText(/双通道电平条在浮层底部实时跳动才算通过/)).toBeInTheDocument();
  });
});

describe("DevicesCard —— 刷新按钮", () => {
  it("点刷新再调一次 loadDevices；检测中按钮文案变「检测中…」并禁用", async () => {
    const user = userEvent.setup();
    let release!: (value: Devices) => void;
    const loadDevices = vi
      .fn<() => Promise<Devices>>()
      .mockImplementationOnce(() => Promise.resolve(devices()))
      .mockImplementationOnce(
        () =>
          new Promise<Devices>((resolve) => {
            release = resolve;
          }),
      );
    renderCard({ loadDevices });

    await waitFor(() => {
      expect(loadDevices).toHaveBeenCalledTimes(1);
    });

    await user.click(screen.getByRole("button", { name: "刷新设备" }));
    const loadingButton = screen.getByRole("button", { name: "检测中…" });
    expect(loadingButton).toBeDisabled();
    expect(screen.getByText("正在检测音频设备…")).toBeInTheDocument();

    release(devices({ inputs: [{ id: "mic-9", name: "新插入麦克风" }], outputs: [] }));
    await waitFor(() => {
      expect(within(micSelect()).getByText("新插入麦克风")).toBeInTheDocument();
    });
    expect(screen.getByRole("button", { name: "刷新设备" })).toBeEnabled();
    expect(screen.queryByRole("option", { name: "USB 麦克风" })).not.toBeInTheDocument();
  });

  it("刷新失败：显示带前缀的错误与排查提示", async () => {
    const user = userEvent.setup();
    const loadDevices = vi
      .fn<() => Promise<Devices>>()
      .mockImplementationOnce(() => Promise.resolve(devices()))
      .mockImplementationOnce(() =>
        Promise.resolve({ inputs: [], outputs: [], error: "设备被独占" }),
      );
    renderCard({ loadDevices });

    await waitFor(() => {
      expect(loadDevices).toHaveBeenCalledTimes(1);
    });
    await user.click(screen.getByRole("button", { name: "刷新设备" }));

    await waitFor(() => {
      expect(screen.getByText("读取设备失败：设备被独占")).toBeInTheDocument();
    });
    expect(screen.getByText("读取设备失败：设备被独占")).toHaveClass("error-text");
    expect(screen.getByText(/请确认麦克风未被其它程序独占/)).toBeInTheDocument();
  });

  it("设备列表为空时给空状态，而不是错误", async () => {
    const loadDevices = vi.fn(() =>
      Promise.resolve({ inputs: [], outputs: [], error: "" }),
    );
    renderCard({ loadDevices });

    await waitFor(() => {
      expect(screen.getByText(/未检测到可用音频设备/)).toBeInTheDocument();
    });
    expect(screen.queryByText(/读取设备失败/)).not.toBeInTheDocument();
    expect(within(micSelect()).getByText("系统默认")).toBeInTheDocument();
  });
});

describe("DevicesCard —— 选项与选择", () => {
  it("两个下拉都带「系统默认」选项", async () => {
    renderCard();
    await waitFor(() => {
      expect(loadDevicesOptions(2));
    });
    expect(within(micSelect()).getByText("系统默认")).toBeInTheDocument();
    expect(within(outputSelect()).getByText("系统默认")).toBeInTheDocument();
    expect((micSelect().querySelector('option[value=""]') as HTMLOptionElement).textContent).toBe(
      "系统默认",
    );
  });

  it("选择设备时 patch 对应字段", async () => {
    const user = userEvent.setup();
    const { patch } = renderCard();
    await waitFor(() => {
      expect(within(micSelect()).getByText("内置麦克风")).toBeInTheDocument();
    });

    await user.selectOptions(micSelect(), "mic-2");
    expect(patch).toHaveBeenLastCalledWith({ micDevice: "mic-2" });

    await user.selectOptions(outputSelect(), "spk-1");
    expect(patch).toHaveBeenLastCalledWith({ outputDevice: "spk-1" });
  });

  it("已保存设备当前不在列表里：保留一个带占位的可见项", async () => {
    const loadDevices = vi.fn(() =>
      Promise.resolve({
        inputs: [{ id: "mic-1", name: "内置麦克风" }],
        outputs: [{ id: "spk-1", name: "扬声器" }],
        error: "",
      }),
    );
    renderCard({
      loadDevices,
      form: makeForm({ micDevice: "mic-old", outputDevice: "spk-old" }),
    });

    await waitFor(() => {
      expect(
        within(micSelect()).getByText("已保存设备（当前未检测到）"),
      ).toBeInTheDocument();
    });
    expect(micSelect().value).toBe("mic-old");
    expect(
      within(outputSelect()).getAllByText("已保存设备（当前未检测到）"),
    ).toHaveLength(1);
    expect(outputSelect().value).toBe("spk-old");
  });

  it("已保存设备还在列表里时不出现占位项", async () => {
    renderCard({ form: makeForm({ micDevice: "mic-1", outputDevice: "spk-1" }) });
    await waitFor(() => {
      expect(within(micSelect()).getByText("内置麦克风")).toBeInTheDocument();
    });
    expect(screen.queryByText("已保存设备（当前未检测到）")).not.toBeInTheDocument();
  });

  it("表单里没选设备时默认停在「系统默认」", async () => {
    renderCard();
    await waitFor(() => {
      expect(within(micSelect()).getByText("内置麦克风")).toBeInTheDocument();
    });
    expect(micSelect().value).toBe("");
    expect(outputSelect().value).toBe("");
  });
});

/** 等到下拉里已经有设备选项（自动加载完成）。 */
function loadDevicesOptions(count: number) {
  expect(micSelect().querySelectorAll("option").length).toBeGreaterThan(count);
}