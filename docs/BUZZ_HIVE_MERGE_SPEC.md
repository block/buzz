# File Spec: **Buzz Hive** — Hợp nhất Buzz + claude-code-cli-ui + Sim

> Trạng thái: Draft v0.1 · Ngày: 2026-08-17
> Nhân vật chính (core platform): **`block/buzz`**
> Sáp nhập vào: **`Ngxba/claude-code-cli-ui`** (Agent Studio) + **`simstudioai/sim`** (Flow/Knowledge Studio)

---

## 1. Tóm tắt & Nguyên tắc thiết kế

Ý tưởng: **Buzz** đã là một "hive mind communication platform" — mọi hành động (chat, reaction, workflow step, git event) là một **Nostr event đã ký** trong một event log duy nhất, có community/tenant làm ranh giới. Đây chính là "xương sống" phù hợp nhất để làm lõi hệ thống, vì nó vốn được thiết kế cho việc người + AI agent cùng làm việc trong một không gian.

Hai dự án còn lại **không được giữ làm sản phẩm độc lập** — chúng bị "mổ xẻ" và cấy vào Buzz như hai module/crate mới:

| Dự án gốc | Vai trò sau khi hợp nhất | Vị trí trong Buzz |
|---|---|---|
| `block/buzz` | **Lõi (core)**: relay, auth, pubsub, search, audit, workflow engine, desktop/web/mobile client, agent surface (ACP) | Toàn bộ `apps/`, `crates/` gốc — giữ nguyên |
| `simstudioai/sim` | **Buzz Flow Studio**: visual workflow builder, block/tool registry, knowledge base (pgvector), Tables, Files, Chat | Module mới `buzz-flow` (backend) + `desktop/src/features/flow-studio` (frontend) |
| `Ngxba/claude-code-cli-ui` | **Buzz Agent Studio**: orchestration/giám sát agent, quản lý skill/command/agent, dependency graph, GitHub import skill | Module mới `buzz-agent-studio` (backend) + `desktop/src/features/agent-studio` (frontend) |

Nguyên tắc bắt buộc khi merge:

1. **Một event log duy nhất.** Sim và cli-ui hiện có DB/state riêng (Postgres+Drizzle cho Sim, file `.claude/*` cho cli-ui). Sau khi merge, **không app nào được ghi state ngoài Nostr event** — mọi write phải đi qua `buzz-relay` dưới dạng event có `kind` mới, để giữ tính năng "audit trail" và "portable identity" của Buzz.
2. **Buzz community = tenant boundary** cho cả 3 tính năng — Flow Studio và Agent Studio đều bị scope theo `community` (không có global workspace ẩn).
3. **Không tạo LLM provider riêng.** Cả 3 dự án gốc đều là "companion", không tự có model — giữ nguyên triết lý này: Buzz Hive chỉ **điều phối**, dùng key/agent runtime do người dùng cấu hình (Anthropic API key, Claude Agent SDK, Claude Code CLI, Ollama/vLLM như Sim hỗ trợ).
4. **Persona = Agent.** `buzz-persona` (đã có trong Buzz) trở thành điểm hợp nhất khái niệm "Agent" của cli-ui và "Agent block" của Sim — tránh 2 khái niệm Agent song song.

---

## 2. Kiến trúc tổng thể

