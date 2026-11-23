# hint 配置页 UI 改造方案

- 适用项目：`E:\github\hint`（Tauri 面试提词器）
- 方案性质：UI/样式层改造，**不改变任何功能行为**
- 结论来源：
  - 现状问题：任务 1《hint 配置页 UI 现状问题清单》（hint-current）
  - 设计语言：任务 2《AIGX 前端 UI 设计语言提炼报告》（aigx-style）
  - 补充核对：team-lead 转达的「`.status` 双用途根因」「全局按钮缺焦点环」两条
- 落点文件：`src/App.css`（全局 token 与控件）、`src/**/*.tsx`（配置页与复盘页结构/类名）
- **禁止改动**：`crates/**`、`src-tauri/**` 任何 Rust 代码；`src/**/*.ts` 中的 hook（`useSettingsForm`、`useModelManager`、`useSessionControl`、`useSessions`、`useOverlayEvents`）语义。

---

## 1. 改造目标与总体原则

1. **风格对齐 AIGX**：浅色、卡片化、克制语义色、清晰聚焦环。保留 hint 自有品牌蓝 `#2f6bff` 作为强调色（AIGX 的近黑主色属于其自身品牌，不照搬色值，只照搬结构规范）。
2. **设计令牌化**：字号、间距、圆角、控件高度、阴影、过渡全部改为 CSS 变量，组件规则禁止硬编码数值/色值。
3. **语义反馈三色化**：成功=绿、警告=黄、错误=红、信息=蓝；不允许「成功/失败同色」。
4. **可访问性**：所有可聚焦控件（含 button / tab / session-item）统一 `:focus-visible` 焦点环；动态提示加 live region；Tab 补齐 `tabpanel` 关系。
5. **零功能变更**：不改 hook 语义、不改后端调用、不改文案含义、不改快捷键与状态机。

---

## 2. 全局设计 Token（`src/App.css` 顶部 `:root`）

### 2.1 命名约定

- 沿用 hint 现有语义变量名（`--bg` / `--surface` / `--text` / `--accent` …），保证浮层等既有代码不受影响。
- 新增 token 采用分组前缀：色板无前缀、`--radius-*`、`--shadow-*`、`--space-*`、`--font-*`、`--control-*`、`--transition-*` / `--ease-*`。
- **禁止**在组件规则里出现新的十六进制色值、`px` 字号、`px` 圆角、`px` 间距、（过渡时长）字面量。确需一次性数值时，必须先在 `:root` 定义 token 再引用。

### 2.2 色板（保留 + 微调 + 新增）

| 变量 | 取值 | 说明 |
| --- | --- | --- |
| `--bg` | `#f2f3f5` | 保留 |
| `--bg-glow` | `rgba(64,120,255,0.08)` | 保留 |
| `--surface` | `#ffffff` | 卡片/面板底，保留 |
| `--surface-muted` | `rgba(255,255,255,0.72)` | Tab 容器等，保留 |
| `--surface-input` | `#ffffff` | 输入控件底，保留 |
| `--bg-hover` | `rgba(0,0,0,0.045)` | 保留 |
| `--bg-selected` | `rgba(47,107,255,0.1)` | 保留 |
| `--border` | `rgba(0,0,0,0.12)` | 保留 |
| `--border-strong` | `rgba(0,0,0,0.2)` | 保留 |
| `--separator` | `rgba(0,0,0,0.07)` | 保留 |
| `--text` | `#1d1d1f` | 保留 |
| `--text-secondary` | `#3f4147` | 保留 |
| `--text-muted` | `#5f5f63` | 保留 |
| `--text-dim` | `#6e6e75` | **调整**（原 `#7c7c82` 对比度贴近 AA 下限，调深） |
| `--accent` | `#2f6bff` | 保留（品牌蓝） |
| `--accent-hover` | `#1e57e8` | 保留 |
| `--accent-soft` | `rgba(47,107,255,0.12)` | 由 0.10 微调到 12%，对齐浅底规范 |
| `--accent-border` | `rgba(47,107,255,0.35)` | 保留 |
| `--on-accent` | `#ffffff` | 保留 |
| `--focus-ring` | `rgba(47,107,255,0.28)` | 保留（输入光晕） |
| `--focus-ring-strong` | `rgba(47,107,255,0.55)` | **新增**（按钮外环） |
| `--focus-ring-width` | `2px` | **新增** |
| `--focus-ring-offset` | `2px` | **新增** |
| `--success` | `#1f7a4d` | 保留 |
| `--success-text` | `#1f7a4d` | **新增**（浅底深字别名） |
| `--success-soft` | `rgba(31,122,77,0.14)` | **调整**为统一 14% 浅底 |
| `--success-border` | `rgba(31,122,77,0.32)` | 保留 |
| `--warning` | `#946208` | 保留 |
| `--warning-text` | `#946208` | **新增** |
| `--warning-soft` | `rgba(148,98,8,0.14)` | **调整**为 14% |
| `--warning-border` | `rgba(148,98,8,0.34)` | 保留 |
| `--danger` | `#c9342f` | 保留 |
| `--danger-text` | `#c9342f` | **新增** |
| `--danger-soft` | `rgba(201,52,47,0.12)` | **调整**为 12% |
| `--danger-border` | `rgba(201,52,47,0.34)` | 保留 |
| `--scroll-thumb` | `rgba(0,0,0,0.18)` | 保留 |
| `--overlay-alpha` | （不在此定义） | 浮层透明度由既有浮层逻辑/后端配置写入，CSS 内继续用 `var(--overlay-alpha, 0.88)` 兜底，**不要覆盖** |

> 说明：hint 当前没有深色主题机制，本次不新增。token 全部变量化后，后续如需深色主题，只需在 `[data-theme="dark"]` 覆盖同名变量即可，组件规则零改动。

### 2.3 圆角

| 变量 | 取值 | 用途 |
| --- | --- | --- |
| `--radius-card` | `12px` | 卡片、Tab 容器（AIGX 卡片 12px） |
| `--radius-modal` | `14px` | 确认弹层 / 浮层（AIGX modal 14px） |
| `--radius-control` | `8px` | 输入框、下拉、按钮内嵌块 |
| `--radius-inline` | `6px` | 内嵌项（session-item、answer-box、mini 按钮） |
| `--radius-pill` | `9999px` | 按钮、徽章、状态胶囊、开关 |

