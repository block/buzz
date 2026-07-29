{{/*
Hard fail guards. Included from every rendered template so misconfigs
surface at template time regardless of which manifest helm renders first.
*/}}

{{- define "buzz.validate" -}}

{{/* relayUrl is required */}}
{{- if not .Values.relayUrl -}}
  {{- fail "relayUrl is required: set --set relayUrl=wss://your.domain" -}}
{{- end -}}

{{/* Multiple replicas do NOT require ReadWriteMany git storage.

     Git ref/object state is object-store-backed: every read and write hydrates
     an ephemeral bare repo from S3-compatible storage per request, and writer
     serialization is the object-store pointer CAS
     (docs/git-on-object-storage.md, Inv_NoFork). No persistent git state lives
     on the PVC, so replicas do not need a shared ReadWriteMany volume to agree
     on refs. Repo-name uniqueness — the last shared-state need — now lives in
     Postgres (git_repo_names), not on local disk.

     The prior hard-fail requiring persistence.git.accessMode=ReadWriteMany was
     removed here: its stated reason ("git on-disk state must be shared across
     replicas") is no longer true. Redis (validated below) remains a hard
     runtime dependency regardless of replica count — see that guard's
     comment for why. */}}

{{/* Autoscaling bounds must be coherent. */}}
{{- if .Values.autoscaling.enabled -}}
  {{- if lt (.Values.autoscaling.minReplicas | int) 1 -}}
    {{- fail "autoscaling.minReplicas must be at least 1" -}}
  {{- end -}}
  {{- if lt (.Values.autoscaling.maxReplicas | int) (.Values.autoscaling.minReplicas | int) -}}
    {{- fail "autoscaling.maxReplicas must be greater than or equal to autoscaling.minReplicas" -}}
  {{- end -}}
  {{- if and .Values.autoscaling.websocketMetricEnabled (not .Values.autoscaling.websocketMetricName) -}}
    {{- fail "autoscaling.websocketMetricName is required when WebSocket scaling is enabled" -}}
  {{- end -}}
{{- end -}}

{{/* Owner pubkey required when requireRelayMembership */}}
{{- if .Values.relay.requireRelayMembership -}}
  {{- if not .Values.ownerPubkey -}}
    {{- fail "ownerPubkey is required when relay.requireRelayMembership=true. Set ownerPubkey to the 64-char lowercase hex Nostr pubkey of the relay operator, or set relay.requireRelayMembership=false for an open relay." -}}
  {{- end -}}
{{- end -}}

{{/* ownerPubkey format check */}}
{{- if .Values.ownerPubkey -}}
  {{- if not (regexMatch "^[0-9a-f]{64}$" .Values.ownerPubkey) -}}
    {{- fail (printf "ownerPubkey must be 64 lowercase hex characters (got %d chars; must match ^[0-9a-f]{64}$)." (len .Values.ownerPubkey)) -}}
  {{- end -}}
{{- end -}}

{{/* Pairing relay deployment must have an advertised public URL. */}}
{{- if and .Values.pairingRelay.enabled (not .Values.pairingRelay.url) -}}
  {{- fail "pairingRelay.url is required when pairingRelay.enabled=true" -}}
{{- end -}}

{{/* ingress + httproute mutually exclusive */}}
{{- if and .Values.ingress.enabled .Values.httproute.enabled -}}
  {{- fail "ingress.enabled and httproute.enabled cannot both be true — choose one." -}}
{{- end -}}

{{/* Postgres source must exist somewhere */}}
{{- if not (or .Values.postgresql.enabled .Values.externalPostgresql.url .Values.secrets.existingSecret) -}}
  {{- fail "Postgres source missing: enable postgresql.enabled=true, set externalPostgresql.url, or provide secrets.existingSecret with key DATABASE_URL." -}}
{{- end -}}

{{/* S3 / object-storage source must exist somewhere (relay hard-fails its
     startup conformance probe without a reachable bucket). */}}
{{- if not (or .Values.minio.enabled .Values.s3.endpoint .Values.secrets.existingSecret) -}}
  {{- fail "S3/object-storage source missing: enable minio.enabled=true (quickstart in-cluster), set s3.endpoint + s3.bucket + credentials, or provide secrets.existingSecret with keys BUZZ_S3_ACCESS_KEY + BUZZ_S3_SECRET_KEY. The relay runs a startup S3 conformance probe and exits if storage is unreachable." -}}
{{- end -}}

{{/* Redis source must exist somewhere, at any replica count.
     This is not just an HA/fan-out concern: the relay unconditionally wires
     buzz-pubsub (presence, typing, rate limiting, cache invalidation) and the
     NIP-98 HTTP-auth replay guard through Redis (state.rs), and the replay
     guard fails closed — with no Redis it hard-fails startup or every bridge
     request. A single-replica install with no Redis source configured has
     always been unready in practice (redis://localhost:6379 default, nothing
     listening); this just makes that fail at `helm install` instead of a
     silent, permanent /_readiness 503 with no explanation in the logs. */}}
{{- if not (or .Values.redis.enabled .Values.externalRedis.url .Values.secrets.existingSecret) -}}
  {{- fail "Redis source missing: the relay requires Redis at any replica count (buzz-pubsub + NIP-98 replay protection). Enable redis.enabled=true, set externalRedis.url, or provide secrets.existingSecret with key REDIS_URL." -}}
{{- end -}}

{{- end -}}