```
                         ┌───────────────────────────────────────────┐
                         │              buzz-relay (Rust/Axum)        │
                         │  Nostr WS + REST · nguồn sự thật duy nhất  │
                         └───────────────┬─────────────────────────┬──┘
              ┌──────────────────────────┼─────────────────────────┼──────────────────────┐
              │                          │                         │                       │
      ┌───────▼───────┐          ┌───────▼────────┐        ┌───────▼────────┐     ┌────────▼───────┐
      │  buzz-core /   │          │  buzz-db /      │        │  buzz-pubsub /  │     │  buzz-audit /   │
      │  buzz-auth     │          │  buzz-search    │        │  presence,typing│     │  hash-chain log │
      └────────────────┘          └─────────────────┘        └─────────────────┘     └────────────────┘
                         │
      ┌──────────────────┴───────────────────────────────────────────────────────────┐
      │                          Tầng module nghiệp vụ (crate mới)                    │
      │                                                                               │
      │  ┌─────────────────────────────┐        ┌────────────────────────────────┐   │
      │  │  buzz-flow   (từ Sim)        │        │  buzz-agent-studio (từ cli-ui) │   │
      │  │  - workflow visual graph     │        │  - agent/skill/command graph   │   │
      │  │  - block & tool registry     │        │  - GitHub skill import         │   │
      │  │  - knowledge base (pgvector) │        │  - context/token/cost monitor  │   │
      │  │  - Tables, Files, Chat block │        │  - SSE session viewer          │   │
      │  │  → emits kind 462xx / 463xx  │        │  → emits kind 472xx / 473xx    │   │
      │  └───────────────┬─────────────┘        └───────────────┬────────────────┘   │
      │                  │                                       │                    │
      │                  └───────────────┬───────────────────────┘                    │
      │                                  ▼                                            │
      │                     ┌─────────────────────────┐                               │
      │                     │  buzz-workflow (đã có)   │  ← engine thực thi step      │
      │                     │  cron, approval gate, WF │                               │
      │                     └─────────────────────────┘                               │
      └───────────────────────────────────────────────────────────────────────────────┘
                         │
      ┌──────────────────┴───────────────────────────────────────────────────────────┐
      │                         Agent surface (đã có + mở rộng)                       │
      │  buzz-cli · buzz-acp (Goose/Codex/Claude Code) · buzz-agent · buzz-dev-mcp     │
      │  buzz-persona  ← hợp nhất "persona pack" (Buzz) + "agent config" (cli-ui)      │
      │                 + "agent block" (Sim)                                          │
      └───────────────────────────────────────────────────────────────────────────────┘
                         │
      ┌──────────────────┴───────────────────────────────────────────────────────────┐
      │                       Client layer (Tauri 2 + React 19, web, mobile)          │
      │  Channels/Threads/DM/Voice (Buzz gốc)                                          │
      │  + Tab "Flow Studio"  (canvas kéo-thả từ Sim)                                  │
      │  + Tab "Agent Studio" (dependency graph, monitor từ cli-ui)                    │
      └─────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Cấu trúc thư mục monorepo sau khi hợp nhất

```
buzz-hive/                              # tên monorepo mới (root = fork của block/buzz)
├── AGENTS.md
├── ARCHITECTURE.md
├── VISION.md
├── Cargo.toml                          # workspace root (Rust)
├── package.json                        # workspace root (bun/pnpm, TS)
│
├── crates/                             # === Rust workspace (gốc Buzz) ===
│   ├── buzz-core/
│   ├── buzz-relay/
│   ├── buzz-db/
│   ├── buzz-auth/
│   ├── buzz-pubsub/
│   ├── buzz-search/
│   ├── buzz-audit/
│   ├── buzz-workflow/                  # engine thực thi (đã có, tái sử dụng)
│   ├── buzz-persona/                   # MỞ RỘNG: hợp nhất Agent config (cli-ui) + Agent block (Sim)
│   ├── buzz-cli/
│   ├── buzz-acp/
│   ├── buzz-agent/
│   ├── buzz-dev-mcp/
│   │
│   ├── buzz-flow/                      # MỚI — port nghiệp vụ từ simstudioai/sim
│   │   ├── src/
│   │   │   ├── blocks/                 # registry block: agent, condition, http, code, loop...
│   │   │   ├── tools/                  # tool registry (ported từ Sim's block/tool system)
│   │   │   ├── knowledge/              # pgvector embeddings, semantic search
│   │   │   ├── tables.rs               # "Tables" feature của Sim
│   │   │   ├── files.rs                # "Files" feature của Sim
│   │   │   ├── chat_bridge.rs          # cầu nối Sim "Chat" module ↔ Buzz channel/thread event
│   │   │   ├── events.rs               # định nghĩa kind 46200–46399 (xem mục 4)
│   │   │   └── lib.rs
│   │   └── Cargo.toml
│   │
│   └── buzz-agent-studio/              # MỚI — port nghiệp vụ từ Ngxba/claude-code-cli-ui
│       ├── src/
│       │   ├── graph.rs                # scan agent/command/skill frontmatter → dependency graph
│       │   ├── skill_import.rs         # GitHub import flow (repo → skill)
│       │   ├── monitor.rs              # token/cost/tool-call tracking theo session
│       │   ├── sse.rs                  # stream trạng thái phiên real-time
│       │   ├── events.rs               # định nghĩa kind 47200–47399 (xem mục 4)
│       │   └── lib.rs
│       └── Cargo.toml
│
├── desktop/                            # Tauri 2 + React 19 (gốc Buzz)
│   └── src/
│       ├── features/
│       │   ├── channels/               # Buzz gốc
│       │   ├── workflows/              # Buzz gốc (YAML automation UI)
│       │   ├── flow-studio/            # MỚI — canvas kéo-thả (port UI từ Sim, Next.js → React thuần)
│       │   │   ├── Canvas.tsx
│       │   │   ├── BlockPalette.tsx
│       │   │   ├── KnowledgeBasePanel.tsx
│       │   │   ├── TablesPanel.tsx
│       │   │   └── FilesPanel.tsx
│       │   └── agent-studio/           # MỚI — port UI từ claude-code-cli-ui (Nuxt/Vue → React)
│       │       ├── AgentGraph.tsx      # visual relationship mapping (agent/command/skill)
│       │       ├── SessionMonitor.tsx  # context/token/cost real-time (SSE)
│       │       ├── SkillImportModal.tsx# GitHub import flow
│       │       └── PersonaEditor.tsx
│       └── ...
│
├── web/                                # Buzz web client (repo browser tại myproject.com)
├── mobile/                             # Buzz mobile (Flutter, gốc)
│
├── migrations/                         # migration duy nhất: gộp schema Postgres của Sim vào buzz-db
│   └── 00xx_add_flow_and_agent_studio_tables.sql
│
└── docs/
    └── MERGE_NOTES.md                  # nhật ký quyết định merge, mapping tính năng