### 2.4 阴影

| 变量 | 取值 |
| --- | --- |
| `--shadow-control` | `0 1px 2px rgba(20,22,28,0.06)`（保留） |
| `--shadow-card` | `0 1px 2px rgba(20,22,28,0.04), 0 8px 24px rgba(20,22,28,0.05)`（保留） |
| `--shadow-hover` | `0 4px 16px rgba(20,22,28,0.10), 0 1px 3px rgba(20,22,28,0.06)`（**新增**，按钮/卡片悬停浮起） |
| `--shadow-popover` | `0 16px 44px rgba(20,22,28,0.14), 0 2px 8px rgba(20,22,28,0.06)`（保留） |
| `--shadow-overlay` | `0 10px 36px rgba(15,18,25,0.22)`（保留） |

### 2.5 间距

| 变量 | 取值 | 用途 |
| --- | --- | --- |
| `--space-1` | `7px` | 紧凑内距 |
| `--space-2` | `10px` | 控件间 |
| `--space-3` | `12px` | 卡内字段间、栅格 gap |
| `--space-4` | `16px` | 区块间 |
| `--space-card` | `20px` | 卡体 padding |
| `--space-card-head-y` | `16px` | 卡头纵向 |
| `--space-card-head-x` | `20px` | 卡头横向 |
| `--space-page-y` | `28px` | 页面上下内边距 |
| `--space-page-x` | `32px` | 页面左右内边距 |

### 2.6 字号

| 变量 | 取值 | 用途 |
| --- | --- | --- |
| `--font-xs` | `10.5px` | 最小标签、tag |
| `--font-sm` | `12px` | 辅助文案、字段标签、小按钮 |
| `--font-md` | `14px` | 正文 |
| `--font-lg` | `16px` | 卡片标题 |
| `--font-xl` | `20px` | 页面标题 |
| `--font-stat` | `28px` | 统计/大数值（预留） |

### 2.7 控件高度

| 变量 | 取值 | 用途 |
| --- | --- | --- |
| `--control-h` | `40px` | 单行输入 / 下拉 |
| `--control-h-btn` | `36px` | 常规按钮 |
| `--control-h-sm` | `28px` | 小按钮 / ghost / preset |
| `--control-h-icon` | `26px` | `.mini` 图标微按钮 |
| `--control-h-row` | `30px` | 列表行（session-item 最小高度参考） |

### 2.8 过渡与缓动

| 变量 | 取值 |
| --- | --- |
| `--transition-fast` | `120ms` |
| `--transition-base` | `150ms` |
| `--transition-slow` | `250ms` |
| `--ease-out` | `cubic-bezier(0.16, 1, 0.3, 1)` |

### 2.9 层级（新增）

| 变量 | 取值 |
| --- | --- |
| `--z-confirm` | `1000` |

---

## 3. 基础控件类名清单（**完整清单，实施严格按此使用，不得自造别名**）

> 约定：标注「保留」的类名不改名、不删；标注「新增」的类名必须按此拼写；标注「调整」的保留原名但样式走 token。所有普通 `button` 自动继承基础样式与全局焦点环。

### 3.1 卡片

| 类名 | 状态 | 说明 / 关键样式 |
| --- | --- | --- |
| `.card` | 调整 | 卡片容器：`--surface` 底、1px `--border`、`--radius-card`、`--shadow-card`、`display:flex;flex-direction:column;padding:0`；hover 边框 `--border-strong` |
| `.card-head` | 调整 | 卡头：左右分布、`padding: var(--space-card-head-y) var(--space-card-head-x)`、底部分隔线 `--separator`、标题 `--font-lg`/600 |
| `.card-body` | **新增** | 卡体：`padding: var(--space-card)`、`display:flex;flex-direction:column;gap:var(--space-3)` |
| `.save-card` | 保留 | 仅 `grid-column: 1/-1` |

> 实施要求：**每个 `.card` 必须包含一个 `.card-head`（标题）和一个 `.card-body`（内容）**；原裸 `<h2>` 一律移入 `.card-head`。

### 3.2 按钮

| 类名 | 状态 | 说明 |
| --- | --- | --- |
| `button`（元素） | 调整 | 基础：`height:var(--control-h-btn)`、`border-radius:var(--radius-pill)`、`padding:0 var(--space-4)`、inline-flex 居中、`--surface` 底、`--border`、`--shadow-control`、过渡 `--transition-base`、hover `translateY(-1px)` + `--shadow-hover`、disabled `opacity:.5` |
| `.primary` | 调整 | 实心强调：`--accent` 底（去掉渐变）、`--on-accent` 字、hover `--accent-hover` |
| `.secondary` | **新增** | 次按钮：`--surface-input` 底 + `--border` 描边，hover `--bg-hover` |
| `.ghost` | 调整 | 透明底、`--text-muted` 字、`height:var(--control-h-sm)`、`--font-sm`、无阴影 |
| `.danger` | 调整 | 危险：`--danger-soft` 底 + `--danger-border` 描边 + `--danger-text` 字，hover 加深（`--danger-soft` 叠加） |
| `.mini` | 调整 | 微按钮：`height:var(--control-h-icon)`、`--font-xs`、`--radius-inline`（浮层共用，见 §7） |
| `.small-danger` | 保留 | 危险小按钮：`height:var(--control-h-sm)`、`--font-sm` |
| `.m5` | 保留 | 截图问答按钮，inline-flex + gap，`--text-muted` |
| `.mini-import` | 保留 | ghost 修饰，`--accent` 字 + `--accent-border` |

### 3.3 输入 / 下拉 / 文本域

| 类名 | 状态 | 说明 |
| --- | --- | --- |
| `.form-input` | **新增** | 单行输入/下拉：`min-height:var(--control-h)`、`--radius-control`、`--surface-input` 底、聚焦 `--accent-border` + `0 0 0 3px var(--focus-ring)` |
| `.form-textarea` | **新增** | 多行：复用 `.form-input`，`resize:vertical`、`line-height:1.6` |
| `.input-error` | **新增** | 错误态：`--danger` 边框 + 红光晕 |
| `.field` | **新增** | 字段容器：`display:flex;flex-direction:column;gap:6px` |
| `.field-label` | **新增** | 字段文字：`--font-sm`、`--text-muted` |
| `.field-head` | 保留 | 字段标题行：左右分布、`gap:var(--space-2)` |
| `.inline-row` | 保留 | 横向排列；内部 `.form-input` 撑满 |
| `.inline-row.slider-row` | 保留 | 滑块行，垂直居中 |
| `.slider-value` | 保留 | 数值右对齐、`--font-sm`、tabular-nums |
| `.char-count` | **新增** | 字数计数：`--font-xs`、`--text-dim`、右对齐；`.char-count.is-over` 用 `--danger` |

