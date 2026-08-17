# Implementation Plan: **Buzz Hive**

> Đi kèm với `BUZZ_HIVE_MERGE_SPEC.md`. Tài liệu này trả lời **ai làm gì, khi nào, xong khi nào biết là xong**.
> Ước lượng dựa trên team giả định: 2 Rust backend, 2 frontend (React/Tauri), 1 DevOps/infra kiêm bán thời gian, 1 PM/tech lead.

---

## 0. Nguyên tắc lập kế hoạch

- Đi theo đúng 5 giai đoạn P0–P5 trong spec, nhưng mỗi giai đoạn ở đây được chẻ thành **sprint 2 tuần** với task cụ thể + Definition of Done (DoD).
- **Không giai đoạn nào bắt đầu khi giai đoạn trước chưa qua DoD** — trừ P1 và một phần P2 có thể chạy song song vì ít phụ thuộc lẫn nhau (Agent Studio ít đụng workflow engine hơn Flow Studio).
- Mỗi sprint kết thúc bằng **demo chạy thật** trên nhánh `main`, không demo bằng slide.

---

## 1. Tổng quan timeline (ước lượng ~24 tuần / 6 tháng, 12 sprint)

| Giai đoạn | Sprint | Tuần | Nội dung chính |
|---|---|---|---|
| P0 — Khung sườn | S1–S2 | 1–4 | Fork, dựng skeleton crate, định nghĩa kind, CI |
| P1 — Agent Studio | S3–S4 | 5–8 | Port graph/skill-import, event kind 472xx/473xx |
| P2 — Flow Studio (lõi) | S5–S7 | 9–14 | Block/tool registry, canvas React, nối workflow engine |
| P3 — Knowledge/Tables/Files | S8–S9 | 15–18 | pgvector, projector event→Postgres, UI panel |
| P4 — Hợp nhất Session Monitor | S10 | 19–20 | 1 màn hình cost/token chung |
| P5 — Dọn dẹp & Rebrand | S11–S12 | 21–24 | Xoá code thừa, hardening, release bản đầu |

Có thể rút còn ~16 tuần nếu bỏ song song hoá an toàn và chấp nhận rủi ro kỹ thuật cao hơn ở P2.

---

## 2. Chi tiết theo Sprint

### 🟦 Sprint 1 (Tuần 1–2) — Khởi tạo repo & workspace

**Mục tiêu:** có `buzz-hive` build xanh, chưa cần chạy được gì mới.

| Task | Người phụ trách | Output |
|---|---|---|
| Fork `block/buzz` → `buzz-hive`, giữ nguyên lịch sử git, cập nhật `README`/`AGENTS.md` | Tech lead | Repo mới, CI pass |
| Audit schema Postgres hiện tại của Buzz (`buzz-db`) — xác nhận có/không pgvector, đối chiếu với schema Drizzle của Sim | Backend #1 | `docs/DB_AUDIT.md` |
| Đọc source `buzz-workflow` để hiểu rõ `StepResult`, `Suspended`, approval-gate hiện tại (đang 🚧 WF-08) | Backend #2 | Ghi chú kỹ thuật + issue fix WF-08 |
| Xác nhận license `claude-code-cli-ui` (MIT) tương thích Apache-2.0, chuẩn bị NOTICE file | Tech lead | `NOTICE` cập nhật |
| Setup Hermit env + `just setup` chạy được trên máy cả team | DevOps | Onboarding doc |

**DoD:** `just ci` xanh trên fork, doc audit DB xong, issue WF-08 được tạo và ước lượng.

---

### 🟦 Sprint 2 (Tuần 3–4) — Skeleton crate mới + Kind registry

