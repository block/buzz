# Buzz self-hosting na VPS

## Rastreadibilidade

- Tracker: GitHub Issue [#1](https://github.com/lolmeida/buzz/issues/1).
- Jira está temporariamente indisponível; esta issue é a fonte operacional de rastreabilidade.
- Upstream: `block/buzz`.
- Fork: `lolmeida/buzz`.
- Deployment: `lolmeida/argo-cd`.

## Objetivo

Publicar diretamente o Buzz em `buzz.lolmeida.com`, usando MicroK8s, ArgoCD,
PostgreSQL/Redis partilhados com database/role e database lógico exclusivos, e
S3-compatible exclusivo.

## Decisões

- Single-community e uma réplica no primeiro release.
- PostgreSQL 15.18 e Redis 7 existentes são reutilizados somente com isolamento
  lógico explícito; migrations e restore têm de ser validados.
- O namespace `buzz` é a única instalação pública; não há promoção staging → produção.
- Não expor PostgreSQL, Redis, MinIO, Adminer ou métricas à Internet.
- Secrets: Vaultwarden → AWS SSM → ExternalSecrets → Kubernetes Secret.
- Imagem fixada por digest; nunca `main` ou `latest` em produção.
- Bootstrap admin/app: ArgoCD lê `/shared/postgres/PASSWORD` para o role
  administrativo e `/buzz/POSTGRES_*` para o role da aplicação; o runtime usa
  somente a credencial da aplicação.
- Operações manuais da base: workflow GHAR através do runner ARC
  `arc-buzz-database`, com OIDC/SSM e sem criação de recursos Kubernetes.
- Agentes privilegiados, workflows, webhooks e huddles ficam desativados inicialmente.

## Fases

1. Fork, branch, pipeline de imagem e documentação.
2. S3, database/role PostgreSQL, database lógico Redis e secrets dedicados.
3. ArgoCD, Ingress, TLS e DNS diretos.
4. Bootstrap PostgreSQL via ArgoCD com credenciais admin/app separadas e
   operações controladas via GHAR/runner ARC.
5. Testes de migrations, runtime, segurança, backup e restore.
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
- GHAR verifica a conectividade com `sslmode=require` e não oferece DROP/reset.

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
- A migração GHAR executa apenas o ficheiro SQL explicitamente selecionado; a
  migração automática embutida do relay continua a ser a fonte de migrações
  SQLx completas.
