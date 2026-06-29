{{/*
Shared Helm hook Job skeleton used by pre-install and pre-upgrade hooks.

Parameters (dict):
  root          — Helm root context (.)
  name          — short hook name suffix (e.g. "pre-install")
  hook          — Helm hook phase (pre-install | pre-upgrade)
  component     — app.kubernetes.io/component label value
  containerName — primary container name
  script        — shell script body (without leading indent)
*/}}
{{- define "stellar-operator.hookJob" -}}
{{- $root := .root -}}
apiVersion: batch/v1
kind: Job
metadata:
  name: {{ include "stellar-operator.fullname" $root }}-{{ .name }}
  namespace: {{ $root.Release.Namespace }}
  labels:
    {{- include "stellar-operator.labelsWithComponent" (dict "root" $root "component" .component) | nindent 4 }}
  annotations:
    "helm.sh/hook": {{ .hook }}
    "helm.sh/hook-weight": "-5"
    "helm.sh/hook-delete-policy": before-hook-creation,hook-succeeded
spec:
  ttlSecondsAfterFinished: 300
  backoffLimit: 3
  template:
    metadata:
      name: {{ include "stellar-operator.fullname" $root }}-{{ .name }}
      labels:
        {{- include "stellar-operator.selectorLabelsWithComponent" (dict "root" $root "component" .component) | nindent 8 }}
    spec:
      restartPolicy: Never
      serviceAccountName: {{ include "stellar-operator.serviceAccountName" $root }}
      securityContext:
        {{- toYaml $root.Values.podSecurityContext | nindent 8 }}
      containers:
      - name: {{ .containerName }}
        image: "{{ include "stellar-operator.operatorImage" $root }}"
        imagePullPolicy: {{ $root.Values.image.pullPolicy }}
        securityContext:
          {{- toYaml $root.Values.securityContext | nindent 10 }}
        command:
        - /bin/sh
        - -c
        - |
{{ .script | nindent 10 }}
{{- end }}

{{- define "stellar-operator.preInstallScript" -}}
set -e
echo "=== Stellar Operator Pre-Install Checks ==="

# Check Kubernetes version
echo "Checking Kubernetes version..."
K8S_VERSION=$(kubectl version --short 2>/dev/null | grep Server | awk '{print $3}' | sed 's/v//')
K8S_MAJOR=$(echo $K8S_VERSION | cut -d. -f1)
K8S_MINOR=$(echo $K8S_VERSION | cut -d. -f2)

if [ "$K8S_MAJOR" -lt 1 ] || ([ "$K8S_MAJOR" -eq 1 ] && [ "$K8S_MINOR" -lt 21 ]); then
  echo "ERROR: Kubernetes version must be 1.21 or higher (found: $K8S_VERSION)"
  exit 1
fi
echo "✓ Kubernetes version $K8S_VERSION is supported"

# Check for required CRDs
echo "Checking for existing CRDs..."
if kubectl get crd stellarnodes.stellar.org 2>/dev/null; then
  echo "⚠ WARNING: StellarNode CRD already exists. This may be an upgrade."
fi

# Check namespace labels for network isolation
{{- if .Values.networkIsolation.enabled }}
echo "Checking namespace network labels..."
NAMESPACE_LABEL=$(kubectl get namespace {{ .Release.Namespace }} -o jsonpath='{.metadata.labels.stellar\.org/network}' 2>/dev/null || echo "")
if [ -z "$NAMESPACE_LABEL" ]; then
  echo "⚠ WARNING: Namespace {{ .Release.Namespace }} is not labeled with stellar.org/network"
  echo "  Network isolation is enabled but namespace is not labeled."
  {{- if .Values.networkIsolation.labelReleaseNamespace }}
  echo "  Will apply label: stellar.org/network={{ .Values.networkIsolation.releaseNamespaceNetwork }}"
  kubectl label namespace {{ .Release.Namespace }} stellar.org/network={{ .Values.networkIsolation.releaseNamespaceNetwork }} --overwrite
  {{- end }}
else
  echo "✓ Namespace labeled with network: $NAMESPACE_LABEL"
fi
{{- end }}

# Check for Prometheus if metrics are enabled
{{- if .Values.podAnnotations }}
{{- if index .Values.podAnnotations "prometheus.io/scrape" }}
echo "Checking for Prometheus..."
if kubectl get servicemonitors.monitoring.coreos.com 2>/dev/null >/dev/null; then
  echo "✓ Prometheus Operator CRDs found"
else
  echo "⚠ WARNING: Prometheus Operator CRDs not found. Metrics may not be scraped."
fi
{{- end }}
{{- end }}

# Check storage classes for PVC support
echo "Checking storage classes..."
if kubectl get storageclass 2>/dev/null | grep -q "(default)"; then
  DEFAULT_SC=$(kubectl get storageclass -o jsonpath='{.items[?(@.metadata.annotations.storageclass\.kubernetes\.io/is-default-class=="true")].metadata.name}')
  echo "✓ Default storage class found: $DEFAULT_SC"
else
  echo "⚠ WARNING: No default storage class found. StellarNodes may fail to provision PVCs."
fi