> 实施要求：配置页所有 `input`/`select` 显式加 `className="form-input"`，`textarea` 加 `className="form-input form-textarea"`。字段标题统一用 `<label className="field-label" htmlFor="...">` 关联控件；导入等按钮必须放在 `<label>` **之外**。

### 3.4 开关（预留，本次未启用）

| 类名 | 状态 | 说明 |
| --- | --- | --- |
| `.switch` | **新增** | 36×20 胶囊、`--radius-pill`、`--border` 底 |
| `.switch-thumb` | **新增** | 14px 白圆、`transition: transform var(--transition-slow) var(--ease-out)` |
| `.switch-on` | **新增** | 开启：`--accent` 底、thumb 位移、thumb 色 `--on-accent` |

> 当前配置页没有开关控件；该组类名为 M5（截图问答等）预留。**实施时不得另起名字**。

### 3.5 徽章 / 状态

| 类名 | 状态 | 说明 |
| --- | --- | --- |
| `.badge` | **新增** | 通用徽章：`--radius-pill`、`--font-xs`、`padding:2px 8px` |
| `.badge-success` | **新增** | `--success-soft` 底 + `--success-text` 字 + `--success-border` |
| `.badge-warning` | **新增** | `--warning-soft` / `--warning-text` / `--warning-border` |
| `.badge-danger` | **新增** | `--danger-soft` / `--danger-text` / `--danger-border` |
| `.badge-info` | **新增** | `--accent-soft` / `--accent` / `--accent-border` |
| `.badge-neutral` | **新增** | `--bg-hover` 底 + `--text-dim` 字 + `--border` |
| `.tag-ok` | 保留 | 等价 `.badge-success`，保留兼容 |
| `.tag-m5` | 保留 | M5 预留徽章 |
| `.hint-badge` | 保留 | 页头快捷键徽标 |
| `.session-state` | 保留 | 会话状态胶囊（基础态） |
| `.session-state.state-listening` | 保留 | 绿 |
| `.session-state.state-connecting` | 保留 | 黄 |
| `.session-state.state-error` | 保留 | 红 |

### 3.6 Tab

| 类名 | 状态 | 说明 |
| --- | --- | --- |
| `.tab-bar` | 调整 | 内嵌容器：`--surface-muted` 底、`--radius-card`、`padding:4px`、`--shadow-control` |
| `.tab` | 调整 | 未激活：透明底、`--text-muted`、`--radius-control` |
| `.tab.active` | 调整 | 激活：`--surface` 白底 + `--shadow-control`（AIGX 白底投影），文字 `--accent` |
| `.tabpanel` | **新增** | 面板容器；`[hidden]` 时 `display:none` |

> 结构要求：`role="tablist"` 的 `.tab-bar` 内每个 `.tab` 需 `id` + `aria-controls`；面板 `.tabpanel` 需 `role="tabpanel"` + `aria-labelledby`。切换用 `hidden`/条件渲染均可，但必须有 tabpanel 关系。

### 3.7 滑块

| 类名 | 状态 | 说明 |
| --- | --- | --- |
| `.slider` | **新增** | `input[type=range]` 显式类：`accent-color:var(--accent)`、透明底、无边框、聚焦环 |
| `.slider-row` | 保留 | 行容器 |
| `.slider-value` | 保留 | 右侧数值 |

### 3.8 反馈 / 横幅

| 类名 | 状态 | 说明 |
| --- | --- | --- |
| `.dashboard-banner` | 保留 | 横幅基础；`--radius-control`、`--font-sm`、click 关闭 |
| `.banner-info` | 保留 | 蓝（`--accent-soft` / `--accent` / `--accent-border`） |
| `.banner-warning` | **新增** | 黄（`--warning-soft` / `--warning-text` / `--warning-border`） |
| `.banner-error` | 保留 | 红（`--danger-soft` / `--danger-text` / `--danger-border`） |
| `.status` | 调整 | **成功语义**：`--success-text`（原为蓝色，改绿） |
| `.status-info` | **新增** | 信息语义：`--accent` |
| `.status-warning` | **新增** | 警告语义：`--warning-text` |
| `.error-text` | 保留 | 错误语义：`--danger-text` |
| `.muted` | 调整 | 颜色改为 `--text-dim`（已调深） |
| `.small` | 调整 | 字号改为 `--font-sm`（12px） |
| `.visually-hidden` | **新增** | 读屏专用隐藏（视觉隐藏但可被读屏读取） |

### 3.9 空状态 / 骨架屏

| 类名 | 状态 | 说明 |
| --- | --- | --- |
| `.empty-state` | **新增** | 居中、`padding:var(--space-4)`、`display:flex;column;align-items:center;gap:var(--space-2)` |
| `.empty-state-icon` | **新增** | 36px 图标、`opacity:.5` |
| `.empty-state-text` | **新增** | `--font-sm`、`--text-dim`、居中 |
| `.skeleton` | **新增** | 骨架块：`--bg-hover` 底、`--radius-inline`、`overflow:hidden`、微光动画 |
| `.skeleton-text` | **新增** | 文本行骨架（固定高度 + 宽度百分比） |
| `@keyframes skeleton-shimmer` | **新增** | 骨架微光 |

### 3.10 确认弹层（供放开 `.ts` 边界后迁移，本次不替换 `window.confirm`，见 §6.3）

| 类名 | 状态 | 说明 |
| --- | --- | --- |
| `.confirm-overlay` | **新增** | 全屏遮罩：`rgba(0,0,0,0.35)`、`z-index:var(--z-confirm)` |
| `.confirm-panel` | **新增** | 弹层：`--surface` 底、`--radius-modal`、`--shadow-popover`、`padding:var(--space-card)` |
| `.confirm-title` | **新增** | 标题 `--font-lg`/600 |
| `.confirm-message` | **新增** | 正文 `--font-md`、保留换行语义 |
| `.confirm-actions` | **新增** | 底部按钮行，右对齐、`gap:var(--space-2)` |
| `.confirm-panel.confirm-danger` | **新增** | 危险确认强调（标题/主按钮走 danger） |

