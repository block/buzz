# Business Dock

本文保留 V1 容器、配置与 CSP 基线说明。Buzz 消息 / Agent 业务资源联动、BusinessResource、宿主历史、Dirty 保护和 Bridge V2 的当前协议见 [V2.md](./V2.md)。

Business Dock 是帕乔利 AI 在 Buzz 桌面端最外层增加的受控业务系统容器。Buzz 继续负责频道、消息、Thread、DM、Agent、Workflow、Search 和协作；销售、采购、库存、往来、资金、发票、核算、税务与报表仍由独立业务系统负责。

普通桌面布局中，Business Dock 可拖动调整宽度，最大占当前窗口的 50%；窄屏使用覆盖式布局，全屏模式仍可占满窗口。

## 架构

`BusinessDockProvider` 位于主题 Provider 内，管理 Dock 状态、iframe 引用、会话历史、主题同步和快捷键。`BusinessDock` 是 `AppShell` 内容区域最右侧的兄弟节点，不属于 `ChannelPane`，因此不会改变频道或 Nostr 模型。

```text
ThemeProvider
└── BusinessDockProvider
    └── AppShell
        ├── AppSidebar + AppShellChannelSurface
        │   └── Channel / Thread / Agent auxiliary panels
        └── BusinessDock
            ├── BusinessDockToolbar
            └── BusinessDockBrowser (iframe)
```

Dock 关闭时使用 0 宽度和 `visibility: hidden`，iframe 仍保持 mounted，以保留登录、表单、筛选器及业务页面连接。全屏仅改变同一个 Dock 的布局，不重新创建 iframe。

## 配置

Vite 构建环境需要同时提供：

```dotenv
VITE_BUSINESS_APP_ORIGIN=https://biz.example.com
VITE_BUSINESS_APP_URL=https://biz.example.com/embed/
```

`VITE_BUSINESS_APP_ORIGIN` 必须是没有路径、凭据、查询参数或 fragment 的 HTTP(S) Origin。`VITE_BUSINESS_APP_URL` 可以是绝对 URL，也可以是相对该 Origin 的路径，但最终 URL 必须属于同一 Origin。

缺少配置或配置非法时 Dock 仍可打开，只显示明确的未配置信息，不创建远程 iframe。

## iframe Origin 安全模型

- 宿主只解析 HTTP(S) URL，并统一通过 `isAllowedBusinessUrl` / `resolveAllowedBusinessUrl` 校验。
- `javascript:`、`data:`、`file:` 和其他 Origin 会被拒绝。
- iframe 没有地址输入框，用户不能把 Dock 当作通用浏览器。
- iframe 使用 sandbox，允许业务页面需要的脚本、表单和同源能力，但不允许顶层导航。
- `postMessage` 的发送目标始终是配置的精确 Origin，不使用 `*`。
- 接收消息同时校验 `event.origin`、`event.source`、协议版本、类型和 payload。
- 第一版不向 iframe 传 Access Token、Cookie、银行数据、发票、会计凭证、密码或 Secret。

跨 Origin iframe 的最终重定向地址无法由宿主 JavaScript 读取；打包应用由 CSP 的精确 `frame-src` 在 WebView 层拒绝越界重定向。业务页面仍应自行限制外链和登录回跳。

## CSP 配置

`desktop/src-tauri/tauri.conf.json` 的默认策略是：

```text
frame-src 'self'
```

`pnpm tauri dev` / `pnpm tauri build` 会经过 `desktop/scripts/tauri-business-dock.mjs`，读取并严格校验 `VITE_BUSINESS_APP_ORIGIN`，然后通过 Tauri `--config` 合并为：

```text
frame-src 'self' https://biz.example.com
```

不要使用 `pnpm exec tauri` 绕过该包装器；直接 Cargo 构建会保持 self-only 并安全失败。不要把 `frame-src` 或 `default-src` 放宽为 `*`、`http:` 或 `https:`。

业务系统响应还必须允许 Buzz 的实际应用 Origin 嵌入，例如配置适当的：

```text
Content-Security-Policy: frame-ancestors <buzz-origin>
```

具体 Buzz Origin 随开发、签名应用及部署方式而不同，应由部署环境显式维护。

## Business Bridge V1

统一消息封装：

```ts
type BusinessBridgeMessage = {
  version: 1;
  type: string;
  requestId?: string;
  payload?: unknown;
};
```

宿主发送：

- `HOST_INIT`
- `SET_THEME`，payload 为 `{ theme: "light" | "dark" }`
- `REFRESH`
- `NAVIGATE`，仅用于 Bridge 管理的 Back / Forward / Home

业务系统发送：

- `BUSINESS_READY`
- `TITLE_CHANGED`，payload 为 `{ title: string }`
- `ROUTE_CHANGED`，payload 为 `{ url: string }`

Back / Forward 只有在业务系统发送 `BUSINESS_READY` 并通过 `ROUTE_CHANGED` 建立受控历史后才启用。没有实现 Bridge 的业务系统仍可加载、刷新、全屏、调整宽度和外部打开。

## 业务系统最小要求

1. 提供可嵌入 URL，例如 `/embed/`。
2. 使用 `frame-ancestors` 允许实际 Buzz Origin。
3. 页面适配 420px 以上宽度。
4. 最好在 embed 模式隐藏业务系统自己的主导航。
5. 可选实现 Business Bridge V1，以支持标题、主题和受控历史。

## 开发 Mock

E2E 模式使用同源 `/business-dock-test.html`。页面展示当前 URL、主题、Bridge 状态与刷新次数，并能发出 `BUSINESS_READY`、`TITLE_CHANGED`、`ROUTE_CHANGED`。它只用于开发和自动化测试，不是正式业务页面。

```bash
cd desktop
pnpm test:e2e:smoke -- business-dock.spec.ts
```

## 已知限制

- V1 不实现 SSO 或 token 传递；业务系统自行建立登录会话。
- 没有 Bridge 时无法可靠读取跨 Origin iframe 的内部 history，因此 Back / Forward 保持 disabled。
- Pin 在 overlay 模式下控制点击遮罩是否关闭 Dock，不改变业务数据生命周期。
- 浏览器预览环境不执行 Tauri CSP；打包 CSP 由 Rust 集成测试和构建包装器单测保护。

## 后续路线

- 与业务系统联合定义登录和 Logout 生命周期后，再设计最小 SSO。
- 在不传敏感业务数据的前提下扩展可审计的业务链接协议。
- 只有 iframe 明确无法满足平台能力时，才评估 Tauri Child Webview。
- 根据真实双面板使用数据继续校准 responsive breakpoint 和资源回收策略。
