import { describe, expect, it } from "vitest";
import {
  isRecord,
  numberValue,
  pickNumber,
  pickText,
  textValue,
} from "./data";

describe("isRecord", () => {
  it("普通对象与 Object.create(null) 都是 record", () => {
    expect(isRecord({})).toBe(true);
    expect(isRecord({ a: 1 })).toBe(true);
    expect(isRecord(Object.create(null))).toBe(true);
  });

  it("null / undefined / 原始值都不是 record", () => {
    expect(isRecord(null)).toBe(false);
    expect(isRecord(undefined)).toBe(false);
    expect(isRecord("text")).toBe(false);
    expect(isRecord(1)).toBe(false);
    expect(isRecord(true)).toBe(false);
    expect(isRecord(Symbol("s"))).toBe(false);
    expect(isRecord(10n)).toBe(false);
  });

  it("数组不是 record（否则 pickText 会把下标当字段名）", () => {
    expect(isRecord([])).toBe(false);
    expect(isRecord(["a", "b"])).toBe(false);
  });

  it("函数不是 record", () => {
    expect(isRecord(() => undefined)).toBe(false);
  });
});

describe("textValue", () => {
  it("字符串原样返回，包括空串", () => {
    expect(textValue("hello")).toBe("hello");
    expect(textValue("")).toBe("");
    expect(textValue("  空格 ")).toBe("  空格 ");
  });

  it("有限数字转成字符串", () => {
    expect(textValue(0)).toBe("0");
    expect(textValue(42)).toBe("42");
    expect(textValue(-1.5)).toBe("-1.5");
  });

  it("NaN / Infinity 不当作文本（避免把脏数字上屏）", () => {
    expect(textValue(Number.NaN)).toBe("");
    expect(textValue(Number.POSITIVE_INFINITY)).toBe("");
    expect(textValue(Number.NEGATIVE_INFINITY)).toBe("");
  });

  it("null / undefined / 布尔 / 对象 / 数组一律返回空串", () => {
    expect(textValue(null)).toBe("");
    expect(textValue(undefined)).toBe("");
    expect(textValue(false)).toBe("");
    expect(textValue(true)).toBe("");
    expect(textValue({ text: "nested" })).toBe("");
    expect(textValue(["a"])).toBe("");
  });
});

describe("numberValue", () => {
  it("有限数字直接返回", () => {
    expect(numberValue(0)).toBe(0);
    expect(numberValue(0.88)).toBe(0.88);
    expect(numberValue(-3)).toBe(-3);
  });

  it("数字字符串会被解析", () => {
    expect(numberValue("0.5")).toBe(0.5);
    expect(numberValue(" 12 ")).toBe(12);
    expect(numberValue("-2.5")).toBe(-2.5);
  });

  it("空串与纯空白返回 null，而不是 0", () => {
    expect(numberValue("")).toBeNull();
    expect(numberValue("   ")).toBeNull();
    expect(numberValue("\t\n")).toBeNull();
  });

  it("不可解析字符串返回 null", () => {
    expect(numberValue("abc")).toBeNull();
    expect(numberValue("12px")).toBeNull();
    expect(numberValue("0x10")).toBe(16);
  });

  it("NaN / Infinity / null / undefined / 布尔 / 对象返回 null", () => {
    expect(numberValue(Number.NaN)).toBeNull();
    expect(numberValue(Number.POSITIVE_INFINITY)).toBeNull();
    expect(numberValue(null)).toBeNull();
    expect(numberValue(undefined)).toBeNull();
    expect(numberValue(true)).toBeNull();
    expect(numberValue({})).toBeNull();
    expect(numberValue([1])).toBeNull();
  });
});

describe("pickText", () => {
  it("按顺序返回第一个非空文本字段", () => {
    expect(pickText({ text: "首句", delta: "增量" }, ["text", "delta"])).toBe("首句");
    expect(pickText({ text: "", delta: "增量" }, ["text", "delta"])).toBe("增量");
  });

  it("字段全部缺失时返回空串", () => {
    expect(pickText({}, ["text", "delta"])).toBe("");
    expect(pickText({ other: "无关" }, ["text", "delta"])).toBe("");
  });

  it("字段存在但类型不对时继续往下找", () => {
    const payload = { text: { nested: "不是字符串" }, delta: 12 };
    expect(pickText(payload, ["text", "delta"])).toBe("12");
  });

  it("嵌套字段缺失不会被拍平读取", () => {
    const payload = { data: { text: "深层的" } };
    expect(pickText(payload, ["text"])).toBe("");
    expect(pickText(payload, ["data"])).toBe("");
  });

  it("字段值为 null / undefined 时跳过", () => {
    expect(pickText({ text: null, delta: undefined, answer: "兜底" }, [
      "text",
      "delta",
      "answer",
    ])).toBe("兜底");
  });

  it("数字 0 也算可读文本，不会被当成空", () => {
    expect(pickText({ text: 0 }, ["text"])).toBe("0");
  });

  it("空的 keys 列表直接返回空串", () => {
    expect(pickText({ text: "有值" }, [])).toBe("");
  });
});

describe("pickNumber", () => {
  it("返回第一个可解析的数字", () => {
    expect(pickNumber({ mic: "0.4", system: 0.9 }, ["mic", "system"])).toBe(0.4);
    expect(pickNumber({ mic: "坏值", system: 0.9 }, ["mic", "system"])).toBe(0.9);
  });

  it("0 是合法结果，不会被当作缺失", () => {
    expect(pickNumber({ mic: 0 }, ["mic"])).toBe(0);
  });

  it("全部不可解析时返回 null", () => {
    expect(pickNumber({}, ["mic"])).toBeNull();
    expect(pickNumber({ mic: "" }, ["mic"])).toBeNull();
  });
});