# Check for admission webhook prerequisites
{{- if .Values.security.admissionWebhook }}
echo "Checking admission webhook prerequisites..."
if kubectl get validatingwebhookconfigurations 2>/dev/null >/dev/null; then
  echo "✓ Admission webhook API available"
else
  echo "ERROR: Admission webhook API not available"
  exit 1
fi
{{- end }}

# Validate resource limits
echo "Validating resource configuration..."
{{- if .Values.resources.limits.memory }}
MEMORY_LIMIT="{{ .Values.resources.limits.memory }}"
echo "  Operator memory limit: $MEMORY_LIMIT"
{{- end }}
{{- if .Values.resources.limits.cpu }}
CPU_LIMIT="{{ .Values.resources.limits.cpu }}"
echo "  Operator CPU limit: $CPU_LIMIT"
{{- end }}

echo ""
echo "=== Pre-Install Checks Complete ==="
echo "✓ All critical checks passed"
echo ""
{{- end }}

{{- define "stellar-operator.preUpgradeScript" -}}
set -e
echo "=== Stellar Operator Pre-Upgrade Checks ==="

# Backup current CRD definitions
echo "Backing up current CRD definitions..."
kubectl get crd stellarnodes.stellar.org -o yaml > /tmp/stellarnode-crd-backup.yaml 2>/dev/null || true

# Check for running StellarNodes
echo "Checking for active StellarNode resources..."
ACTIVE_NODES=$(kubectl get stellarnodes --all-namespaces --no-headers 2>/dev/null | wc -l)
if [ "$ACTIVE_NODES" -gt 0 ]; then
  echo "⚠ WARNING: Found $ACTIVE_NODES active StellarNode(s)"
  echo "  Upgrade will proceed but nodes may experience brief disruption"
  kubectl get stellarnodes --all-namespaces
else
  echo "✓ No active StellarNodes found"
fi

# Check operator deployment status
echo "Checking current operator deployment..."
if kubectl get deployment {{ include "stellar-operator.fullname" . }} -n {{ .Release.Namespace }} 2>/dev/null; then
  READY_REPLICAS=$(kubectl get deployment {{ include "stellar-operator.fullname" . }} -n {{ .Release.Namespace }} -o jsonpath='{.status.readyReplicas}' 2>/dev/null || echo "0")
  DESIRED_REPLICAS=$(kubectl get deployment {{ include "stellar-operator.fullname" . }} -n {{ .Release.Namespace }} -o jsonpath='{.spec.replicas}' 2>/dev/null || echo "0")
  echo "  Current replicas: $READY_REPLICAS/$DESIRED_REPLICAS"

  if [ "$READY_REPLICAS" != "$DESIRED_REPLICAS" ]; then
    echo "⚠ WARNING: Operator is not fully ready before upgrade"
  fi
fi

# Check for breaking changes in CRD schema
echo "Validating CRD compatibility..."
CURRENT_VERSION=$(kubectl get crd stellarnodes.stellar.org -o jsonpath='{.spec.versions[0].name}' 2>/dev/null || echo "unknown")
echo "  Current CRD version: $CURRENT_VERSION"

# Validate that no nodes are in critical state
echo "Checking node health status..."
CRITICAL_NODES=$(kubectl get stellarnodes --all-namespaces -o json 2>/dev/null | \
  jq -r '.items[] | select(.status.phase == "Failed" or .status.phase == "Error") | .metadata.name' | wc -l)

if [ "$CRITICAL_NODES" -gt 0 ]; then
  echo "⚠ WARNING: $CRITICAL_NODES node(s) in Failed/Error state"
  echo "  Consider resolving these issues before upgrading"
fi

# Check for pending PVCs
echo "Checking for pending PVCs..."
PENDING_PVCS=$(kubectl get pvc --all-namespaces --field-selector=status.phase=Pending 2>/dev/null | grep stellar | wc -l || echo "0")
if [ "$PENDING_PVCS" -gt 0 ]; then
  echo "⚠ WARNING: $PENDING_PVCS pending PVC(s) found"
fi

# Verify rollback capability
{{- if .Values.hooks.preUpgrade.enableRollbackCheck }}
echo "Verifying rollback capability..."
HELM_HISTORY=$(helm history {{ .Release.Name }} -n {{ .Release.Namespace }} --max 1 -o json 2>/dev/null || echo "[]")
if [ "$HELM_HISTORY" != "[]" ]; then
  echo "✓ Previous release found - rollback is possible"
else
  echo "⚠ WARNING: No previous release found - rollback may not be possible"
fi
{{- end }}

# Create upgrade checkpoint
echo "Creating upgrade checkpoint..."
kubectl create configmap {{ include "stellar-operator.fullname" . }}-upgrade-checkpoint \
  --from-literal=timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --from-literal=from-version="{{ .Chart.Version }}" \
  --from-literal=active-nodes="$ACTIVE_NODES" \
  -n {{ .Release.Namespace }} \
  --dry-run=client -o yaml | kubectl apply -f -

echo ""
echo "=== Pre-Upgrade Checks Complete ==="
echo "✓ Ready to proceed with upgrade"
echo ""
{{- end }}
