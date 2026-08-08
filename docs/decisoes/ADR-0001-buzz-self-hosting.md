# ADR-0001 — Self-hosting do Buzz na VPS

- Estado: aceito
- Data: 2026-08-06
- Issue: https://github.com/lolmeida/buzz/issues/1
- Supersedido em deployment por: [ADR-0002](ADR-0002-buzz-direct-shared-services.md)

## Contexto

O Buzz será executado no MicroK8s da VPS e exposto por um subdomínio de
`lolmeida.com`. A VPS já possui PostgreSQL e Redis associados a outros serviços,
mas esses serviços não devem ser usados como dependências compartilhadas sem
isolamento explícito.

## Decisão

Usar deployment GitOps via ArgoCD, com uma instalação single-community inicial,
uma réplica, PostgreSQL 17 dedicado, Redis 7.x dedicado e S3-compatible com
bucket ou prefixo exclusivo. A aplicação será publicada inicialmente em staging
antes da promoção para produção.

Secrets serão entregues por Vaultwarden → AWS SSM → ExternalSecrets. O relay,
PostgreSQL, Redis e storage não serão expostos diretamente à Internet; apenas o
Ingress HTTPS/WSS será público.

O bucket `buzz-staging-media` usa bloqueio integral de acesso público,
`BucketOwnerEnforced`, SSE-S3, versionamento e expiração de versões não atuais
após 30 dias. Credenciais S3 dedicadas serão criadas apenas depois de o
Vaultwarden estar disponível e sincronizadas pelo fluxo Vaultwarden → SSM.

## Consequências

- A operação ganha isolamento entre o Buzz e os serviços existentes.
- A VPS single-node continua sendo um ponto único de falha.
- PostgreSQL e Redis dedicados aumentam consumo e responsabilidade de backup.
- A primeira instalação será limitada até serem validados agentes, workflows,
  webhooks, huddles e migrations.
- O versionamento do bucket permite recuperação operacional, mas exige política
  de retenção para limitar custo de versões antigas.
- GitHub Issue #1 substitui temporariamente o Jira como tracker operacional.
