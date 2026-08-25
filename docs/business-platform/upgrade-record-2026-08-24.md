# Buzz 升级验证记录：2026-08-24

## 基线

- 原业务台基线：`fe7c6808e7430d185498178e07e58e378d2e4c7d`
- Buzz 目标：`0720f5380ce8a6c050afac159f8462c06cd51ab5`
- 最新桌面发布：`desktop-v0.5.18`
- 升级候选分支：`codex/business-platform-buzz-latest`

## 迁移范围和冲突

完整迁移覆盖 40 个相对 Buzz 基线修改的跟踪文件，以及 451 个业务台新增文件。普通补丁无法直接应用；三方应用后只有以下 5 个文件需要人工合并：

1. `crates/buzz-acp/src/lib.rs`
2. `desktop/src-tauri/src/lib.rs`
3. `desktop/src/app/AppShell.tsx`
4. `desktop/src/shared/hooks/useThreadPanelWidth.ts`
5. `pnpm-lock.yaml`

解决原则是保留 Buzz 新增的通用能力，并重新接入产品扩展：保留 ACP `cwd`、Tauri `observed_unread`/`persona_catalog`、项目面板 resize 状态和上游依赖版本；业务台继续只通过 ACP、Tauri 与 Desktop 的通用扩展组装点接线。

## 已通过验证

在目标 Buzz commit 的完整候选上执行并通过：

```bash
cargo check -p buzz-acp -p business-auth-gateway -p business-read-api \
  -p business-read-mcp -p business-action-service -p business-core
pnpm install --frozen-lockfile
pnpm --dir desktop typecheck
pnpm --dir desktop build:e2e
just desktop-tauri-check
pnpm --dir apps/business-web test
pnpm --dir apps/business-web build
pnpm --dir desktop test
pnpm --dir desktop test:e2e:smoke --grep 'Business Dock'
```

验证结果：

- Rust 业务服务与 ACP：通过
- Desktop TypeScript 与 E2E 构建：通过
- Tauri：通过
- Business Web：18/18 测试通过，构建通过
- Desktop：5475/5475 测试通过
- Business Dock：11/11 E2E 通过

这证明当前业务台能够完整迁移到该 Buzz 版本；它不是对未来任意版本“零冲突”的承诺。每次升级仍必须重新运行全平台兼容预演和真实工作流验收。
