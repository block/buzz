# Buzz self-hosting na VPS

## Rastreadibilidade

- Tracker: GitHub Issue [#1](https://github.com/lolmeida/buzz/issues/1).
- Jira está temporariamente indisponível; esta issue é a fonte operacional de rastreabilidade.
- Upstream: `block/buzz`.
- Fork: `lolmeida/buzz`.
- Deployment: `lolmeida/argo-cd`.

## Objetivo

Publicar o Buzz primeiro em `buzz-staging.lolmeida.com` e depois em
`buzz.lolmeida.com`, usando MicroK8s, ArgoCD, PostgreSQL, Redis e S3-compatible
isolados.

## Decisões

- Single-community e uma réplica no primeiro release.
- PostgreSQL 17 é o baseline de produção.
- PostgreSQL 16 só pode ser aceite após testes completos de migrations e restore.
- Não reutilizar databases, roles ou Redis de outros serviços.
- Não expor PostgreSQL, Redis, MinIO, Adminer ou métricas à Internet.
- Secrets: Vaultwarden → AWS SSM → ExternalSecrets → Kubernetes Secret.
- Imagem fixada por digest; nunca `main` ou `latest` em produção.
- Agentes privilegiados, workflows, webhooks e huddles ficam desativados inicialmente.

## Fases

1. Fork, branch, pipeline de imagem e documentação.
2. PostgreSQL 17, Redis 7.x, S3/prefixo e secrets dedicados.
3. ArgoCD, Ingress, TLS, DNS e staging.
4. Testes de migrations, runtime, segurança, backup e restore.
5. Promoção controlada para produção.
6. Ativação gradual de agentes, workflows, webhooks e HA.

## Critérios de aceitação

- ArgoCD `Synced` e `Healthy`.
- HTTPS e WSS funcionais.
- Membership fechado validado.
- Mensagens, pesquisa, media e Git funcionais.
- PostgreSQL e Redis não acessíveis externamente.
- Nenhum secret no Git, manifests renderizados ou logs.
- Backup e restore demonstrados.
- Imagem e chart fixados em versões imutáveis.
- Rollback para o digest anterior validado.

## Rollback

- Reverter o commit GitOps e aguardar reconciliação do ArgoCD.
- Reverter imagem para digest anterior.
- Não remover PVCs, buckets ou secrets automaticamente.
- Migration incompatível exige restore validado e correção no Git.

## Riscos conhecidos

- VPS single-node não fornece HA real.
- O relay é parte da fronteira de confiança das DMs.
- Approval gates e algumas ações de workflow estão incompletos upstream.
- Redis é necessário para fanout e coordenação distribuída.
- A especificação de multi-tenancy upstream ainda é draft.
