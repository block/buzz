{{/*
Hard fail guards. Included from every rendered template so misconfigs
surface at template time regardless of which manifest helm renders first.
*/}}

{{- define "buzz.validate" -}}

{{/* relayUrl is required */}}
{{- if not .Values.relayUrl -}}
  {{- fail "relayUrl is required: set --set relayUrl=wss://your.domain" -}}
{{- end -}}

{{/* replicaCount > 1 requires Redis */}}
{{- if gt (.Values.replicaCount | int) 1 -}}
  {{- if and (not .Values.redis.enabled) (not .Values.externalRedis.url) (not .Values.secrets.existingSecret) -}}
    {{- fail (printf "replicaCount=%d requires Redis for buzz-pubsub. Enable redis.enabled=true, set externalRedis.url, or provide secrets.existingSecret with key REDIS_URL." (.Values.replicaCount | int)) -}}
  {{- end -}}
{{- end -}}

{{/* replicaCount > 1 requires ReadWriteMany git storage */}}
{{- if gt (.Values.replicaCount | int) 1 -}}
  {{- if and .Values.persistence.git.enabled (not .Values.persistence.git.existingClaim) -}}
    {{- if ne .Values.persistence.git.accessMode "ReadWriteMany" -}}
      {{- fail (printf "replicaCount=%d requires persistence.git.accessMode=ReadWriteMany (got %q). The relay's git on-disk state must be shared across replicas." (.Values.replicaCount | int) .Values.persistence.git.accessMode) -}}
    {{- end -}}
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

{{/* ingress + httproute mutually exclusive */}}
{{- if and .Values.ingress.enabled .Values.httproute.enabled -}}
  {{- fail "ingress.enabled and httproute.enabled cannot both be true — choose one." -}}
{{- end -}}

{{/* Postgres source must exist somewhere */}}
{{- if not (or .Values.postgresql.enabled .Values.externalPostgresql.url .Values.secrets.existingSecret) -}}
  {{- fail "Postgres source missing: enable postgresql.enabled=true, set externalPostgresql.url, or provide secrets.existingSecret with key DATABASE_URL." -}}
{{- end -}}

{{/* Typesense source must exist somewhere */}}
{{- if not (or .Values.typesense.enabled .Values.typesense.url .Values.secrets.existingSecret) -}}
  {{- fail "Typesense source missing: enable typesense.enabled=true (quickstart in-cluster), set typesense.url + typesense.apiKey, or provide secrets.existingSecret with keys TYPESENSE_URL + TYPESENSE_API_KEY." -}}
{{- end -}}

{{/* S3 / object-storage source must exist somewhere (relay hard-fails its
     startup conformance probe without a reachable bucket). */}}
{{- if not (or .Values.minio.enabled .Values.s3.endpoint .Values.secrets.existingSecret) -}}
  {{- fail "S3/object-storage source missing: enable minio.enabled=true (quickstart in-cluster), set s3.endpoint + s3.bucket + credentials, or provide secrets.existingSecret with keys BUZZ_S3_ACCESS_KEY + BUZZ_S3_SECRET_KEY. The relay runs a startup S3 conformance probe and exits if storage is unreachable." -}}
{{- end -}}

{{- end -}}
