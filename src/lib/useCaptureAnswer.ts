import { useCallback, useEffect, useState } from "react";
import { isRecord, pickText } from "./data";
import { safeInvoke, safeListen } from "./tauri";

/**
 * 截图问答的流式增量事件（M5 契约，事件名不可改）。
 *
 * 故意不加进 overlay/types.ts 的 HINT_EVENTS：那份常量是「useOverlayEvents
 * 订阅了哪些事件」的唯一事实来源，单测按 `Object.keys(HINT_EVENTS).length`
 * 校验注册数量。截图问答由本 hook 单独订阅，混进去会让那条断言与真实订阅
 * 脱节。
 */
export const CAPTURE_ANSWER_EVENT = "hint://capture-answer-delta";

/**
 * 截图问答的显示状态。
 *
 * 与面试答案（AnswerState）完全独立：截图是「一次性看图问答」，没有排队、
 * 没有归档历史，也不该被会话状态（status=idle）清掉。
 */
export type CaptureAnswerState = {
  text: string;
  streaming: boolean;
  error: string;
  model: string;
};

const EMPTY_CAPTURE_ANSWER: CaptureAnswerState = {
  text: "",
  streaming: false,
  error: "",
  model: "",
};

/** payload 形状：{ text, reset, done, error?, model? }（字段缺失按缺省处理）。 */
export function useCaptureAnswer() {
  const [state, setState] = useState<CaptureAnswerState>(EMPTY_CAPTURE_ANSWER);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;

    void safeListen<unknown>(CAPTURE_ANSWER_EVENT, (payload) => {
      if (disposed || !isRecord(payload)) return;
      const chunk = pickText(payload, ["text", "delta"]);
      const reset = payload.reset === true;
      const done = payload.done === true;
      const error = pickText(payload, ["error"]);
      const model = pickText(payload, ["model"]);
      setState((current) => {
        if (disposed) return current;
        // reset 帧代表新一轮截图：整段替换，不拼到上一次的答案后面。
        const text = reset ? chunk : current.text + chunk;
        const nextModel = model || current.model;
        // 错误帧优先：错误上屏，并把流式状态收掉（之后不会再有内容）。
        if (error !== "") {
          return { text, streaming: false, error, model: nextModel };
        }
        // done 帧收尾：停止光标闪烁，但不丢弃已收到的文本。
        return { text, streaming: !done, error: "", model: nextModel };
      });
    }).then((dispose) => {
      if (!dispose) return;
      // 注册是异步的：卸载可能发生在 listen 返回之前，这里补一次解绑，
      // 否则浮层窗口关掉后监听器会留在事件总线上。
      if (disposed) dispose();
      else unlisten = dispose;
    });

    return () => {
      disposed = true;
      if (unlisten) {
        unlisten();
        unlisten = null;
      }
    };
  }, []);

  /** 收起截图问答区；顺带尽力取消后端还在跑的截图问答（失败不打扰用户）。 */
  const dismiss = useCallback(() => {
    setState(EMPTY_CAPTURE_ANSWER);
    void safeInvoke("cancel_capture_answer");
  }, []);

  return { ...state, dismiss };
}