| Task | Người phụ trách | Output |
|---|---|---|
| Tạo `crates/buzz-flow` rỗng (chỉ `lib.rs`, `Cargo.toml`, đăng ký vào workspace) | Backend #1 | Crate compile, chưa có logic |
| Tạo `crates/buzz-agent-studio` rỗng tương tự | Backend #2 | Crate compile |
| Viết `events.rs` cho cả 2 crate — định nghĩa struct Rust cho toàn bộ kind 46200–46399 và 47200–47399 theo bảng ở spec mục 4 | Backend #1 + #2 | Kind định nghĩa, unit test serialize/deserialize NIP-01 |
| Thêm feature flag / route rỗng cho `flow-studio` và `agent-studio` trong `desktop/src/features/` (chỉ khung React, chưa logic) | Frontend #1 | Tab hiện trong sidebar, nội dung "Coming soon" |
| CI: thêm job build cho 2 crate mới | DevOps | Pipeline pass |

**DoD:** Build workspace pass với 2 crate mới; toàn bộ kind number có test roundtrip; 2 tab UI rỗng hiển thị được trong desktop app.

---

### 🟩 Sprint 3 (Tuần 5–6) — Agent Studio: Dependency graph

| Task | Người phụ trách |
|---|---|
| Port thuật toán scan frontmatter (agent/command/skill → tham chiếu `agent:` / `/command`) từ `claude-code-cli-ui` sang Rust trong `graph.rs` | Backend #2 |
| Thiết kế API: `GET /agent-studio/graph` trả về node/edge, backed bởi event kind 47350–47399 | Backend #2 |
| Port UI `AgentGraph.tsx` (Vue → React), dùng lib graph tương đương (vd. reactflow) thay VueFlow | Frontend #1 |
| Viết event `agent_config_created` (47200) khi user tạo/sửa persona qua Agent Studio, nối vào `buzz-persona` hiện có | Backend #1 | 

**DoD:** Tạo 1 persona mới qua UI → thấy node xuất hiện trên graph, event log thấy đúng kind 47200 + 47350.

---

### 🟩 Sprint 4 (Tuần 7–8) — Agent Studio: GitHub import + Session monitor (v1)

| Task | Người phụ trách |
|---|---|
| Port `skill_import.rs`: nhập skill từ GitHub repo (clone/tải, parse, ghi thành event kind 47250) | Backend #2 |
| UI `SkillImportModal.tsx`: nhập URL repo → chọn skill → import | Frontend #1 |
| Port `monitor.rs` v1: nhận stream token/cost/tool-call từ ACP session (`buzz-acp`), phát event kind 47300 | Backend #1 |
| UI `SessionMonitor.tsx` v1: bảng real-time qua SSE | Frontend #2 |
| **Demo P1 hoàn chỉnh** trước stakeholder | Cả team |

**DoD (chốt P1):** Import 1 skill thật từ GitHub công khai → skill khả dụng trong danh sách persona; theo dõi 1 phiên Claude Code chạy qua `buzz-acp` thấy token/cost cập nhật real-time trên UI.

---

### 🟨 Sprint 5 (Tuần 9–10) — Flow Studio: Block/Tool registry

| Task | Người phụ trách |
|---|---|
| Port cấu trúc block/tool registry của Sim sang Rust (`blocks/`, `tools/`) — tối thiểu: Agent block, Condition block, HTTP block, Code block | Backend #1 |
| Định nghĩa cách 1 block map sang `StepResult` của `buzz-workflow` hiện có (không viết engine mới) | Backend #1 + #2 |
| Viết fix cho WF-08 (persist approval token, resume `execute_from_step`) — **bắt buộc xong trước khi có block "Human approval"** | Backend #2 |
| Thiết kế event kind 46200–46249 (flow saved / block executed / block failed) | Backend #1 |

**DoD:** Chạy được 1 workflow gồm 2 block (HTTP → Condition) hoàn toàn qua CLI/test, không cần UI, log đúng event, WF-08 fixed và có test.

---

### 🟨 Sprint 6 (Tuần 11–12) — Flow Studio: Canvas UI

