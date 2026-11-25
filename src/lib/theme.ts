import { normalizeTheme, type Theme } from "../settings/types";

/**
 * 主题落点：根元素上的 `data-theme` 属性。
 *
 * 为什么是属性而不是 class：`data-theme` 在 DevTools 里一眼能看出当前主题，
 * 也不会与组件自己的 className 抢命名空间；CSS 侧用 `[data-theme="dark"]`
 * 覆盖 `:root` 的语义化配色变量，浅色规则一字不改。
 *
 * 只在 `dark` 时写属性，`light` 时整个删掉：没有属性的状态就是浅色，
 * 这样「浅色」永远等于升级前的 DOM，不会因为多一个属性触发任何意外样式。
 */
export function applyTheme(raw: unknown): Theme {
  const theme = normalizeTheme(raw);
  const root = document.documentElement;
  if (theme === "dark") {
    root.dataset.theme = "dark";
  } else {
    delete root.dataset.theme;
  }
  return theme;
}
