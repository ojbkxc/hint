import { describe, expect, it } from "vitest";
import {
  DEFAULT_OVERLAY_CONTENT_PROTECTED,
  DEFAULT_STT_LANGUAGE,
  DEFAULT_OVERLAY_FONT_SCALE,
  DEFAULT_OVERLAY_OPACITY,
  MAX_OVERLAY_OPACITY,
  MIN_OVERLAY_OPACITY,
  normalizeBaseUrl,
  normalizeOpacity,
  normalizeSettings,
} from "./types";

describe("normalizeOpacity", () => {
  it("区间内的值原样通过", () => {
    expect(normalizeOpacity(0.2)).toBe(0.2);
    expect(normalizeOpacity(0.88)).toBe(0.88);
    expect(normalizeOpacity(1)).toBe(1);
    expect(normalizeOpacity(0.5)).toBe(0.5);
  });

  it("越界值被钳到边界", () => {
    expect(normalizeOpacity(0)).toBe(MIN_OVERLAY_OPACITY);
    expect(normalizeOpacity(-5)).toBe(MIN_OVERLAY_OPACITY);
    expect(normalizeOpacity(1.5)).toBe(MAX_OVERLAY_OPACITY);
    expect(normalizeOpacity(100)).toBe(MAX_OVERLAY_OPACITY);
  });

  it("NaN / Infinity 回落默认值而不是钳到边界", () => {
    expect(normalizeOpacity(Number.NaN)).toBe(DEFAULT_OVERLAY_OPACITY);
    expect(normalizeOpacity(Number.POSITIVE_INFINITY)).toBe(
      DEFAULT_OVERLAY_OPACITY,
    );
    expect(normalizeOpacity(Number.NEGATIVE_INFINITY)).toBe(
      DEFAULT_OVERLAY_OPACITY,
    );
  });

  it("非空数字字符串会被解析后再钳位", () => {
    expect(normalizeOpacity("0.5")).toBe(0.5);
    expect(normalizeOpacity(" 0.9 ")).toBe(0.9);
    expect(normalizeOpacity("9")).toBe(MAX_OVERLAY_OPACITY);
    expect(normalizeOpacity("0")).toBe(MIN_OVERLAY_OPACITY);
  });

  it("空串 / 脏字符串 / null / undefined / 对象回落默认值", () => {
    expect(normalizeOpacity("")).toBe(DEFAULT_OVERLAY_OPACITY);
    expect(normalizeOpacity("   ")).toBe(DEFAULT_OVERLAY_OPACITY);
    expect(normalizeOpacity("abc")).toBe(DEFAULT_OVERLAY_OPACITY);
    expect(normalizeOpacity(null)).toBe(DEFAULT_OVERLAY_OPACITY);
    expect(normalizeOpacity(undefined)).toBe(DEFAULT_OVERLAY_OPACITY);
    expect(normalizeOpacity(true)).toBe(DEFAULT_OVERLAY_OPACITY);
    expect(normalizeOpacity({})).toBe(DEFAULT_OVERLAY_OPACITY);
  });

  it("默认值本身落在合法区间内", () => {
    expect(DEFAULT_OVERLAY_OPACITY).toBeGreaterThanOrEqual(
      MIN_OVERLAY_OPACITY,
    );
    expect(DEFAULT_OVERLAY_OPACITY).toBeLessThanOrEqual(MAX_OVERLAY_OPACITY);
  });
});

