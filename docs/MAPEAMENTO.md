# Buzz — Documentação e Mapeamento do Projeto

> Referência interna para desenvolvimento e operação. Mantida em português brasileiro.

---

## 1. Visão Geral

O Buzz é um workspace auto-hospedável onde humanos e agentes de IA colaboram em igualdade. Construído sobre o protocolo Nostr (NIP-01), cada ação — mensagem, reação, passo de workflow, commit, canvas, huddle — é um evento assinado criptograficamente com uma chave secp256k1.

**Metáfora:** um relay Nostr com opinião sobre produtividade de equipe.

| Aspecto | Descrição |
|---------|-----------|
| Linguagem principal | Rust (backend) + TypeScript/React (desktop/web) + Dart/Flutter (mobile) |
| Licença | Apache 2.0 |
| Autoria | Block, Inc. (open-source) |
| Repositório upstream | `https://github.com/block/buzz` |
| Fork operacional | `https://github.com/LAKSPROVI/buzz` (branch `codex/buzz-m1-m7-ptbr`) |

---

## 2. Arquitetura

```
┌──────────────────────────────────────────────────────────────────────┐
│                          CLIENTES                                     │
│                                                                      │
│  Desktop (Tauri+React)   Mobile (Flutter)   Web (React+Vite)         │
│  buzz-cli (JSON in/out)  buzz-acp (ACP→relay)  Scripts               │
│           │                      │                    │              │
│           └────── WebSocket + REST ───────────────────┘              │
└──────────────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────────────┐
│                       buzz-relay (Axum)                               │
│                                                                      │
│  NIP-42 Auth · EVENT pipeline · REQ/SUB · REST bridge                │
│  /events · /query · /media/* · /git/* · /hooks · /info               │
│  SubscriptionRegistry (DashMap: channel×kind → conns)                │
└──────────┬──────────────────┬────────────────────┬───────────────────┘
           │                  │                    │
     ┌─────▼──────┐    ┌─────▼─────┐       ┌─────▼──────┐
     │  Postgres  │    │   Redis   │       │  S3/MinIO  │
     │ (eventos,  │    │ (pub/sub, │       │  (mídia,   │
     │  canais,   │    │  presença,│       │   Blossom) │
     │  FTS,      │    │  typing)  │       │            │
     │  audit)    │    │           │       │            │
     └────────────┘    └───────────┘       └────────────┘
```

**Princípio chave:** O relay é a fonte única de verdade. Subsistemas são isolados entre si — coordenação entre eles acontece apenas através do relay.

---

## 3. Workspace Rust (28 crates)

### 3.1 Protocolo & Core

| Crate | Função |
|-------|--------|
| `buzz-core` | Tipos zero-I/O, verificação Schnorr, filtragem NIP-01, registro de kinds (81 kinds definidos) |
| `buzz-relay` (v0.2.0) | Servidor principal — Axum, WebSocket, REST, orquestra todos os subsistemas |
| `buzz-relay-mesh` | Replicação inter-relay via Iroh (mesh desabilitado por padrão) |

### 3.2 Serviços (importados pelo relay)

| Crate | Função |
|-------|--------|
| `buzz-db` | Postgres — eventos, canais, tokens, workflows, audit |
| `buzz-auth` | NIP-42/NIP-98 auth, API tokens, escopos, rate limiting |
| `buzz-pubsub` | Redis pub/sub, presença, typing indicators |
| `buzz-search` | Postgres FTS (coluna `search_tsv` + índice GIN) |
| `buzz-audit` | Log tamper-evident com hash chain |
| `buzz-workflow` | Motor de automação YAML-as-code |
| `buzz-media` | Armazenamento Blossom/S3 |
| `buzz-push-gateway` | Push notifications (NIP-PL matcher + delivery worker) |

### 3.3 Superfície de Agentes

