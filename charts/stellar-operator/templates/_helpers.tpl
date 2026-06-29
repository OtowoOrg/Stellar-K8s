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
Primary operator container image (repository:tag).
Tag defaults to Chart.appVersion when image.tag is empty.
*/}}
{{- define "stellar-operator.operatorImage" -}}
{{- printf "%s:%s" .Values.image.repository (.Values.image.tag | default .Chart.AppVersion) -}}
{{- end }}

{{/*
Resolve a component image from optional overrides, falling back to the operator image.
Usage: include "stellar-operator.componentImage" (dict "root" . "repository" .Values.forkDetector.image.repository "tag" .Values.forkDetector.image.tag)
*/}}
{{- define "stellar-operator.componentImage" -}}
{{- $root := .root -}}
{{- $repo := .repository | default $root.Values.image.repository -}}
{{- $tag := .tag | default $root.Values.image.tag | default $root.Chart.AppVersion -}}
{{- printf "%s:%s" $repo $tag -}}
{{- end }}

{{/*
Common labels with an app.kubernetes.io/component label.
Usage: include "stellar-operator.labelsWithComponent" (dict "root" . "component" "fork-detector")
*/}}
{{- define "stellar-operator.labelsWithComponent" -}}
{{- include "stellar-operator.labels" .root }}
app.kubernetes.io/component: {{ .component }}
{{- end }}

{{/*
Selector labels with an app.kubernetes.io/component label.
Usage: include "stellar-operator.selectorLabelsWithComponent" (dict "root" . "component" "fork-detector")
*/}}
{{- define "stellar-operator.selectorLabelsWithComponent" -}}
{{- include "stellar-operator.selectorLabels" .root }}
app.kubernetes.io/component: {{ .component }}
{{- end }}

{{/*
Prometheus scrape annotations for sidecar and watcher pods.
Usage: include "stellar-operator.prometheusAnnotations" (dict "metricsPort" .Values.forkDetector.metricsPort)
*/}}
{{- define "stellar-operator.prometheusAnnotations" -}}
prometheus.io/scrape: "true"
prometheus.io/port: {{ .metricsPort | quote }}
prometheus.io/path: {{ .path | default "/metrics" | quote }}
{{- end }}

{{/*
Compatibility aliases for legacy Soroban RPC-oriented templates.
Prefer stellar-operator.* helpers in new templates.
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