> **本次实施禁止**把该类名用于「先自建确认弹层再调用 hook」——hook 内部仍会 `window.confirm`，会造成双重确认。该类名仅作为后续放开边界时的目标样式，本次只定义样式、不接入。

### 3.11 进度条（保留，仅 token 化）

`.progress`、`.progress-head`、`.progress-text`、`.progress-track`、`.progress-fill`、`.progress-fill.active`、`.progress-foot`、`@keyframes progress-flow`。

### 3.12 布局 / 工具类（保留，仅 token 化）

`.dashboard-root`、`.dashboard-header`、`.dashboard-grid`、`.subtitle`、`.action-row`、`.preset-row`、`.answer-box`、`.model-path`、`.model-ready`、`.screenshot-row`。

### 3.13 复盘专用（保留 + 新增图例）

| 类名 | 状态 |
| --- | --- |
| `.review-layout` / `.review-list` / `.review-detail` / `.review-body` / `.review-subtitle` / `.review-transcript` / `.review-answers` / `.review-answer` | 保留（token 化） |
| `.session-items` / `.session-item` / `.session-item.active` / `.session-title` / `.session-meta` | 保留（token 化） |
| `.line` / `.line .who` / `.line .what` / `.line.interviewer` / `.line.me` / `.line.echo` | **保留（与浮层共用，见 §7）** |
| `.echo-tag` | 保留 |
| `.legend` | **新增**：图例行 |
| `.legend-item` | **新增**：图例项（色块 + 文案） |
| `.legend-swatch` | **新增**：12px 圆/方块色标 |

### 3.14 类名总表（实施对照）

```
卡片：   .card .card-head .card-body .save-card
按钮：   button .primary .secondary .ghost .danger .mini .small-danger .m5 .mini-import
输入：   .form-input .form-textarea .input-error .field .field-label .field-head
         .inline-row .slider-row .slider .slider-value .char-count(.is-over)
开关：   .switch .switch-thumb .switch-on                       （预留）
徽章：   .badge .badge-success .badge-warning .badge-danger .badge-info .badge-neutral
         .tag-ok .tag-m5 .hint-badge .session-state .state-listening .state-connecting .state-error
Tab：    .tab-bar .tab .tab.active .tabpanel
反馈：   .dashboard-banner .banner-info .banner-warning .banner-error
         .status .status-info .status-warning .error-text .muted .small .visually-hidden
空/骨架： .empty-state .empty-state-icon .empty-state-text .skeleton .skeleton-text
确认：   .confirm-overlay .confirm-panel .confirm-danger .confirm-title .confirm-message .confirm-actions
进度：   .progress .progress-head .progress-text .progress-track .progress-fill .progress-foot
布局：   .dashboard-root .dashboard-header .dashboard-grid .subtitle .action-row .preset-row
         .answer-box .model-path .model-ready .screenshot-row
复盘：   .review-layout .review-list .review-detail .review-body .review-subtitle
         .review-transcript .review-answers .review-answer .session-items .session-item
         .session-title .session-meta .legend .legend-item .legend-swatch
共享：   .line .who .what .echo-tag                               （见 §7）
深化轮： .segment-bar .segment(.active) .preset(.active) .input-wrap .input-toggle
         .test-result(.is-ok/.is-error) .test-result-head .test-result-meta
         .field-group .field-group-title .loading-dot             （见 §3.15）
```

### 3.15 深度优化轮新增类名（模型接口卡片深化）

> 本轮聚焦「模型接口」卡片深度优化；下列类名为本轮新增，全部走既有 token，不引入硬编码。
> 行为约束不变：协议分段控件替代原生 `select` 时，值（`Provider`）与 `onPatch` 语义完全一致；
> API Key 显隐只切换输入框 `type`，Key 内容不进任何日志 / DOM 属性；测试结果卡只在 tsx 层派生耗时与成败。

| 类名 | 状态 | 说明 / 关键样式 |
| --- | --- | --- |
| `.field-group` | **新增** | 卡内信息分组容器：`flex column` + `gap:var(--space-3)`；相邻分组间 `--separator` 分隔线 + `padding-top` |
| `.field-group-title` | **新增** | 分组小标题：`--font-sm` / 600 / `--text-muted` / `letter-spacing:0.5px` |
| `.segment-bar` | **新增** | 分段切换容器：`flex` 等分、`gap:var(--space-1)`；配 `role="group"` + `aria-labelledby` |
| `.segment` | **新增** | 分段按钮：`flex:1`、`--control-h` 高、`--radius-control`、`--surface-input` 底、`--font-sm`；hover 不浮起 |
| `.segment.active` | **新增** | 选中态：`--accent-soft` 底 + `--accent-border` 描边 + `--accent` 字 + 600 字重；配 `aria-pressed` |
| `.preset` | **新增** | 预设芯片按钮（`.preset-row` 内）：`--control-h-sm`、`--radius-pill`、`--font-sm`；替代原 `.ghost` 用法 |
| `.preset.active` | **新增** | 表单与某预设完全一致时的高亮：`--accent-soft` / `--accent-border` / `--accent` |
| `.input-wrap` | **新增** | 输入框 + 右侧内嵌按钮包装：`position:relative`；内部 `.form-input` 右侧加 padding 避让按钮 |
| `.input-toggle` | **新增** | 内嵌切换按钮（API Key 显隐眼睛）：绝对定位右侧、`--control-h-icon` 方寸、透明底、hover `--bg-hover` |
| `.test-result` | **新增** | 测试连接结果卡：`--radius-control`、`--bg-hover` 底；`role="status"` + `aria-live` |
| `.test-result.is-ok` | **新增** | 成功语义：`--success-soft` 底 + `--success-border` |
| `.test-result.is-error` | **新增** | 失败语义：`--danger-soft` 底 + `--danger-border` |
| `.test-result-head` | **新增** | 结果卡头部：语义徽章 + 耗时/模型摘要，左右分布、可换行 |
| `.test-result-meta` | **新增** | 摘要文字：`--font-sm`、`--text-muted`、`tabular-nums` |
| `.loading-dot` | **新增** | 按钮内行内加载圆点：6px、`currentColor`、复用 `pulse` 动画 |