| Task | Người phụ trách |
|---|---|
| `Canvas.tsx` kéo-thả (reactflow), tương tác với block registry qua API | Frontend #1 + #2 |
| `BlockPalette.tsx` — danh sách block khả dụng, kéo vào canvas | Frontend #2 |
| Nút "Chuyển đổi YAML ↔ Canvas" cho workflow cũ của Buzz (tương thích ngược) | Frontend #1 + Backend #2 |
| Nối persona/agent (từ Agent Studio) làm 1 loại block trong palette | Backend #1 + Frontend #1 |

**DoD:** Người dùng tạo workflow bằng kéo-thả từ đầu đến cuối, lưu, chạy, thấy kết quả trong channel Buzz — không cần sửa YAML tay.

---

### 🟨 Sprint 7 (Tuần 13–14) — Flow Studio: hoàn thiện + Human approval block

| Task | Người phụ trách |
|---|---|
| Block "Human approval" dùng approval-gate đã fix ở Sprint 5 | Backend #2 |
| Loop block, error-handling/retry ở cấp block | Backend #1 |
| UI: trạng thái block (running/success/fail/suspended) hiển thị trực tiếp trên canvas | Frontend #1 |
| Test tải: 100 workflow chạy đồng thời qua `Arc<Semaphore>` (đã có), xác nhận `CapacityExceeded` trả đúng, không deadlock | Backend #2 + DevOps |
| **Demo chốt P2** | Cả team |

**DoD (chốt P2):** Một workflow có bước cần người duyệt → tạm dừng đúng, người dùng duyệt trong Buzz UI (không phải Flow Studio riêng) → workflow resume và chạy tiếp.

---

### 🟧 Sprint 8 (Tuần 15–16) — Knowledge base + Projector event→Postgres

| Task | Người phụ trách |
|---|---|
| Thêm pgvector extension vào `buzz-db` (nếu Sprint 1 audit xác nhận chưa có) | Backend #1 + DevOps |
| Viết "projector": subscribe relay, nhận event kind 46250–46299 → ghi/update row Postgres (đọc nhanh, KHÔNG phải nguồn sự thật) | Backend #2 |
| `knowledge/` crate: ingest document → embedding → lưu vector, expose semantic search API | Backend #1 |
| UI `KnowledgeBasePanel.tsx`: upload tài liệu, tìm kiếm ngữ nghĩa | Frontend #2 |

**DoD:** Upload 1 file text → thấy event ingest → query ngữ nghĩa trả kết quả đúng trong <2s.

---

### 🟧 Sprint 9 (Tuần 17–18) — Tables + Files

| Task | Người phụ trách |
|---|---|
| `tables.rs`: CRUD row qua event kind 46300–46349, projector tương ứng | Backend #2 |
| `files.rs`: upload/version/xoá qua event kind 46350–46399, tái sử dụng cơ chế media của Buzz (media sharing đã có) thay vì viết storage riêng | Backend #1 |
| UI `TablesPanel.tsx`, `FilesPanel.tsx` | Frontend #1 + #2 |
| Đảm bảo toàn bộ Tables/Files/Knowledge scope đúng theo `community_id` (multi-tenant) — viết test isolation | Backend #2 |

**DoD (chốt P3):** Test 2 community khác nhau không thấy dữ liệu Tables/Files/Knowledge của nhau (test tự động, không chỉ kiểm tra tay).

---

### 🟥 Sprint 10 (Tuần 19–20) — Hợp nhất Session Monitor

| Task | Người phụ trách |
|---|---|
| Mở rộng `monitor.rs` để nhận cả token/cost từ block "Agent" trong Flow Studio, không chỉ session ACP của Agent Studio | Backend #1 |
| 1 màn hình `UnifiedCostMonitor.tsx` gộp cả 2 nguồn | Frontend #1 |
| Cảnh báo ngưỡng chi phí (theo community), thông báo qua channel Buzz | Backend #2 + Frontend #2 |

**DoD (chốt P4):** Chạy 1 workflow gọi agent + 1 phiên Claude Code trực tiếp, cả 2 chi phí cộng dồn đúng trên 1 dashboard.

