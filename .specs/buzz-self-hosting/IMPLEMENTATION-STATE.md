# Implementation State: buzz-self-hosting

## Phase 1 — Bootstrap

- [x] Fork e remotes configurados
- [x] Plano e ADR documentados
- [ ] Pipeline de imagem validado — workflow de release ainda em execução
- [x] PR de bootstrap aberto e mergeado (#2)

## Phase 2 — Infraestrutura GitOps

- [x] ArgoCD repository preparado (#101, #102)
- [x] PostgreSQL 17 dedicado declarado
- [x] Redis 7.x dedicado declarado
- [ ] S3/prefixo dedicado — bucket ainda não provisionado
- [x] ExternalSecret declarado

## Phase 3 — Staging

- [x] Namespace e aplicação ArgoCD declarados (sync desativado)
- [ ] Ingress, DNS e TLS — manifests prontos; DNS e controladores estão pendentes
- [ ] Relay acessível por HTTPS/WSS
- [ ] Testes funcionais concluídos

## Phase 4 — Produção

- [ ] Backup e restore validados — CronJob declarado; restore ainda não executado
- [x] PRs de staging aprovados/mergeados
- [ ] Promoção para produção
- [ ] PR de produção aprovado/mergeado

## Bloqueios operacionais

- parâmetros `/buzz-staging/*` ainda não existem no AWS SSM;
- `external-secrets`, `cert-manager` e `external-dns` estão em `CreateContainerError`;
- `buzz-staging.lolmeida.com` ainda não resolve DNS;
- bucket S3 `buzz-staging-media` ainda não existe;
- a identidade Nostr do owner/relay ainda precisa de decisão e armazenamento no Vaultwarden;
- a imagem multi-arch ainda precisa de digest publicado no GHCR.