```

---

## 4. Data model — mở rộng Nostr kind

Buzz mở rộng NIP-01 bằng custom `kind` integer cho từng tính năng mới, tính năng mới **không** được phá vỡ client cũ (nguyên tắc "Zero breaking changes" của Buzz).

| Kind range | Nguồn gốc | Ý nghĩa |
|---|---|---|
| 1–9, 40001–4000x | Buzz gốc | message, reaction, channel, DM... |
| 46001–46012 | Buzz gốc | workflow execution (đã có) |
| **46200–46249** | **Sim → buzz-flow** | flow graph saved / block executed / block failed |
| **46250–46299** | **Sim → buzz-flow** | knowledge base: document ingested / embedding indexed / semantic query |
| **46300–46349** | **Sim → buzz-flow** | Tables: row created/updated/deleted (thay Drizzle+Postgres app-side state) |
| **46350–46399** | **Sim → buzz-flow** | Files: upload / version / delete |
| **47200–47249** | **cli-ui → buzz-agent-studio** | agent config created/updated (persona binding) |
| **47250–47299** | **cli-ui → buzz-agent-studio** | skill/command imported (kèm nguồn GitHub repo, commit sha) |
| **47300–47349** | **cli-ui → buzz-agent-studio** | session telemetry: token usage, cost, tool-call, per turn |
| **47350–47399** | **cli-ui → buzz-agent-studio** | dependency-graph edge (agent→command, command→skill) |

> Ghi chú kỹ thuật: các bảng nội bộ mà Sim dùng Drizzle/Postgres (Tables, Files, Knowledge) **vẫn giữ Postgres+pgvector làm read-model / cache**, nhưng **write path bắt buộc qua event** — `buzz-flow` subscribe chính relay của nó rồi project event thành row Postgres (giống cách buzz-search index Postgres FTS từ event log). Điều này giữ đúng nguyên tắc "audit trail" toàn cục của Buzz.

---

## 5. Điểm tích hợp UI (client)

| Khu vực Buzz Desktop hiện có | Bổ sung |
|---|---|
| Sidebar: Channels / Threads / DMs | + mục **Flow Studio** (canvas, giống Sim) |
| Sidebar: Workflows (YAML automation) | Nâng cấp: nút "Mở bằng Flow Studio" → chuyển YAML ↔ canvas kéo-thả |
| Team / Persona management (đã có) | + tab **Agent Studio**: dependency graph, GitHub skill import, session monitor (từ cli-ui) |
| Repo browser (git-sign-nostr) | Flow Studio's "Code block" và Agent Studio's "skill file" đều trỏ về cùng repo pointer đã có trong Buzz |

Luồng người dùng mẫu:
1. Người dùng tạo **Persona** "Reviewer" trong Buzz (đã có) → gán skill import từ GitHub qua **Agent Studio** (mới) → skill này xuất hiện dạng block trong **Flow Studio** (mới) để kéo vào workflow → workflow chạy qua `buzz-workflow` (đã có) → kết quả log thành event, hiện trực tiếp trong channel Buzz mà team đang xem.

---

## 6. Auth, quota & giới hạn kỹ thuật (kế thừa từ ARCHITECTURE.md của Buzz)

- Auth: NIP-42/98 Schnorr, dùng chung cho Flow Studio API và Agent Studio API — **không tạo hệ auth song song** (Sim vốn dùng Better Auth, cli-ui dùng key cục bộ — cả hai bị loại bỏ, thay bằng Buzz auth).
- Concurrency: workflow block execution trong `buzz-flow` tái sử dụng `Arc<Semaphore>` pattern đã có trong `buzz-workflow` (100 permits, `try_acquire`, trả `CapacityExceeded` ngay thay vì queue).
- Approval gate: nếu một block Sim (vd. "Human approval" block) cần dừng chờ người duyệt, dùng lại cơ chế `request_approval` / `StepResult::Suspended` đã có — **hiện đang gắn cờ 🚧 (WF-08, chưa persist token)** trong Buzz gốc, cần fix trước khi ship tính năng approval của Flow Studio.
- Multi-tenant: mọi bảng Tables/Files/Knowledge của Sim phải được scope theo `community_id` giống cách Buzz scope cache key, search doc, audit chain hiện nay.

---

## 7. Lộ trình triển khai (phased)

| Giai đoạn | Nội dung | Output |
|---|---|---|
| **P0 — Khảo sát & khung sườn** | Fork `block/buzz`, tạo skeleton `crates/buzz-flow` và `crates/buzz-agent-studio` rỗng, định nghĩa đầy đủ kind number ở mục 4 | Repo `buzz-hive` build được, chưa có tính năng |
| **P1 — Agent Studio (dễ hơn)** | Port `graph.rs` (scan frontmatter agent/command/skill), `skill_import.rs`, gắn vào `buzz-persona` | Tab Agent Studio hoạt động, đọc/ghi qua event kind 472xx/473xx |
| **P2 — Flow Studio (lõi)** | Port block/tool registry của Sim, canvas React, nối với `buzz-workflow` engine hiện có | Kéo-thả workflow, chạy qua engine Buzz, log event kind 46200+ |
| **P3 — Knowledge/Tables/Files** | Port pgvector knowledge base, Tables, Files; xây projector event→Postgres | Semantic search, bảng dữ liệu, quản lý file trong workspace |
| **P4 — Hợp nhất session monitor** | SSE token/cost monitor (cli-ui) áp dụng luôn cho session Flow Studio, không chỉ Agent Studio | 1 màn hình giám sát chi phí duy nhất cho cả 2 module |
| **P5 — Dọn dẹp & rebrand** | Xóa mã nguồn Next.js/Nuxt còn sót không dùng, chuẩn hoá theo Tauri/React của Buzz, cập nhật `AGENTS.md`, `VISION.md` | Sản phẩm hợp nhất "Buzz Hive" phát hành bản đầu |

---

## 8. Rủi ro & câu hỏi mở

- **Xung đột framework frontend:** Sim dùng Next.js, cli-ui dùng Nuxt/Vue, Buzz dùng React 19 + Tauri. Toàn bộ UI của 2 module mới cần **viết lại bằng React**, không thể "mount" trực tiếp — đây là phần tốn công nhất trong roadmap.
- **Approval-gate chưa hoàn thiện (WF-08)** ở Buzz gốc — cần fix trước khi Flow Studio phụ thuộc vào nó cho các block "cần duyệt".
- **pgvector cho knowledge base** cần thêm vào `buzz-db` (Buzz hiện dùng Postgres nhưng README không xác nhận sẵn pgvector) — cần audit schema hiện tại.
- **Giấy phép:** Buzz và Sim đều Apache-2.0; cần xác nhận giấy phép cụ thể của `claude-code-cli-ui` (ghi MIT theo mô tả tìm được) trước khi port code — MIT vào dự án Apache-2.0 nhìn chung tương thích nhưng cần giữ đúng attribution/NOTICE.
- **Định danh Agent xuyên hệ thống:** Buzz nhấn mạnh "danh tính agent di động, xác minh được" qua Nostr keys — cần đảm bảo Agent Studio (vốn quản lý config cục bộ trong `.claude/*`) không tạo ra một identity song song ngoài keypair Nostr của Buzz.

---

*Tài liệu này là bản đặc tả khái niệm dựa trên README/ARCHITECTURE/VISION công khai của 3 repo tại thời điểm 2026-08-17. Trước khi code, nên đọc trực tiếp mã nguồn `buzz-workflow`, schema Drizzle của `sim`, và cấu trúc `.claude/*` mà `claude-code-cli-ui` thao tác để chốt chi tiết field-level.*
