# Implementation State: buzz-self-hosting

## Phase 1 — Bootstrap

- [x] Fork e remotes configurados
- [x] Plano e ADR documentados
- [x] Pipeline de chart validado (workflow `31128437223`)
- [ ] Pipeline de imagem própria — workflow `31128599898` ainda em execução
- [x] PR de bootstrap aberto e mergeado (#2)

## Phase 2 — Infraestrutura GitOps

- [x] ArgoCD repository preparado (#101, #102)
- [x] PostgreSQL 15.18 e Redis 7 existentes verificados
- [x] Database/role PostgreSQL e database lógico Redis dedicados definidos
- [x] Bucket S3 direto provisionado e endurecido
- [x] ExternalSecrets `/buzz/*` sincronizados

## Phase 3 — Deployment direto

- [x] Namespace `buzz` e aplicação ArgoCD declarados com auto-sync
- [ ] Ingress, DNS e TLS — manifests prontos; validação final pendente
- [ ] Relay acessível por HTTPS/WSS
- [ ] Testes funcionais concluídos

## Operações da base

- [x] Bootstrap ArgoCD com credencial admin `/shared/postgres/PASSWORD` e
  credenciais de aplicação `/buzz/POSTGRES_*`
- [x] GHAR dual-credential workflow mergeado (`lolmeida/github-actions#36`)
- [x] Runner ARC dedicado `arc-buzz-database` declarado e acesso de rede
  permitido
- [ ] Workflow Buzz GHAR mergeado e healthcheck/ensure executados

## Phase 4 — Validação operacional

- [ ] Backup e restore validados — CronJob declarado; restore ainda não executado
- [ ] Migrations PostgreSQL 15 validadas
- [ ] Teste funcional Nostr/WebSocket/media concluído
- [ ] PR direto aprovado/mergeado

## Bloqueios operacionais

- a validação de migrations/restore em PostgreSQL 15 ainda está pendente;
- o digest próprio do fork ainda depende do workflow `31128599898`;
- a validação pública de `buzz.lolmeida.com` ainda está pendente.
- o workflow operacional do Buzz ainda está no PR #12.
