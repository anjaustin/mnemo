{{/*
Expand the name of the chart.
*/}}
{{- define "mnemo.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "mnemo.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "mnemo.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels.
*/}}
{{- define "mnemo.labels" -}}
helm.sh/chart: {{ include "mnemo.chart" . }}
{{ include "mnemo.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels.
*/}}
{{- define "mnemo.selectorLabels" -}}
app.kubernetes.io/name: {{ include "mnemo.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use.
*/}}
{{- define "mnemo.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "mnemo.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Redis URL: use subchart or external.
*/}}
{{- define "mnemo.redisUrl" -}}
{{- if .Values.redis.enabled }}
{{- printf "redis://%s-redis-master:6379" .Release.Name }}
{{- else }}
{{- .Values.mnemo.redisUrl }}
{{- end }}
{{- end }}

{{/*
Qdrant URL: use subchart or external.
*/}}
{{- define "mnemo.qdrantUrl" -}}
{{- if .Values.qdrant.enabled }}
{{- printf "http://%s-qdrant:6334" .Release.Name }}
{{- else }}
{{- .Values.mnemo.qdrantUrl }}
{{- end }}
{{- end }}

{{/*
Image reference.
*/}}
{{- define "mnemo.image" -}}
{{- printf "%s:%s" .Values.image.repository (default .Chart.AppVersion .Values.image.tag) }}
{{- end }}