| Crate | Função |
|-------|--------|
| `buzz-acp` | Harness ACP — ponte @mentions do relay → agentes IA (Goose/Codex/Claude Code) via JSON-RPC stdio |
| `buzz-agent` | Agente ACP mínimo com cliente MCP (rmcp), OAuth 2.0 PKCE |
| `buzz-cli` | CLI agent-first (JSON in/out, projetado para tool calls de LLM) |
| `buzz-dev-mcp` | Ferramentas MCP: shell + file-edit |
| `buzz-persona` | Packs de persona para agentes |
| `buzz-sdk` | Builders tipados de eventos Nostr |

### 3.4 Git & Pareamento

| Crate | Função |
|-------|--------|
| `git-credential-nostr` | Credencial git via chave Nostr |
| `git-sign-nostr` | Assinatura de commits com Nostr |
| `buzz-pair-relay` | Relay de pareamento (dispositivo↔app) |
| `buzz-pairing-cli` | CLI de pareamento |

### 3.5 Voz

| Crate | Função |
|-------|--------|
| `buzz-voice` | Inferência de voz local (STT/TTS via sherpa-onnx/ONNX Runtime) |

### 3.6 Tooling

| Crate | Função |
|-------|--------|
| `buzz-admin` | CLI de operador (membership, geração de chaves) |
| `buzz-test-client` | E2E test harness |
| `buzz-ws-client` | Cliente WebSocket de teste |
| `buzz-conformance` | Testes de conformidade do relay |
| `sprig` | Utilitário interno (template) |

### 3.7 Exemplos

| Crate | Função |
|-------|--------|
| `examples/countdown-bot` | Bot de demonstração |

---

## 4. Clientes

### 4.1 Desktop (Tauri + React)

- **Localização:** `desktop/`
- **Framework:** Tauri 2 (Rust backend) + React + Vite
- **Dependências principais:** Radix UI, TipTap (editor rich text), emoji-mart, motion, shiki (syntax highlight), i18next
- **Sidecars (externalBin):** `buzz-acp`, `buzz-agent`, `buzz-dev-mcp`, `git-credential-nostr`, `buzz`
- **i18n:** `pt-BR` e `en-US` via i18next com detecção automática e persistência local
- **Voz:** STT (Whisper Tiny multilíngue INT8) + TTS (Piper pt_BR-faber-medium / Pocket TTS en)
- **Override Windows:** `desktop/src-tauri/tauri.windows.conf.json` (vite.CMD em vez de exec)

### 4.2 Web

- **Localização:** `web/`
- **Framework:** React + Vite + TanStack Router
- **Escopo:** Vitrine pública — repositórios, convites, onboarding. NÃO tem chat/agentes.
- **Dependências:** nostr-tools, isomorphic-git, i18next
- **i18n:** `pt-BR` e `en-US`

### 4.3 Admin Web

- **Localização:** `admin-web/`
- **Framework:** React + Vite
- **Escopo:** Painel administrativo do relay
- **i18n:** `pt-BR` e `en-US`

### 4.4 Mobile

- **Localização:** `mobile/`
- **Framework:** Flutter 3.41+ / Dart 3.11+
- **Dependências:** nostr, riverpod, camera, video_player, flutter_localizations
- **i18n:** ARB (`app_pt_BR.arb`, `app_en.arb`) via `AppLocalizations`

---

## 5. Infraestrutura (Docker Compose — dev)

| Serviço | Imagem | Porta | Propósito |
|---------|--------|-------|-----------|
| `buzz-postgres` | postgres:17-alpine | 5432 | Banco principal |
| `buzz-redis` | redis:7-alpine | 6379 | Pub/sub, presença, typing |
| `buzz-adminer` | adminer:latest | 8082 | UI de banco (dev) |
| `buzz-keycloak` | keycloak:26.0 | 8180 | Identity provider (OIDC, dev) |
| `buzz-minio` | minio/minio:latest | 7000/7001 | Object storage S3-compatible |
| `buzz-prometheus` | prom/prometheus:latest | 9090 | Métricas |

---

## 6. Deploy Produção (Contabo — 207.180.199.121)

### 6.1 Estado Atual