describe("normalizeSettings", () => {
  it("null / undefined / 非对象输入返回空设置副本", () => {
    for (const raw of [null, undefined, "text", 42, true]) {
      expect(normalizeSettings(raw)).toEqual({
        provider: "openai",
        baseUrl: "",
        model: "",
        resume: "",
        projects: "",
        answerStyle: "concise",
        triggerMode: "auto",
        micDevice: "",
        outputDevice: "",
        overlayOpacity: DEFAULT_OVERLAY_OPACITY,
        overlayFontScale: DEFAULT_OVERLAY_FONT_SCALE,
        overlayContentProtected: DEFAULT_OVERLAY_CONTENT_PROTECTED,
        proxyMode: "system",
        proxyUrl: "",
        sttLanguage: DEFAULT_STT_LANGUAGE,
        schemaVersion: 1,
      });
    }
  });

  it("空对象补齐全部默认字段", () => {
    const settings = normalizeSettings({});
    expect(settings.provider).toBe("openai");
    expect(settings.answerStyle).toBe("concise");
    expect(settings.triggerMode).toBe("auto");
    expect(settings.overlayOpacity).toBe(DEFAULT_OVERLAY_OPACITY);
    expect(settings.micDevice).toBe("");
  });

  it("合法字段全部透传", () => {
    const settings = normalizeSettings({
      provider: "anthropic",
      baseUrl: "https://api.anthropic.com",
      model: "claude-sonnet-5-5",
      resume: "简历",
      projects: "项目",
      answerStyle: "outline",
      triggerMode: "manual",
      micDevice: "mic-1",
      outputDevice: "spk-2",
      overlayOpacity: 0.5,
      overlayFontScale: DEFAULT_OVERLAY_FONT_SCALE,
        overlayContentProtected: DEFAULT_OVERLAY_CONTENT_PROTECTED,
        proxyMode: "system",
        proxyUrl: "",
        sttLanguage: DEFAULT_STT_LANGUAGE,
        schemaVersion: 1,
    });
    expect(settings).toEqual({
      provider: "anthropic",
      baseUrl: "https://api.anthropic.com",
      model: "claude-sonnet-5-5",
      resume: "简历",
      projects: "项目",
      answerStyle: "outline",
      triggerMode: "manual",
      micDevice: "mic-1",
      outputDevice: "spk-2",
      overlayOpacity: 0.5,
      overlayFontScale: DEFAULT_OVERLAY_FONT_SCALE,
        overlayContentProtected: DEFAULT_OVERLAY_CONTENT_PROTECTED,
        proxyMode: "system",
        proxyUrl: "",
        sttLanguage: DEFAULT_STT_LANGUAGE,
        schemaVersion: 1,
    });
  });

  it("provider 非法值回落 openai，gemini 保留", () => {
    expect(normalizeSettings({ provider: "gemini" }).provider).toBe("gemini");
    expect(normalizeSettings({ provider: "xxx" }).provider).toBe("openai");
    expect(normalizeSettings({ provider: 1 }).provider).toBe("openai");
  });

  it("字符串字段传了非字符串类型时回落空串而不是抛错", () => {
    const settings = normalizeSettings({
      baseUrl: 42,
      model: { nested: true },
      resume: null,
      projects: ["a"],
    });
    expect(settings.baseUrl).toBe("");
    expect(settings.model).toBe("");
    expect(settings.resume).toBe("");
    expect(settings.projects).toBe("");
  });

  it("answerStyle / triggerMode 非法值回落合法枚举", () => {
    expect(normalizeSettings({ answerStyle: "fancy" }).answerStyle).toBe(
      "concise",
    );
    expect(normalizeSettings({ answerStyle: 1 }).answerStyle).toBe("concise");
    expect(normalizeSettings({ triggerMode: "both" }).triggerMode).toBe(
      "auto",
    );
  });

  it("蛇形 overlay_opacity 也被识别并钳位", () => {
    const snake = normalizeSettings({ overlay_opacity: 9 });
    expect(snake.overlayOpacity).toBe(MAX_OVERLAY_OPACITY);
    const both = normalizeSettings({ overlayOpacity: 0.5, overlay_opacity: 0.9 });
    expect(both.overlayOpacity).toBe(0.5);
  });

  it("overlayOpacity 非数字回落默认值", () => {
    expect(normalizeSettings({ overlayOpacity: "abc" }).overlayOpacity).toBe(
      DEFAULT_OVERLAY_OPACITY,
    );
    expect(normalizeSettings({ overlayOpacity: null }).overlayOpacity).toBe(
      DEFAULT_OVERLAY_OPACITY,
    );
  });
});

describe("normalizeBaseUrl", () => {
  it("空输入返回空串", () => {
    expect(normalizeBaseUrl("", "openai")).toBe("");
    expect(normalizeBaseUrl("   ", "openai")).toBe("");
  });

  it("缺协议头补 https://，openai 缺版本段补 /v1", () => {
    expect(normalizeBaseUrl("api.deepseek.com", "openai")).toBe(
      "https://api.deepseek.com/v1",
    );
  });

  it("误贴完整端点时剥掉再补版本段", () => {
    expect(
      normalizeBaseUrl("https://api.openai.com/v1/chat/completions", "openai"),
    ).toBe("https://api.openai.com/v1");
  });

  it("anthropic / gemini 保持域名", () => {
    expect(normalizeBaseUrl("https://api.anthropic.com", "anthropic")).toBe(
      "https://api.anthropic.com",
    );
    expect(
      normalizeBaseUrl(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini:generateContent",
        "gemini",
      ),
    ).toBe("https://generativelanguage.googleapis.com/v1beta");
  });
});

describe("事件与容量契约", () => {
  it("事件名都是 hint:// 前缀且不含非法字符", async () => {
    const { HINT_EVENTS } = await import("../overlay/types");
    for (const name of Object.values(HINT_EVENTS)) {
      expect(name.startsWith("hint://")).toBe(true);
      expect(name).toMatch(/^[a-z0-9:/-]+$/);
    }
  });

  it("不透明度边界常量与浮层契约一致", () => {
    expect(MIN_OVERLAY_OPACITY).toBe(0.2);
    expect(MAX_OVERLAY_OPACITY).toBe(1);
  });

  it("代理默认跟随系统；显式 direct / manual 仍然生效", () => {
    // 默认 system：装了代理软件的机器不用手动配就能连上。
    expect(normalizeSettings({}).proxyMode).toBe("system");
    // 老配置里没有这个字段，同样按 system 处理。
    expect(normalizeSettings({ provider: "openai" }).proxyMode).toBe("system");
    // 显式选择必须被尊重——默认值改了不等于用户不能选直连。
    expect(normalizeSettings({ proxyMode: "direct" }).proxyMode).toBe("direct");
    expect(normalizeSettings({ proxyMode: "manual" }).proxyMode).toBe("manual");
    // 非法值回落默认。
    expect(normalizeSettings({ proxyMode: "bogus" }).proxyMode).toBe("system");
  });

  it("schemaVersion 必须原样透传（丢了会让后端重复迁移）", () => {
    // 后端靠它判断要不要跑一次性旧值修正；前端丢掉会让版本归零，
    // 下次加载重新迁移，把用户手动打开的内容保护强行关掉。
    expect(normalizeSettings({ schemaVersion: 1 }).schemaVersion).toBe(1);
    expect(normalizeSettings({ schemaVersion: 7 }).schemaVersion).toBe(7);
    // 缺失时给当前版本（1），而不是 0——0 会触发迁移。
    expect(normalizeSettings({}).schemaVersion).toBe(1);
    // 脏值同样回落当前版本。
    expect(normalizeSettings({ schemaVersion: "bad" }).schemaVersion).toBe(1);
  });
});