> 说明：`.answer-box` 在结果卡内单独铺白底（`.test-result .answer-box`），让语义色只做外框，
> 返回正文仍保持高对比可读；`button.ghost.m5` 本轮改为虚线胶囊，弱化「M5 预留」入口。

---

## 4. 逐卡片改造清单

> 每项固定四段：**现状问题 → 改造动作 → 所用类名 → 保留/不变**。

### 4.1 Dashboard 布局（`src/Dashboard.tsx`）

**现状问题**
- `.tab-bar` 有 `role="tablist"`、`.tab` 有 `role="tab"`，但缺少 `tabpanel` / `aria-controls` 关系，读屏切换不友好。
- banner 无 `aria-live`，自动消失的提示读屏读不到。
- 加载态仅一行 `.muted`「正在读取配置…」，无骨架屏。
- 页头 `h1` 26px / letter-spacing 2px，与 AIGX「页头 20px 标题 + 13px 副题」不符。
- 空白项目均无焦点环（全局问题，见 §5）。

**改造动作**
1. 页头走 token：`h1` → `--font-xl`、`letter-spacing:1px`；`.subtitle` → `--font-sm`、`--text-muted`；`.hint-badge` 圆角 `--radius-pill`。
2. Tab：每个 `.tab` 加 `id`（如 `tab-settings` / `tab-review`）+ `aria-controls`（如 `tabpanel-settings` / `tabpanel-review`）；内容区包一层 `<div className="tabpanel" role="tabpanel" id="tabpanel-settings" aria-labelledby="tab-settings">`（复盘面板同理）。`.tab.active` 改为白底 + 投影。
3. Banner：加 `role="status"`；`aria-live={banner.level === "error" ? "assertive" : "polite"}`；保留点击关闭与自动消失。
4. 加载态：把 `<p className="muted">正在读取配置…</p>` 替换为骨架块（`.dashboard-grid` 内放若干 `.skeleton`），并加 `aria-busy="true"` 与 `.visually-hidden` 文本「正在读取配置」。
5. 栅格 `gap` → `var(--space-3)`。

**所用类名**：`.dashboard-root`、`.dashboard-header`、`.subtitle`、`.hint-badge`、`.dashboard-banner`、`.banner-info`、`.banner-error`、`.tab-bar`、`.tab`、`.tab.active`、`.tabpanel`、`.dashboard-grid`、`.skeleton`、`.visually-hidden`、`.muted`。

**保留 / 不变**
- 保留：默认 Tab=`settings`、切换逻辑、`aria-selected` 逻辑、banner 自动消失时长（error 10s / info 6s）与点击关闭、快捷键徽标文案「Ctrl+Shift+Space 显示浮层」。
- 不变：所有 hook 调用与数据流、`hint://notice` 订阅逻辑、`open_log_dir` 行为。

### 4.2 ModelCard（`src/settings/ModelCard.tsx`）

**现状问题**
- `.status` 双用途：`useSettingsForm` 的 `save/loadModels/testConnection` 把成功与失败都写进同一个 `status` 字符串，渲染时统一蓝色，导致「连接失败/保存失败/拉取失败」呈蓝色，与 `.error-text` 红色体系冲突（根因在 hook 的单一 status，本次不改 hook）。
- 测试结果裸 `<pre className="answer-box">`，无 label/aria。
- 空表单不拦截（属 hook 行为，本次不新增校验）。
- 辅助文案密集。
- 按钮无焦点环。

**改造动作**
1. **状态语义拆分（tsx 层派生，不动 hook）**：新增 `src/settings/statusTone.tsx`，导出：
   ```tsx
   export type StatusTone = "success" | "warning" | "error" | "info";
   const WARNING_MARKERS = ["但", "未完成", "没有返回可用模型"];
   const SUCCESS_MARKERS = ["已保存", "已获取", "连接成功", "已导入", "已删除", "已清空",
                            "已导出", "检查完成", "已启动", "已结束"];
   export function deriveStatusTone(text: string): StatusTone {
     const t = text.trim();
     if (!t) return "info";
     if (WARNING_MARKERS.some((m) => t.includes(m))) return "warning";
     if (/失败[:：]?|错误[:：]?/.test(t)) return "error";
     if (SUCCESS_MARKERS.some((m) => t.includes(m))) return "success";
     return "info";
   }
   ```
   ModelCard / 保存卡片 / ProfileCard / ModelDownloadCard / SessionReview 统一引用它，按 tone 渲染 `.error-text` / `.status-warning` / `.status` / `.status-info`。**hook 一行不改。**
2. 测试结果：结果卡（`.test-result` + 语义徽章 + 耗时/模型摘要）+ 内嵌 `<pre aria-label="测试连接返回内容" className="answer-box">`，并保留 `.visually-hidden` 说明；`.answer-box` 走 token（`--radius-inline`、内嵌底、`--font-sm`）。
3. 辅助文案统一 `.muted.small`；卡片结构改 `.card` + `.card-head`（标题「模型接口」）+ `.card-body`。
4. 预设按钮 `.preset`（当前预设高亮 `.active`）；「刷新模型」`.secondary`（加载态带 `.loading-dot`）；「测试连接」默认按钮（基础样式）；输入控件 `.form-input`；协议选择改 `.segment-bar` 分段控件；API Key 加 `.input-wrap` + `.input-toggle` 显隐；卡内用 `.field-group` 分「常用预设 / 接口凭据 / 预留」三组。
5. 按钮焦点环由 §5 全局规则提供。

**所用类名**：`.card`、`.card-head`、`.card-body`、`.field-group`、`.field-group-title`、`.preset-row`、`.preset`、`.segment-bar`、`.segment`、`.form-input`、`.input-wrap`、`.input-toggle`、`.inline-row`、`.action-row`、`.test-result`、`.test-result-head`、`.test-result-meta`、`.answer-box`、`.badge`、`.badge-success`、`.badge-danger`、`.badge-neutral`、`.loading-dot`、`.status`、`.status-info`、`.status-warning`、`.error-text`、`.muted`、`.small`、`.visually-hidden`、`.screenshot-row`、`.m5`、`.tag-m5`。

