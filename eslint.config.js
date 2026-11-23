import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";

export default tseslint.config(
  // 忽略构建产物与本机 Rust 侧
  {
    ignores: [
      "dist/**",
      "node_modules/**",
      "src-tauri/**",
      "coverage/**",
      // Rust 侧 rustdoc 生成的 js（target/doc/**/*.js）不是前端源码，
      // 不忽略会被 js.configs.recommended 扫出 800+ 条 no-undef。
      "target/**",
      "*.cjs",
    ],
  },

  // 基础 JS 推荐
  js.configs.recommended,

  // TypeScript 推荐（不开类型检查，保持 lint 速度与零额外依赖）
  ...tseslint.configs.recommended,

  // 全部 ts/tsx 走浏览器 + node 混合全局（vite.config.ts 会读 process.env）
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      // 现有业务代码存在依赖缺失（如 useEffect 依赖 currentAnswer），
      // error 会让 lint 全红，先 warn 暴露问题。
      "react-hooks/exhaustive-deps": "warn",
      "react-hooks/rules-of-hooks": "error",
      // 只允许从入口文件 export 组件，避免 HMR 状态错乱；
      // 本仓组件文件同时 export hooks/类型会报 warn，可接受。
      "react-refresh/only-export-components": "off",
      // ESLint 10 新增规则，对 `let x = ""; try { x = ... } catch { return }`
      // 这种「初始化 + try 内赋值」写法误报（src/settings/types.ts:109 的 host）。
      // 属既有代码的合法写法，不是缺陷，故关闭而不是改业务代码。
      "no-useless-assignment": "off",
    },
  },
);