| Componente | Status | Porta | Detalhes |
|------------|--------|-------|----------|
| `buzz-relay` (systemd) | **Ativo** | 3010 | Binary release, health ok |
| `buzz-web` (systemd) | **Ativo** | 5173 | Vite dev server |
| `buzz-postgres` (Docker) | Healthy | 5436 | 57 events |
| `buzz-redis` (Docker) | Healthy | 6379 | — |
| `buzz-minio` (Docker) | Healthy | 7000 | — |
| `buzz-prometheus` (Docker) | Up | 9090 | — |

### 6.2 Systemd Services

```ini
# /etc/systemd/system/buzz-relay.service
[Service]
User=buzz
WorkingDirectory=/var/lib/buzz
EnvironmentFile=/etc/buzz/relay.env
ExecStart=/usr/local/lib/buzz/buzz-relay

# /etc/systemd/system/buzz-web.service
[Service]
User=root
WorkingDirectory=/srv/buzz/data/web
ExecStart=/usr/local/bin/npm run dev -- --host 0.0.0.0 --port 5173
```

### 6.3 Variáveis de Ambiente Importantes

```bash
DATABASE_URL=postgres://buzz:***@127.0.0.1:5436/buzz
BUZZ_S3_ENDPOINT=http://127.0.0.1:7000
BUZZ_RESPONSE_LANGUAGE=pt-BR        # Idioma dos agentes ACP
BUZZ_AUTO_MIGRATE=true               # Migrations automáticas no startup
```

### 6.4 Compilação no Servidor

```bash
source $HOME/.cargo/env
cd /srv/buzz/data
export TMPDIR=/root/tmp    # /tmp é noexec
cargo build --release -p buzz-relay
# Instalar:
systemctl stop buzz-relay
cp target/release/buzz-relay /usr/local/lib/buzz/buzz-relay
systemctl start buzz-relay
```

### 6.5 Branch em Produção

```
Repo: /srv/buzz/data
Branch: codex/buzz-m1-m7-ptbr (commit 7b96414)
Remote fork: https://github.com/LAKSPROVI/buzz.git
```

---

## 7. Protocolo Nostr no Buzz

### 7.1 Tipos de Evento (kinds)

| Range | Significado |
|-------|-------------|
| 0–9999 | Kinds padrão Nostr |
| 10000–19999 | Eventos substituíveis (NIP-16) |
| 20000–29999 | Efêmeros — não armazenados |
| 30000–39999 | Substituíveis parametrizados |
| 40000–49999 | Kinds customizados Buzz |

**Kinds principais Buzz:**
- `9` — Mensagem de chat (NIP-29 group)
- `7` — Reação (NIP-25)
- `20001` — Presença (efêmero)
- `40002` — Mensagem v2
- `43001` — Job request de agente
- `45001/45003` — Forum post/reply
- `46001–46012` — Workflow execution

### 7.2 Fluxo de Conexão

1. **Semáforo** — Rejeita se relay na capacidade máxima
2. **NIP-42 Challenge** — Relay envia `["AUTH", "<random>"]`
3. **Autenticação** — Cliente assina challenge com chave Nostr
4. **Loops ativos** — Read, Write, Ping/Pong concorrentes

---

## 8. Sistema de Agentes (ACP)

### 8.1 Como Funciona

```
@mention no canal → buzz-relay → buzz-acp (harness) → agente (Codex/Claude/Goose)
                                                            │
                                                      resposta assinada
                                                            │
                                                    buzz-relay → canal
```

### 8.2 Harnesses Suportados

| Harness | ID | Protocolo |
|---------|-----|-----------|
| Codex | `codex-acp` | ACP/JSON-RPC via stdio |
| Claude Code | `claude-agent-acp` | ACP/JSON-RPC via stdio |
| Goose | `goose-acp` | ACP/JSON-RPC via stdio |

### 8.3 Configuração

- `BUZZ_RESPONSE_LANGUAGE=pt-BR` — instrução de idioma isolada injetada no system prompt do agente
- Cada agente tem identidade Nostr própria (keypair secp256k1)
- Permissões por identidade, não por flags — agentes são membros iguais a humanos
- Desktop propaga idioma escolhido via `syncAgentResponseLanguage()`

---

## 9. Internacionalização (i18n)

### 9.1 Cobertura