**保留 / 不变**
- 保留：PRESETS 点击 `patch`、Base URL `onBlur` 自动识别协议并规范化、刷新模型、测试连接、截图问答提示展开与全部文案语义、`keySaved` 占位文案。
- 不变：「测试连接成功自动保存 Key」时机、`busy` 禁用逻辑、空表单不拦截的现有行为（**不新增校验**）。

### 4.3 SessionCard（`src/settings/SessionCard.tsx`）

**现状问题**
- 错误用 `<br/>` 硬换行拼提示，排版脆弱。
- error 与 notice 分色但无明确层次；notice 走蓝色 `.status`。
- 按钮无焦点环。

**改造动作**
1. 移除 `<br/>`，拆为两个独立 `<p>`：`.error-text`（错误正文）+ `.muted.small`（引导句「可检查上方 API Key / 语音模型状态后重新开始。」）。
2. 语义分色：`error` → `.error-text`；`notice` → `.status-info`；`message` / 状态补充 → `.muted.small`。
3. 状态徽标保留 `.session-state.state-*` 映射，仅 token 化。
4. 按钮：「开始面试」`.primary`、「结束面试」`.secondary`。

**所用类名**：`.card`、`.card-head`、`.card-body`、`.session-state`、`.state-listening`、`.state-connecting`、`.state-error`、`.muted`、`.small`、`.action-row`、`.primary`、`.secondary`、`.error-text`、`.status-info`。

**保留 / 不变**
- 保留：`STATE_LABEL` 文案、`startLabel` 三分支文案、状态徽标配色映射、会话编号展示、connecting 提示文案。
- 不变：`useSessionControl` 的 start/stop、禁用条件（`busy || running` / `busy || !running`）。

### 4.4 ModelDownloadCard（`src/settings/ModelDownloadCard.tsx`）

**现状问题**
- 进度条是全页最佳状态组件（保留）。
- 未下载态 `toFixed(0)`、下载态 `toFixed(1)`，精度不统一。
- 删除模型用原生 `window.confirm`，破坏主题、不可定制。
- `notice` 与 `view.error` 同色。

**改造动作**
1. 精度统一：`UnpreparedBody` 的 `percent.toFixed(0)` → `percent.toFixed(1)`；`ProgressBody` 保持 `toFixed(1)`。
2. 语义分色：`view.error` → `.error-text`；`notice` 用 `deriveStatusTone` 渲染（`.status` / `.status-info` / `.status-warning` / `.error-text`）。
3. `window.confirm`：**本次不替换**。确认逻辑在 `useModelManager.ts`（不可改的 `.ts` hook）内部，组件层无法在不改 hook 的前提下替换，否则会双重确认。保留原生确认；`.confirm-*` 类名已定义，待放开 `.ts` 边界后再迁移（见 §6.3）。
4. 卡片结构 `.card-head`（标题「语音模型」+ `tag-ok`）+ `.card-body`；按钮「下载模型/检查模型文件」`.primary`、「取消下载」`.secondary`、「删除模型」`.danger`；进度条 token 化。

**所用类名**：`.card`、`.card-head`、`.card-body`、`.tag-ok`、`.muted`、`.small`、`.model-ready`、`.model-path`、`.progress`、`.progress-head`、`.progress-text`、`.progress-track`、`.progress-fill`、`.progress-fill.active`、`.progress-foot`、`.action-row`、`.primary`、`.secondary`、`.danger`、`.status`、`.status-info`、`.status-warning`、`.error-text`。

**保留 / 不变**
- 保留：下载 / 取消 / 删除流程、断点续传说明、`ReadyBody/UnpreparedBody/ProgressBody` 分支、`title` 提示、`stepLabel`/`sourceLabel` 文案。
- 不变：`useModelManager` 全部语义（含内部 `window.confirm` 文案：「确定删除语音模型吗？…约 241MB…」）。

### 4.5 ProfileCard（`src/settings/ProfileCard.tsx`）

**现状问题**
- 两个 textarea（9/7 行）无字数计数，6000 上限只写在 placeholder。
- 导入按钮嵌在 `<label>` 内，读屏把「从文件导入」读成字段名。
- 导入成功/失败同用蓝色 `.status`。

**改造动作**
1. 结构改为可访问写法（按钮移出 label）：
   ```tsx
   <div className="field">
     <div className="field-head">
       <label className="field-label" htmlFor="profile-resume">简历</label>
       <button type="button" className="ghost mini-import" ...>从文件导入</button>
     </div>
     <textarea id="profile-resume" className="form-input form-textarea" rows={9} ... />
     <p className="char-count" aria-live="polite">{form.resume.length} / 6000</p>
   </div>
   ```
   `projects` 同理（`id="profile-projects"`）。字数超 6000 时加 `.is-over`（红）。
2. 语义分色：本地 `status` 用 `deriveStatusTone`，成功 → `.status`，失败（「导入失败：」）→ `.error-text`。
3. textarea 加 `.form-input.form-textarea`；卡片结构 `.card-head`（「资料注入」）+ `.card-body`。

**所用类名**：`.card`、`.card-head`、`.card-body`、`.field`、`.field-head`、`.field-label`、`.form-input`、`.form-textarea`、`.char-count`、`.is-over`、`.ghost`、`.mini-import`、`.status`、`.error-text`、`.muted`、`.small`。

**保留 / 不变**
- 保留：`IMPORT_LIMIT = 6000`、导入对话框过滤器、文件文本导入、超限截断与提示文案、`patch({resume/projects})`。
- 不变：`import_document_text` 调用与错误处理；手动输入超 6000 **不强制截断**（截断仍只发生在导入路径），字数为纯展示。

### 4.6 BehaviorCard（`src/settings/BehaviorCard.tsx`）

**现状问题**
- 滑块 + 数值显示已规范（保留）。
- 辅助文案冗长、留白不足，偏挤（风格本身统一）。

**改造动作**
1. 输入/下拉加 `.form-input`；滑块加 `.slider`，包在 `.inline-row.slider-row` 内，数值 `.slider-value`。
2. 文案保留全部信息，统一 `.muted.small`；卡体 `.card-body` 的 `gap:var(--space-3)` 提供留白，辅助段落之间用 `--space-2` 收紧。
3. 卡片结构 `.card-head`（「回答行为」）+ `.card-body`。

