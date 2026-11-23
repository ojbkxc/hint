import { describe, expect, it } from "vitest";
import { deriveStatusTone, statusToneClass, type StatusTone } from "./statusTone";

describe("deriveStatusTone —— 空文本与未知文本", () => {
  it("空串与纯空白都是 info", () => {
    expect(deriveStatusTone("")).toBe("info");
    expect(deriveStatusTone("   \n\t ")).toBe("info");
  });

  it("中立文案不染色", () => {
    expect(deriveStatusTone("正在读取配置文件")).toBe("info");
    expect(deriveStatusTone("已连接，等待指令")).toBe("info");
  });

  it("首尾空白不影响判定", () => {
    expect(deriveStatusTone("   已保存配置   ")).toBe("success");
  });
});

describe("deriveStatusTone —— 成功关键词", () => {
  const successCases = [
    "设置已保存",
    "设备列表已获取",
    "热键连接成功",
    "配置已导入",
    "会话已删除",
    "全部 3 条会话记录已清空",
    "已导出 Markdown 文件",
    "模型文件检查完成，可以开始面试。",
    "会话已启动",
    "会话已结束",
  ];

  it.each(successCases)("%s → success", (text) => {
    expect(deriveStatusTone(text)).toBe("success");
  });
});

describe("deriveStatusTone —— 失败关键词", () => {
  it("「失败」与「错误」都判 error", () => {
    expect(deriveStatusTone("导出失败")).toBe("error");
    expect(deriveStatusTone("保存失败：磁盘只读")).toBe("error");
    expect(deriveStatusTone("读取错误")).toBe("error");
    expect(deriveStatusTone("配置错误：字段缺失")).toBe("error");
  });

  it("半角冒号同样命中", () => {
    expect(deriveStatusTone("连接失败: 超时")).toBe("error");
    expect(deriveStatusTone("解析错误: 无效 JSON")).toBe("error");
  });

  it("一句话里既有成功词又有失败词时按 error 处理", () => {
    // 不带「但」等 warning 标记，成功词不掩盖失败语义
    expect(deriveStatusTone("已保存，写入失败")).toBe("error");
    expect(deriveStatusTone("已获取 0 条，请求失败")).toBe("error");
  });

  it("失败相关但不是「失败/错误」字样的文案仍是 info", () => {
    // 没有关键词时不猜语义，保持 info（展示层只做关键词派生）
    expect(deriveStatusTone("无法读取设备")).toBe("info");
    expect(deriveStatusTone("取消下载")).toBe("info");
  });
});

describe("deriveStatusTone —— 警告关键词优先", () => {
  it("「但」判 warning，即便同时含失败字样", () => {
    expect(deriveStatusTone("已保存，但有一项需要重启")).toBe("warning");
    expect(deriveStatusTone("已导入，但解析失败")).toBe("warning");
  });

  it("「未完成」判 warning", () => {
    expect(deriveStatusTone("模型下载未完成：网络不可达")).toBe("warning");
  });

  it("「没有返回可用模型」判 warning", () => {
    expect(deriveStatusTone("接口连通，但没有返回可用模型")).toBe("warning");
  });

  it("warning 优先于成功词（同一句里同时出现）", () => {
    expect(deriveStatusTone("已保存，但未完成校验")).toBe("warning");
  });
});

describe("statusToneClass —— 语义色到类名", () => {
  it("四个语义各自映射到既有类名", () => {
    const table: Array<[StatusTone, string]> = [
      ["success", "status"],
      ["warning", "status-warning"],
      ["error", "error-text"],
      ["info", "status-info"],
    ];
    for (const [tone, className] of table) {
      expect(statusToneClass(tone)).toBe(className);
    }
  });

  it("成功色沿用旧的 status 类（不是 status-success）", () => {
    // 这是最容易改坏的一处：误改成 status-success 会让现有 CSS 失效
    expect(statusToneClass("success")).not.toBe("status-success");
    expect(statusToneClass(deriveStatusTone("已保存"))).toBe("status");
  });

  it("未知语义回落 info 类名", () => {
    expect(statusToneClass("unknown" as StatusTone)).toBe("status-info");
  });

  it("deriveStatusTone 的输出可以直接喂给 statusToneClass", () => {
    expect(statusToneClass(deriveStatusTone("清空失败：权限不足"))).toBe("error-text");
    expect(statusToneClass(deriveStatusTone("模型下载未完成"))).toBe("status-warning");
    expect(statusToneClass(deriveStatusTone("已导出 Markdown 文件"))).toBe("status");
    expect(statusToneClass(deriveStatusTone("随便什么"))).toBe("status-info");
  });
});