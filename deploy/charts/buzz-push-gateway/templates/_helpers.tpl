{{- define "push.name" -}}{{ .Release.Name }}-buzz-push-gateway{{- end }}
{{- define "push.labels" -}}
app.kubernetes.io/name: buzz-push-gateway
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}
