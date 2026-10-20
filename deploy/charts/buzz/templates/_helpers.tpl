{{/* Standard naming/labels helpers. */}}

{{- define "buzz.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "buzz.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{/* Kubernetes CronJob names are limited to 52 characters. */}}
{{- define "buzz.cronJobName" -}}
{{- $maxBaseLength := sub 51 (len .suffix) | int -}}
{{- $base := include "buzz.fullname" .root | trunc $maxBaseLength | trimSuffix "-" -}}
{{- printf "%s-%s" $base .suffix -}}
{{- end -}}

{{- define "buzz.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "buzz.labels" -}}
helm.sh/chart: {{ include "buzz.chart" . }}
{{ include "buzz.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: buzz
{{- end -}}

{{- define "buzz.selectorLabels" -}}
app.kubernetes.io/name: {{ include "buzz.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/* Relay-specific selector: scopes the relay Deployment + Service so they do
     not also match the quickstart MinIO pods, which share the base
     selectorLabels but carry their own component label. */}}
{{- define "buzz.relaySelectorLabels" -}}
{{ include "buzz.selectorLabels" . }}
app.kubernetes.io/component: relay
{{- end -}}

{{- define "buzz.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "buzz.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- define "buzz.image" -}}
{{- if .Values.image.digest -}}
{{- printf "%s@%s" .Values.image.repository .Values.image.digest -}}
{{- else -}}
{{- $tag := default .Chart.AppVersion .Values.image.tag -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
{{- end -}}
{{- end -}}

{{- define "buzz.imageRevision" -}}
{{- if .Values.image.digest -}}
{{- .Values.image.digest -}}
{{- else -}}
{{- default .Chart.AppVersion .Values.image.tag -}}
{{- end -}}
{{- end -}}

{{/*
Datadog unified-service-tagging version for every chart-managed Pod (relay,
pairing relay, storage accounting, operator jobs). It names the deployed image:
image.tag when set — the readable build name operators and dashboards already
use, which promotion tooling writes together with image.digest — otherwise the
digest, otherwise Chart.AppVersion. The chart owns this label so a wrapper never
has to pin a second copy of the image identity that silently drifts; the exact
runtime identity stays in the image reference and in
BUZZ_STORAGE_SNAPSHOT_CODE_SHA ("buzz.imageRevision").
*/}}
{{- define "buzz.imageVersionSource" -}}
{{- if .Values.image.tag -}}
{{- .Values.image.tag -}}
{{- else if .Values.image.digest -}}
{{- .Values.image.digest -}}
{{- else -}}
{{- .Chart.AppVersion -}}
{{- end -}}
{{- end -}}

{{/*
Kubernetes-label-safe rendering of "buzz.imageVersionSource".

A label value is at most 63 bytes, must begin and end with an alphanumeric,
and may otherwise contain only [-._a-zA-Z0-9]. image.tag is an unconstrained
string, so the domain is larger than the codomain and no mapping can be
injective; what the chart provides is a total mapping that is collision
*resistant*, in three cases:

  1. A "sha256:<64 hex>" digest drops its algorithm prefix and keeps 63 hex
     characters — the behaviour this chart has always had for digests.
  2. A revision that is already a valid label value *and* already in Datadog's
     normal form — no uppercase, no "__" — and is not of the reserved shape
     below, is emitted byte for byte, so ordinary tags like 1.2.3-rc.4 and
     sha-1a2b3c4 are preserved exactly.
  3. Anything else — a leading "_", an uppercase letter, a "__" run, a byte
     outside the label alphabet, or more than 63 bytes — is replaced by the
     first 63 hex characters of its SHA-256, the same shape and the same 252
     retained bits as case 1.

Case 2 is restricted to Datadog's normal form because the label's consumer is
Datadog, which lowercases tag values and collapses runs of "_". A passthrough
of "ReleaseA" would report the same version as "releasea", and "release__a"
the same as "release_a"; an uppercased 63-hex value would also slip past the
reserved-shape check below and then normalize onto a hashed or digest label.
Hashing everything not already in normal form means every value this helper
emits is a fixed point of Datadog's normalization, so distinct outputs stay
distinct where they are read.

Exactly 63 lowercase hex characters is the reserved shape: it is what cases 1
and 3 emit, so case 2 must not also be able to emit it. Without that exclusion
the two namespaces overlap and colliding two revisions costs no hash work at
all — render one, copy the label into image.tag, and the passthrough branch
echoes it back. Reserving the shape keeps every cross-case collision behind a
252-bit preimage, which is the same margin cases 1 and 3 already rely on. The
cost is that a tag which is itself 63 lowercase hex characters is hashed rather
than preserved; such a tag is not a form anything generates except this label.

Case 3 deliberately keeps no readable prefix. An earlier revision of this
helper kept a sanitized prefix plus 10 hex characters of SHA-256, and that
40-bit namespace was small enough to brute-force: the tags
"b"x52 + "-collision-22356" and "b"x52 + "-collision-914141" both rendered
"b"x52 + "-430be57931" (see tests/storage_accounting_test.yaml). Readability
is not worth a forgeable telemetry identity when the exact revision is always
available in the image reference itself.
*/}}
{{- define "buzz.imageVersionLabel" -}}
{{- $revision := include "buzz.imageVersionSource" . -}}
{{- if regexMatch "^sha256:[0-9a-f]{64}$" $revision -}}
{{- $revision | trimPrefix "sha256:" | trunc 63 -}}
{{- else if and (le (len $revision) 63) (regexMatch "^[a-z0-9]([-._a-z0-9]*[a-z0-9])?$" $revision) (not (contains "__" $revision)) (not (regexMatch "^[0-9a-f]{63}$" $revision)) -}}
{{- $revision -}}
{{- else -}}
{{- sha256sum $revision | trunc 63 -}}
{{- end -}}
{{- end -}}

{{/*
Pod-template labels for a chart-managed workload: the caller's podLabels with
the chart-owned Datadog version merged on top. Every other key, including
tags.datadoghq.com/env and tags.datadoghq.com/service, passes through.
Usage: include "buzz.podLabelsWithVersion" (dict "root" $ "labels" $podLabels)
*/}}
{{- define "buzz.podLabelsWithVersion" -}}
{{- $labels := merge (dict "tags.datadoghq.com/version" (include "buzz.imageVersionLabel" .root)) (.labels | default dict) -}}
{{- toYaml $labels -}}
{{- end -}}

{{/*
Name of the chart-managed Secret holding relay-identity material and any
chart-composed connection strings.
*/}}
{{- define "buzz.chartSecretName" -}}
{{- printf "%s-relay" (include "buzz.fullname" .) -}}
{{- end -}}

{{/*
The Secret name the relay should pull env from. If the operator supplied
secrets.existingSecret, use that. Otherwise use the chart-managed one.
*/}}
{{- define "buzz.envSecretName" -}}
{{- if .Values.secrets.existingSecret -}}
{{- .Values.secrets.existingSecret -}}
{{- else -}}
{{- include "buzz.chartSecretName" . -}}
{{- end -}}
{{- end -}}

{{/* Host derived from relayUrl, used as ingress default + media domain. */}}
{{- define "buzz.relayHost" -}}
{{- $url := required "relayUrl is required: set --set relayUrl=wss://your.domain" .Values.relayUrl -}}
{{- $stripped := $url | replace "wss://" "" | replace "ws://" "" | replace "https://" "" | replace "http://" "" -}}
{{- first (splitList "/" $stripped) -}}
{{- end -}}

{{/* Default media base URL: https://<host>/media derived from relayUrl. */}}
{{- define "buzz.mediaBaseUrl" -}}
{{- if .Values.mediaBaseUrl -}}
{{- .Values.mediaBaseUrl -}}
{{- else -}}
{{- printf "https://%s/media" (include "buzz.relayHost" .) -}}
{{- end -}}
{{- end -}}

{{/* Quickstart-only in-cluster service hostnames (eval profile). */}}
{{- define "buzz.minioFullname" -}}
{{- printf "%s-minio" (include "buzz.fullname" .) -}}
{{- end -}}

{{/* In-cluster MinIO endpoint, used when minio.enabled and s3.endpoint unset. */}}
{{- define "buzz.minioEndpoint" -}}
{{- printf "http://%s.%s.svc.cluster.local:9000" (include "buzz.minioFullname" .) .Release.Namespace -}}
{{- end -}}

{{/* Minimum number of relay replicas the release can run. */}}
{{- define "buzz.minimumReplicas" -}}
{{- if .Values.autoscaling.enabled -}}
{{- .Values.autoscaling.minReplicas -}}
{{- else -}}
{{- .Values.replicaCount -}}
{{- end -}}
{{- end -}}

{{/* Effective huddle-audio availability. Nil means safe chart default: on for
     one replica, off for multi-pod until an SFU/shared-room story exists. */}}
{{- define "buzz.huddleAudioAvailable" -}}
{{- if kindIs "invalid" .Values.relay.huddleAudioAvailable -}}
{{- if gt (include "buzz.minimumReplicas" . | int) 1 -}}false{{- else -}}true{{- end -}}
{{- else -}}
{{- .Values.relay.huddleAudioAvailable -}}
{{- end -}}
{{- end -}}

{{/* Effective S3 endpoint: explicit s3.endpoint wins, else bundled MinIO. */}}
{{- define "buzz.s3Endpoint" -}}
{{- if .Values.s3.endpoint -}}
{{- .Values.s3.endpoint -}}
{{- else if .Values.minio.enabled -}}
{{- include "buzz.minioEndpoint" . -}}
{{- end -}}
{{- end -}}

{{- define "buzz.pairingRelaySelectorLabels" -}}
{{ include "buzz.selectorLabels" . }}
app.kubernetes.io/component: pairing-relay
{{- end -}}