---

### ⬛ Sprint 11 (Tuần 21–22) — Dọn dẹp

| Task | Người phụ trách |
|---|---|
| Xoá toàn bộ code Next.js (Sim) / Nuxt (cli-ui) còn sót trong repo tạm dùng để tham chiếu port | Frontend #1 + #2 |
| Chuẩn hoá style/lint theo chuẩn Buzz (Rust fmt, biome cho desktop/web/mobile) | DevOps |
| Cập nhật `ARCHITECTURE.md`, `VISION.md`, `AGENTS.md` phản ánh kiến trúc mới | Tech lead |
| Security review: auth path của Flow Studio & Agent Studio không có đường vòng qua auth Buzz | Backend #1 + #2 |

**DoD:** Không còn file `.tsx`/`.vue` gốc ngoài source đã port; docs khớp code thật; security review ký duyệt.

---

### ⬛ Sprint 12 (Tuần 23–24) — Hardening & Release

| Task | Người phụ trách |
|---|---|
| Load test toàn hệ thống (chat + workflow + agent studio đồng thời) | DevOps |
| Viết `RELEASING.md` cập nhật cho luồng release mới (đã có sẵn ở Buzz, chỉ bổ sung 2 module) | Tech lead |
| Beta release nội bộ, thu thập feedback 1 tuần | Cả team |
| Fix bug chặn release, tag `v0.1.0-buzz-hive` | Cả team |

**DoD (chốt toàn dự án):** Bản release `v0.1.0-buzz-hive` chạy self-host qua Docker Compose, đủ 3 tính năng: Channel/Workflow gốc, Flow Studio, Agent Studio.

---

## 3. Bảng phân bổ nhân sự (RACI rút gọn)

| Vai trò | P0 | P1 | P2 | P3 | P4 | P5 |
|---|---|---|---|---|---|---|
| Tech Lead | R | C | C | C | C | R |
| Backend #1 | R | C | R | R | R | C |
| Backend #2 | C | R | R | C | C | C |
| Frontend #1 | C | R | R | C | C | R |
| Frontend #2 | — | C | C | R | R | R |
| DevOps | R | C | C | R | — | R |

(R = Responsible/thực thi chính, C = Contribute/hỗ trợ)

---

## 4. Rủi ro theo lịch & phương án dự phòng

| Rủi ro | Ảnh hưởng lịch | Phương án |
|---|---|---|
| WF-08 (approval gate) phức tạp hơn ước lượng | Trễ Sprint 5–7 (P2) | Cắt block "Human approval" ra khỏi P2, đẩy sang P4, Flow Studio v1 ship không có approval |
| pgvector chưa có sẵn trong `buzz-db`, cần migrate dữ liệu lớn | Trễ Sprint 8 | Thêm 1 sprint đệm P3, chạy migration nền trong lúc P2 đang chạy |
| Port UI React từ 2 framework khác quá tốn công | Trễ tất cả sprint có Frontend | Ưu tiên port logic (hooks/state) trước, UI thô (chưa đẹp) trước, polish ở P5 |
| License MIT/Apache xung đột phát sinh khi audit kỹ | Chặn toàn bộ port code cli-ui | Viết lại thuật toán graph/skill-import từ đặc tả hành vi thay vì copy code, chỉ giữ ý tưởng |

---

## 5. Chỉ số thành công (Success Metrics) sau P5

- 100% dữ liệu Flow Studio/Agent Studio đi qua event log (audit bằng cách tắt projector, xác nhận event vẫn tái tạo được toàn bộ state).
- Độ trễ workflow event → UI cập nhật < 500ms (kế thừa yêu cầu real-time của Buzz gốc).
- 0 identity song song ngoài Nostr keypair cho agent.
- Community A không truy cập được bất kỳ dữ liệu Flow/Agent Studio nào của Community B (test tự động trong CI, không phải kiểm tra thủ công).
