# Buzz em português brasileiro: arquitetura e execução M1–M7

## Resultado

O Buzz agora possui a fundação de `pt-BR` e `en-US` nas interfaces desktop, web, administração e mobile. Os fluxos principais de agentes, configurações, convite, repositórios, onboarding e pareamento foram traduzidos; superfícies legadas ainda não migradas usam o fallback em inglês. No desktop, o idioma escolhido também governa:

- o idioma de resposta dos agentes ACP (Codex e Claude Code);
- o reconhecimento de fala local (STT);
- a síntese de fala local (TTS).

O inglês permanece disponível e funciona como fallback. A mudança não altera comandos, código, caminhos, identificadores nem citações literais produzidos pelos agentes.

## Como o projeto funciona

O Buzz usa um relay Nostr como fonte de verdade. Pessoas, agentes, workflows, mensagens, reações, Git e auditoria produzem eventos assinados no mesmo log.

- `buzz-relay`: WebSocket/REST, autenticação e coordenação do workspace.
- `buzz-core`, `buzz-db`, `buzz-auth`, `buzz-pubsub`, `buzz-search` e `buzz-audit`: protocolo, Postgres, identidade, Redis, busca e trilha de auditoria.
- `buzz-acp`, `buzz-agent`, `buzz-dev-mcp` e `buzz-cli`: ponte entre agentes ACP, ferramentas MCP e o relay.
- `buzz-workflow`: automações YAML e aprovações.
- `buzz-media`: armazenamento compatível com S3/MinIO.
- `buzz-voice`: inferência de voz usada pelo desktop.
- `desktop`: Tauri/Rust + React.
- `web` e `admin-web`: clientes React.
- `mobile`: Flutter.

O deploy de nó único usa relay, Postgres, Redis, MinIO e proxy TLS. Portas internas não precisam ser publicadas; clientes acessam apenas o endpoint TLS do relay.

## Integração de Codex e Claude Code

Ambos funcionam como harnesses ACP oficiais:

- Codex: `codex-acp`;
- Claude Code: `claude-agent-acp`.

O `buzz-acp` recebe eventos do canal, inicia o harness configurado, oferece ferramentas MCP do Buzz e publica a resposta assinada no relay. A autenticação de cada CLI continua sob controle do próprio provedor. A variável `BUZZ_RESPONSE_LANGUAGE` adiciona uma instrução de idioma isolada ao prompt de sistema; valores aceitos: `pt-BR` e `en-US`.

Exemplo:

```env
BUZZ_RESPONSE_LANGUAGE=pt-BR
```

A preferência global é propagada pelo desktop. Uma variável definida no ambiente de um agente/persona específico continua tendo precedência, permitindo override por agente.

## Internacionalização

### Desktop, web e administração

As aplicações React usam `i18next` e `react-i18next`, com recursos tipados para `pt-BR` e `en-US`. O idioma é detectado pelo navegador/sistema, persistido localmente e pode ser alterado nas configurações.

### Mobile

O Flutter usa ARB, `flutter_localizations`, delegates e persistência da preferência. Os fluxos principais de onboarding, pareamento, conexão, aparência e remoção de comunidade têm textos em português e inglês.

## Voz local em pt-BR

Nenhum áudio precisa sair do dispositivo.

- STT: Whisper Tiny multilíngue INT8, executado pelo `sherpa-onnx`, com `language=pt` e `task=transcribe`.
- TTS: Piper `pt_BR-faber-medium-int8`, executado pelo `sherpa-onnx`/VITS.
- Inglês: Parakeet TDT-CTC + Pocket TTS continuam inalterados.

Os arquivos são baixados das releases oficiais do `sherpa-onnx`, verificados por SHA-256, extraídos com bloqueio de path traversal/symlinks e instalados por troca atômica. Um manifesto versionado evita reutilizar modelos incompletos ou incompatíveis.

Fontes dos modelos:

- <https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models>
- <https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models>
- <https://k2-fsa.github.io/sherpa/onnx/tts/all/>

## Execução por marco

1. **M1 — baseline e segurança:** inventário local/servidor, backup antes de mudanças e redução da superfície exposta.
2. **M2 — agentes reais:** instalação dos adaptadores ACP e homologação de publicação de eventos por Codex e Claude Code.
3. **M3 — fundação i18n:** catálogo, detecção, persistência e seletor de idioma no desktop.
4. **M4 — desktop:** localização das superfícies principais de agentes e configurações.
5. **M5 — demais clientes:** web, administração e mobile, com fallback regional `pt` → `pt_BR` no Flutter.
6. **M6 — respostas dos agentes:** propagação segura do idioma escolhido ao `buzz-acp`, com override por agente.
7. **M7 — voz e acabamento:** Whisper/Piper pt-BR, migração das configurações de voz, hashes fixados, testes e documentação.

## Operação e rollback

- Antes de atualizar produção, preserve banco, volumes e configuração do relay.
- A troca de idioma reinicia somente pipelines/agentes afetados; eventos e canais não são alterados.
- Para voltar ao comportamento anterior, selecione `English (US)`; os modelos ingleses continuam separados no cache.
- Modelos incompletos não recebem o manifesto de pronto e não são carregados.
- Nunca coloque chaves privadas, tokens de provedor ou credenciais do relay em arquivos versionados.

## Validação executada

- Desktop React: build de produção e 3.909 testes aprovados.
- Web e administração: checks, typecheck e builds de produção aprovados.
- Mobile com Flutter 3.41.7/Dart 3.11.5: `flutter analyze` sem issues; 1.022 testes aprovados e 1 teste previamente ignorado.
- Rust/Tauri: `cargo check --release` sem erros; testes focados de configuração, modelo, pré-processamento e TTS aprovados.
- Voz pt-BR: hashes dos dois arquivos confirmados e teste funcional Piper → Whisper aprovado com os modelos reais.
- ACP: testes de idioma aprovados; eventos reais publicados por Codex e Claude Code no relay de produção.

O agregador `just ci` não fica verde nesta máquina Windows por dívida anterior do crate Tauri: o comando `cargo clippy --all-targets -- -D warnings` encontra 43 avisos/erros no `HEAD` em módulos Windows não relacionados (imports condicionais, código morto e lints antigos). Os lints introduzidos por M1–M7 foram eliminados; os builds release e as suítes diretamente afetadas estão verdes.