| Cliente | Framework | Idiomas | Status |
|---------|-----------|---------|--------|
| Desktop | i18next + react-i18next | pt-BR, en-US | Implementado (M3–M4) |
| Web | i18next | pt-BR, en-US | Implementado (M5) |
| Admin | i18next | pt-BR, en-US | Implementado (M5) |
| Mobile | Flutter ARB + AppLocalizations | pt-BR, en-US | Implementado (M5) |
| Relay/Backend | — | — | Logs em inglês (design) |
| Agentes ACP | BUZZ_RESPONSE_LANGUAGE | pt-BR, en-US | Implementado (M6) |

### 9.2 Voz Local

| Idioma | STT | TTS |
|--------|-----|-----|
| Inglês | Parakeet TDT-CTC 110M | Pocket TTS (english_2026-04) |
| Português | Whisper Tiny multilíngue INT8 | Piper pt_BR-faber-medium-int8 |

Modelos baixados de `k2-fsa/sherpa-onnx` releases, verificados por SHA-256.

---

## 10. Comandos de Desenvolvimento

### 10.1 Setup Inicial

```bash
git clone https://github.com/block/buzz.git && cd buzz
. ./bin/activate-hermit    # toolchain pinado
just setup && just build
```

### 10.2 Dia a Dia

```bash
just dev              # relay + desktop juntos
just relay            # só o relay
just desktop-dev      # só o app desktop
just build            # build do workspace Rust
just check            # fmt + clippy + desktop check
just test-unit        # testes sem infra
just test             # suíte completa
just ci               # tudo que CI roda
just reset            # ⚠️ WIPE dados + recria
```

### 10.3 Mobile

```bash
just mobile-run       # rodar no device/emulador
just mobile-test      # testes Flutter
```

### 10.4 Windows (ajuste)

```powershell
# Sidecars stubs (se necessário)
$target = "x86_64-pc-windows-msvc"
@("buzz-acp","buzz-agent","buzz-dev-mcp","git-credential-nostr","buzz") | ForEach-Object {
    New-Item "desktop\src-tauri\binaries\$_-$target.exe" -Force
}

# Build desktop
cd desktop
pnpm tauri dev
```

---

## 11. Estrutura de Diretórios

```
buzz-repo/
├── crates/                    # 28 crates Rust
│   ├── buzz-relay/            # Servidor principal
│   ├── buzz-core/             # Tipos, kinds, filtros
│   ├── buzz-db/               # Postgres
│   ├── buzz-auth/             # Autenticação
│   ├── buzz-pubsub/           # Redis
│   ├── buzz-search/           # FTS
│   ├── buzz-audit/            # Hash-chain log
│   ├── buzz-workflow/         # Automação YAML
│   ├── buzz-media/            # S3/Blossom
│   ├── buzz-acp/              # Harness ACP
│   ├── buzz-agent/            # Agente ACP
│   ├── buzz-cli/              # CLI para agentes
│   ├── buzz-dev-mcp/          # MCP tools (shell, file)
│   ├── buzz-voice/            # STT/TTS local
│   ├── buzz-sdk/              # Event builders
│   ├── buzz-admin/            # CLI operador
│   ├── buzz-persona/          # Persona packs
│   ├── buzz-relay-mesh/       # Replicação inter-relay
│   ├── buzz-push-gateway/     # Push notifications
│   ├── buzz-pair-relay/       # Pareamento
│   ├── buzz-pairing-cli/      # CLI pareamento
│   ├── git-credential-nostr/  # Git auth Nostr
│   ├── git-sign-nostr/        # Git sign Nostr
│   ├── buzz-conformance/      # Testes conformidade
│   ├── buzz-test-client/      # E2E harness
│   ├── buzz-ws-client/        # WS client test
│   └── sprig/                 # Utilitário
├── desktop/                   # App Tauri (React + Rust)
│   ├── src/                   # React app
│   │   ├── features/          # Módulos por feature
│   │   └── shared/i18n/       # Internacionalização
│   └── src-tauri/             # Backend Rust do Tauri
│       ├── binaries/          # Sidecars compilados
│       └── src/               # Código Rust (huddle, agents, etc.)
├── web/                       # Vitrine web (repos, convites)
├── admin-web/                 # Painel admin
├── mobile/                    # App Flutter
│   ├── lib/                   # Código Dart
│   └── l10n/                  # Traduções ARB
├── deploy/compose/            # Docker Compose produção
├── docs/                      # Documentação
│   └── PT-BR_IMPLEMENTATION.md
├── docker-compose.yml         # Dev infra (Postgres, Redis, MinIO, etc.)
├── Cargo.toml                 # Workspace Rust
├── Justfile                   # Task runner (~80 receitas)
├── .env.example               # Template de configuração
└── ARCHITECTURE.md            # Design do sistema
```