**所用类名**：`.card`、`.card-head`、`.card-body`、`.form-input`、`.inline-row`、`.slider-row`、`.slider`、`.slider-value`、`.muted`、`.small`。

**保留 / 不变**
- 保留：选项值与文案（口语短答/要点提纲、自动/手动）、自动/手动模式说明、热键全量说明、不透明度说明、百分比显示。
- 不变：`normalizeOpacity`、`MIN/MAX_OVERLAY_OPACITY`、`patch` 行为。

### 4.7 DevicesCard（`src/settings/DevicesCard.tsx`）

**现状问题**
- `.card-head` 内嵌「刷新设备」按钮与标题混排。
- 加载/空/错误三态纯文字，无进度/重试引导层次。

**改造动作**
1. 保留 `.card-head`（标题「设备」+「刷新设备」`.ghost` 左右分布，符合 AIGX 卡头）。
2. 三态分层：
   - 加载 → `.status-info` 文案「正在检测音频设备…」或 `.skeleton` 行；
   - 空 → `.empty-state`（`.empty-state-icon` + `.empty-state-text`，文案沿用原文）；
   - 错误 → `.error-text`（保留「请确认麦克风未被其它程序独占后点『刷新设备』。」引导）；
   - 正常 → `.muted.small` 提示。
3. 两个下拉 `.form-input`；卡片结构 `.card-head` + `.card-body`。

**所用类名**：`.card`、`.card-head`、`.card-body`、`.ghost`、`.form-input`、`.status-info`、`.error-text`、`.empty-state`、`.empty-state-icon`、`.empty-state-text`、`.skeleton`、`.muted`、`.small`。

**保留 / 不变**
- 保留：自动首次 `refresh`、`attempted` 去重、缺失设备保留项（`已保存设备（当前未检测到）`）、「系统默认」、双通道电平说明。
- 不变：`loadDevices` 调用、`loading/error` 状态、select 的 `patch`。

### 4.8 保存卡片（`src/Dashboard.tsx` 内联）

**现状问题**
- 单卡全宽，结构与其他卡片不统一；成功/失败同走蓝色 `.status`。

**改造动作**
1. 结构改为 `.card.save-card` + `.card-head`（「保存」）+ `.card-body`。
2. 说明文字 `.muted.small`，保留 `logs\hint.log` 引用。
3. 按钮「保存全部配置」`.primary`、「打开日志目录」`.secondary`。
4. `form.status` 用 `deriveStatusTone` 渲染语义色（成功绿/警告黄/错误红）。

**所用类名**：`.card`、`.save-card`、`.card-head`、`.card-body`、`.muted`、`.small`、`.action-row`、`.primary`、`.secondary`、`.status`、`.status-warning`、`.error-text`。

**保留 / 不变**
- 保留：保存流程、按钮文案（`保存中…` / `保存全部配置`）、`busy` 禁用、打开日志目录与失败 banner、全部说明文案。
- 不变：`form.save()` / `open_log_dir` 语义。

### 4.9 SessionReview（`src/sessions/SessionReview.tsx`）

**现状问题**
- 双栏布局合理、`.session-item.active` 清晰（保留）。
- 删除 / 清空用 `window.confirm`（在 `useSessions.ts`，本次不改）。
- 转录固定色区分「面试官/我」，无图例。
- 加载/空/错误为散落灰字。

**改造动作**
1. 删除/清空：**保留原生 `window.confirm`**（hook 内，边界外）；按钮样式 `.danger` / `.small-danger` 并享全局焦点环。`.confirm-*` 为后续迁移目标（§6.3）。
2. 三态组件化：加载 → `.skeleton`；空 → `.empty-state`（文案沿用原文）；错误 → `.error-text`。
3. 转录区加图例：
   ```tsx
   <div className="legend">
     <span className="legend-item"><i className="legend-swatch swatch-interviewer" />面试官</span>
     <span className="legend-item"><i className="legend-swatch swatch-me" />我</span>
     <span className="legend-item"><i className="legend-swatch swatch-echo" />回声</span>
     <span className="legend-item"><i className="legend-swatch swatch-recovered" />补录</span>
   </div>
   ```
   图例色标与 `.line.interviewer/.me/.echo`、`.echo-tag`、`.recovered-tag` 使用同一组 token，保证颜色一致（浮层同步，见 §7）。
4. `.session-item` 走 token（`--radius-inline`、hover/active 保留）；`.review-*` token 化。
5. `notice` 用 `deriveStatusTone`；`detailError` / `error` → `.error-text`。

**所用类名**：`.review-layout`、`.review-list`、`.review-detail`、`.review-body`、`.review-subtitle`、`.review-transcript`、`.review-answers`、`.review-answer`、`.session-items`、`.session-item`、`.session-item.active`、`.session-title`、`.session-meta`、`.line`、`.who`、`.what`、`.echo-tag`、`.legend`、`.legend-item`、`.legend-swatch`、`.empty-state`、`.empty-state-icon`、`.empty-state-text`、`.skeleton`、`.status`、`.status-warning`、`.error-text`、`.muted`、`.small`、`.ghost`、`.danger`、`.small-danger`。

**保留 / 不变**
- 保留：刷新 / 打开 / 导出 / 删除 / 清空按钮逻辑与文案、双栏布局、`.session-item.active`、时长计算、转录/答案渲染结构、回声/补录标记。
- 不变：`useSessions` 全部语义（含内部两次 `window.confirm` 文案），导出 Markdown 行为。

---

## 5. 全局可访问性与焦点环规范

1. **按钮焦点环（全局，修复所有 button 无 `:focus-visible` 的问题）**：
   ```css
   button:focus-visible,
   [role="tab"]:focus-visible,
   .session-item:focus-visible,
   .switch:focus-visible {
     outline: var(--focus-ring-width) solid var(--focus-ring-strong);
     outline-offset: var(--focus-ring-offset);
   }
   ```
   覆盖 `.primary` / `.ghost` / `.danger` / `.mini` / `.tab` / `.session-item` 全部按钮形态。
2. 输入类保留并统一为 `focus` 光晕（`--focus-ring`）；按钮用 `focus-visible` 外环（不抢鼠标点击）。
3. 动态反馈：
   - Dashboard banner：`role="status"` + `aria-live`（错误 assertive，其余 polite）。
   - 字数计数 `.char-count`：`aria-live="polite"`。
   - 测试结果区：`aria-label`。
