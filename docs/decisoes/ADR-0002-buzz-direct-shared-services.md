# ADR-0002 — Deployment direto com serviços partilhados isolados

- Estado: aceito
- Data: 2026-08-06
- Issue: https://github.com/lolmeida/buzz/issues/1

## Contexto

A VPS tem um único nó de 4 vCPU. O PostgreSQL 17 e o Redis dedicados do
deployment intermédio não conseguem ser agendados com os requests dos
workloads existentes. O PostgreSQL 15.18 e o Redis 7 existentes em
`prod-database` estão operacionais.

## Decisão

Publicar diretamente em `buzz.lolmeida.com`, no namespace `buzz`, sem uma fase
de staging. Reutilizar os serviços de `prod-database` com isolamento lógico:

- database e role PostgreSQL exclusivos do Buzz;
- database lógico Redis exclusivo (`7`);
- NetworkPolicies explícitas entre o namespace `buzz` e os serviços partilhados;
- bucket S3 e credenciais IAM exclusivos;
- ExternalSecrets separados sob `/buzz/*`.

O bootstrap PostgreSQL é idempotente e executado pelo ArgoCD, que continua a
ser a fonte de verdade para Job, RBAC, NetworkPolicy, ExternalSecrets e
workloads. Operações controladas da base — healthcheck, ensure e migrations SQL
explicitamente escolhidas — são expostas pelo GHAR através de um runner ARC
dedicado com acesso à rede do cluster.

O bootstrap usa `/shared/postgres/PASSWORD` apenas para a administração e
`/buzz/POSTGRES_USER`, `/buzz/POSTGRES_PASSWORD` e `/buzz/POSTGRES_DB` para a
aplicação. O relay usa exclusivamente as credenciais da aplicação. O relay
continua sem dependência de serviços expostos à Internet; apenas HTTPS/WSS fica
público.

## Consequências

- Reduz consumo de CPU/memória e elimina PVCs de PostgreSQL/Redis do Buzz.
- PostgreSQL e Redis passam a partilhar disponibilidade e manutenção com a
  infraestrutura existente, mitigado por database/role, database lógico,
  NetworkPolicy e backup separado.
- PostgreSQL 15 requer validação das migrations e do restore antes de tratar a
  instalação como operacional.
- O runner GitHub-hosted não alcança `*.svc.cluster.local`; o workflow GHAR
  requer o scale set `arc-buzz-database` e OIDC restrito ao Environment
  `production`.
- A referência `@main` no consumidor é temporária para adopção imediata; deve
  voltar a `@v1` após a próxima release compatível do GHAR.
- O bucket anterior `buzz-staging-media` deixa de ser usado; o bucket direto
  tem bloqueio público, `BucketOwnerEnforced`, SSE-S3, versionamento e retenção
  de versões não atuais.
