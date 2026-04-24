{{/*
Expand the name of the chart.
*/}}
{{- define "stellar-operator.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "stellar-operator.fullname" -}}
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
{{- define "stellar-operator.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "stellar-operator.labels" -}}
helm.sh/chart: {{ include "stellar-operator.chart" . }}
{{ include "stellar-operator.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "stellar-operator.selectorLabels" -}}
app.kubernetes.io/name: {{ include "stellar-operator.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "stellar-operator.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "stellar-operator.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Compatibility aliases for Soroban RPC-oriented templates.
*/}}
{{- define "stellar-rpc.name" -}}
{{- include "stellar-operator.name" . -}}
{{- end }}

{{- define "stellar-rpc.fullname" -}}
{{- include "stellar-operator.fullname" . -}}
{{- end }}

{{- define "stellar-rpc.labels" -}}
{{- include "stellar-operator.labels" . -}}
{{- end }}

{{- define "stellar-rpc.selectorLabels" -}}
{{- include "stellar-operator.selectorLabels" . -}}
{{- end }}