4. 最小组件字号不低于 `--font-sm`（12px）用于可读文案；`.tag-m5` 等纯装饰标记允许 `--font-xs`。
5. 转录图例与状态徽章必须颜色 + 文字双编码，不单靠颜色。

---

## 6. 边界与约束

### 6.1 允许改动范围
- `src/App.css`（唯一全局样式文件）。
- `src/**/*.tsx`：`Dashboard.tsx`、`settings/*.tsx`、`sessions/SessionReview.tsx`。
- 允许新增 `src/settings/statusTone.tsx`（纯 tsx，导出语义派生函数；无 JSX 亦可）。

### 6.2 明确禁止
- 不改 `crates/**`、`src-tauri/**` 任何 Rust 代码或 Tauri 配置。
- 不改 `src/**/*.ts` hook 的既有语义（`useSettingsForm` / `useModelManager` / `useSessionControl` / `useSessions` / `useOverlayEvents` / `lib/*`）。
- 不改文案**含义**（可调整排版与换行方式，不得删改信息内容）。
- 不改快捷键、状态机、请求时机、保存时机。
- 不新增表单校验（如「空表单拦截」）——属于功能行为变更。
- 不在组件规则中硬编码颜色/字号/圆角/间距。

### 6.3 关于 `window.confirm`（重要，避免歧义）
- 两处确认逻辑位于 `.ts` hook：`useModelManager.remove()`、`useSessions.remove()/removeAll()`。
- 在「只改 .tsx」的边界内，**无法**替换为自定义弹层而不产生双重确认。
- 本方案处置：**本次保留原生 `window.confirm`**，`.confirm-*` 类名与样式仅定义、不接入。若后续要迁移，需团队明确放开 `.ts` 边界（把确认意图从 hook 提升为可注入回调），届时再按 `.confirm-*` 接入。

### 6.4 状态语义拆分约束
- `useSettingsForm` 的单一 `status` 字符串不改；语义颜色在 `.tsx` 层用 `deriveStatusTone` 派生。
- 派生函数只做展示映射，不参与任何业务判断。

---

## 7. 兼容性：浮层与其它页面共享类名

`App.css` 同时服务配置页、复盘页、浮层（`src/Overlay.tsx`、`src/overlay/*`、`src/CaptureOverlay.tsx`）。以下类名**跨页面共享**，调整时必须同步验证，不能只改配置页：

| 共享类名 | 使用方 | 调整注意 |
| --- | --- | --- |
| `button` 基础、`.mini`、`.mini.ghost`、`.ghost` | 浮层 `AnswerPanel` / `Overlay` 已有按钮 | 按钮高度/圆角/焦点环变化会波及浮层；`.mini` 必须维持 26px 紧凑，浮层 320×180、320×240 档不得溢出 |
| `.line`、`.line .who/.what`、`.line.interviewer/.me/.echo`、`.echo-tag` | 浮层 `TranscriptPanel` + 复盘 `SessionReview` | 若调整转录配色/图例，两处必须用同一组 token，保持颜色一致；`.line.echo` 的透明度语义不变 |
| `.answer-box` | 配置页测试结果 + 复盘答案 | token 化后两处同步 |
| `.status`、`.error-text`、`.muted`、`.small`、`.placeholder` | 浮层/复盘/配置页可能共用 | 改字号/颜色需全局回归 |
| `.card`、`.review-*`、`.session-*` | 配置页 + 复盘页 | 浮层不使用，可安全调整 |
| `.progress*` | 仅 ModelDownloadCard | 可安全调整 |

**浮层专属类（本次不改、配置页不得复用）**：
`overlay-root`、`overlay-header`、`overlay-brand`、`overlay-status`、`dot`、`tone-ok/pending/error/idle`、`header-latency`、`overlay-transcript`、`overlay-empty`、`overlay-answer`、`overlay-answer-toolbar`、`overlay-question-row`、`answer-current`、`answer-history-timeline`、`history-caption`、`answer-block.past`、`block-question`、`block-answer`、`overlay-question`、`pending-badge`、`answer-title`、`answer-count`、`streaming-dot`、`answer-actions`、`answer-text`、`answer-error`、`caret`、`overlay-footer`、`footer-group`、`speaking-badge`、`footer-item`、`level`、`level-label`、`level-track`、`level-fill`、`level-mic`、`level-system`、`overlay-toast`、`toast-info`、`toast-error`、`answer-error-bar`、`answer-error-text`、`answer-error-actions`、`recovered-tag`、`capture-root`、`capture-rect`、`capture-size`、`capture-hint`、`capture-error`。

**浮层相关约束**：
- `--overlay-alpha` 继续以 `var(--overlay-alpha, 0.88)` 引用，不得在 `:root` 写死覆盖。
- 浮层的 `max-height` 媒体查询（240px / 200px）与 `max-width: 380px` 规则保持不变。
- 浮层背景不使用 `backdrop-filter`（现注释说明与 WebView2 合成有关），不得新增。

---

## 8. 实施顺序建议

1. **基础层（task 4，ui-base）**：`App.css` 顶部 token 全量落地 → 基础控件类（卡片/按钮/输入/徽章/Tab/反馈/空态/骨架/确认）= Dashboard 布局改造。
2. **卡片层（task 5，ui-cards）**：按 §4.2–4.8 逐卡改造，统一 `.card-head/.card-body` 结构与语义色。
3. **复盘层（task 6，ui-review）**：按 §4.9 改造 + 图例，与浮层转录配色同源。
4. **验证（task 7，ui-verify）**：`tsc`/`vite build` 通过；配置页、复盘页、浮层三处回归；焦点环、aria、对比度逐项检查；确认无功能行为变化。

**验收硬指标**
- 全页无新增硬编码色值/字号/圆角（抽查 `App.css` 组件段）。
- 所有按钮键盘 Tab 可达且可见焦点环。
- Tab 有 `role="tabpanel"` + `aria-controls`。
- 成功/警告/错误/信息四态颜色与文案一致。
- 浮层在 320×180 / 320×240 / 常规尺寸下无布局回归。
- `git diff` 仅涉及 `src/**/*.tsx` 与 `src/App.css`；Rust/hook 零改动。
