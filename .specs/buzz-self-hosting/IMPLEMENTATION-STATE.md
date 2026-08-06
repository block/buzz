# Implementation State: buzz-self-hosting

## Phase 1 — Bootstrap

- [x] Fork e remotes configurados
- [x] Plano e ADR documentados
- [x] Pipeline de chart validado (workflow `31128437223`)
- [ ] Pipeline de imagem própria — workflow `31128599898` ainda em execução
- [x] PR de bootstrap aberto e mergeado (#2)

## Phase 2 — Infraestrutura GitOps

- [x] ArgoCD repository preparado (#101, #102)
- [x] PostgreSQL 17 dedicado declarado
- [x] Redis 7.x dedicado declarado
- [x] Bucket S3 dedicado provisionado e endurecido; credenciais ainda pendentes
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
- credenciais S3 dedicadas ainda não foram criadas/armazenadas;
- `external-secrets`, `cert-manager` e `external-dns` estão em `CreateContainerError`;
- o ingress-nginx e o ArgoCD também estão afetados;
- `buzz-staging.lolmeida.com` ainda não resolve DNS;
- o Vaultwarden está inacessível pelo endpoint HTTPS;
- o containerd do MicroK8s reporta snapshots/mounts ausentes;
- a identidade Nostr do owner/relay ainda precisa de decisão e armazenamento no Vaultwarden;
- staging já está fixado no digest público imutável do relay oficial;
- o digest próprio do fork ainda depende do workflow `31128599898`.