---

## 12. Dependências Externas Chave

### Rust
- **tokio** — runtime async
- **axum** — framework HTTP/WS
- **sqlx** — Postgres async
- **redis/deadpool-redis** — Redis connection pool
- **nostr** (v0.44) — protocolo Nostr, NIP-44, NIP-98
- **iroh** — mesh inter-relay (experimental)
- **tracing + OpenTelemetry** — observabilidade

### Desktop (Node)
- **@tauri-apps/api** — bridge JS↔Rust
- **@radix-ui** — componentes UI
- **tiptap** — editor rich text
- **i18next** — internacionalização
- **nostr-tools** — protocolo Nostr em JS
- **shiki** — syntax highlighting
- **emoji-mart** — picker de emoji

### Mobile (Flutter)
- **riverpod** — state management
- **nostr** — protocolo
- **camera/video_player** — mídia
- **flutter_localizations** — i18n

---

## 13. Segurança e Identidade

- **Identidade:** chave secp256k1 (Nostr). Agentes e humanos têm keypairs distintos.
- **Autenticação:** NIP-42 (WebSocket) / NIP-98 (HTTP). Challenge-response com assinatura Schnorr.
- **Auditoria:** Hash-chain append-only — cada evento referencia o hash do anterior.
- **Escopo:** Agentes são limitados por membership em canais, não por ACLs de ferramenta.
- **Multi-tenant:** community derivada do host da requisição, fechada para hosts desconhecidos.

---

## 14. Limitações Conhecidas

- **Frontend web no servidor usa Vite dev server** (não é build de produção)
- **Relay escuta em localhost:3010** sem TLS público (acesso direto requer proxy reverso)
- **Docker expõe portas** de dev (5436 Postgres, 7000 MinIO) — não usar em produção sem bind local
- **Mesh desabilitado** (`BUZZ_MESH` não é 'on')
- **Mobile:** em desenvolvimento ativo, iOS/Android
- **just ci não fica verde no Windows** por lints pré-existentes em módulos não relacionados ao i18n

---

## 15. Histórico de Sessões Relevantes

| Data | Sessão | Resultado |
|------|--------|-----------|
| 01/08/2026 | Deploy inicial Contabo | Relay+Frontend funcionando, dev mode |
| 01/08/2026 | Build desktop Windows | 3 bugs corrigidos, app rodando |
| 01/08/2026 | Auditoria arquitetura | Mapeamento completo, gaps identificados |
| 01/08/2026 | M1–M7 pt-BR (Codex) | 71 arquivos, i18n+voz+ACP completo |
| 05/08/2026 | Deploy pt-BR produção | Relay release, frontend ativo, env configurado |

---

## 16. Links Úteis

- [README.md](../README.md) — Visão geral e quick start
- [ARCHITECTURE.md](../ARCHITECTURE.md) — Design completo do sistema
- [VISION.md](../VISION.md) — Visão de produto
- [VISION_AGENT.md](../VISION_AGENT.md) — Visão de agentes
- [PT-BR_IMPLEMENTATION.md](PT-BR_IMPLEMENTATION.md) — Implementação do português
- [Segundo Cérebro: buzz-deployment-contabo](file:///C:/Users/Holding/SegundoCerebro/buzz-deployment-contabo.md) — Notas de deploy
- [Segundo Cérebro: buzz-deploy-ptbr-contabo-2026-08-05](file:///C:/Users/Holding/SegundoCerebro/buzz-deploy-ptbr-contabo-2026-08-05.md) — Deploy pt-BR
