#!/usr/bin/env bash
# Copyright 2024 Stellar-K8s Contributors
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
# scripts/prune-registry-images.sh — prune stale versions of a GHCR container package.
#
# Lists the versions of one GitHub Container Registry package (by default the
# operator image ghcr.io/otowoorg/stellar-k8s), decides which are safe to remove
# and prints the plan. Nothing is deleted unless --delete is passed.
#
# Usage:
#   scripts/prune-registry-images.sh [OPTIONS]            # dry run (the default)
#   scripts/prune-registry-images.sh --delete [OPTIONS]   # print the plan, then delete it
#
# Options:
#   --owner NAME           package owner (default: owner of $GITHUB_REPOSITORY, else OtowoOrg)
#   --package NAME         package name (default: lowercased repo name, else stellar-k8s)
#   --owner-type TYPE      org or user (default: org)
#   --min-age-days N       only versions older than N days may be deleted (default: 30, minimum 1)
#   --keep-latest N        always keep the N newest versions (default: 10)
#   --protect-tag REGEX    additional tag regex to protect (repeatable)
#   --max-delete N         refuse to delete more than N versions in one run (default: 100)
#   --repo-root DIR        repository to scan for referenced tags (default: this repository)
#   --now EPOCH            reference time in epoch seconds (default: now)
#   --versions-file FILE   read versions from a JSON file in the Packages API shape
#                          instead of calling the API (offline; cannot be combined with --delete)
#   --manifests-file FILE  JSON object mapping an index digest to its child digests
#                          (offline stand-in for the registry lookup; needs --versions-file)
#   -h, --help             show this help
#
# A version is KEPT when any of these hold:
#   - one of its tags is protected: latest, main/master, a release tag
#     (X.Y.Z, vX.Y.Z, X.Y.Z-rc.1, X.Y, X) or anything given with --protect-tag
#   - a tag or digest of it is referenced by the repo (chart appVersion, chart
#     values, manifests, kustomize images under charts/, config/ and examples/)
#   - it is one of the --keep-latest newest versions
#   - it is younger than --min-age-days
#   - it is a child manifest of a version that is kept (multi-arch images and
#     attestations are untagged versions that belong to a tagged index)
#   - it is a cosign signature/attestation (sha256-<digest>[.sig|.att|.sbom])
#     of a version that is kept
# Everything else (stale per-commit tags and orphaned untagged versions) is
# planned for deletion. Selection is pure data in, data out, so it can be
# checked offline with --versions-file.
#
# Live runs need `gh` and `jq`, and a token that can read (and with --delete,
# delete) the package: GH_TOKEN or GITHUB_TOKEN with packages: write in Actions.
# Exit codes: 0 plan produced / versions deleted, 1 runtime error, 2 bad usage,
# 3 plan exceeds --max-delete (nothing deleted).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEFAULT_REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

REGISTRY_HOST="ghcr.io"
tmp=""
MANIFEST_ACCEPT="application/vnd.oci.image.index.v1+json,application/vnd.docker.distribution.manifest.list.v2+json,application/vnd.oci.image.manifest.v1+json,application/vnd.docker.distribution.manifest.v2+json"

# Tags that are never pruned: moving pointers and release tags produced by the
# release pipeline (docker/metadata-action semver patterns in release.yml).
DEFAULT_PROTECT_REGEXES=(
  '^latest$'
  '^(main|master)$'
  '^v?[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$'
  '^v?[0-9]+(\.[0-9]+)?$'
)

die() { echo "error: $*" >&2; exit 1; }
usage_error() { echo "error: $*" >&2; echo "Run with --help for usage." >&2; exit 2; }
usage() {
  awk '/^# Usage:/ {p=1} p && /^[^#]/ {exit} p {sub(/^# ?/, ""); print}' "${BASH_SOURCE[0]}"
}

# ── Selection logic (pure: JSON in, JSON out) ────────────────────────────────

# Reads a JSON array of normalized versions {id, digest, created, tags} on stdin
# and prints the same versions with `action` (keep|delete), `reason`, `age_days`
# and `subject` (digest a cosign attachment belongs to, else null).
# shellcheck disable=SC2016  # jq variables, not shell expansions
PLAN_JQ='
def attachment_tag: test("^sha256-[0-9a-f]{64}(\\.(sig|att|sbom))?$");
def subject_of:
  if (.tags | length) > 0 and (.tags | all(attachment_tag))
  then "sha256:" + (.tags[0] | capture("^sha256-(?<h>[0-9a-f]{64})").h)
  else null end;
def closure($seed):
  ($seed | unique) as $s
  | ($s + [$s[] | ($children[.] // [])[]] | unique) as $n
  | if ($n | length) == ($s | length) then $s else closure($n) end;

map(. + {subject: subject_of, age_days: (($now - (.created | fromdateiso8601)) / 86400 | floor)}) as $vs
| ($vs | map(select(.subject == null)) | sort_by(.created) | reverse | .[0:$keepLatest] | map(.id)) as $recent
| ($vs | map(select(.subject == null) | . as $v
    | (if ($v.tags | any(. as $t | $protectRegexes | any(. as $r | $t | test($r)))) then "protected-tag"
       elif ($v.digest | IN($refDigests[])) then "referenced-digest"
       elif ($v.tags | any(. as $t | $t | IN($refTags[]))) then "referenced-in-repo"
       elif ($v.id | IN($recent[])) then "recent"
       elif $v.age_days < $minAgeDays then "too-young"
       else null end) as $why
    | select($why != null) | {digest: $v.digest, why: $why})) as $seed
| closure($seed | map(.digest)) as $kept
| ($vs | map(.digest)) as $all
| $vs | map(
    . as $v
    | if $v.subject != null then
        if ($v.subject | IN($kept[])) then $v + {action: "keep", reason: "attached-to-kept-image"}
        elif ($v.subject | IN($all[])) then $v + {action: "delete", reason: "attached-to-pruned-image"}
        else $v + {action: "keep", reason: "attachment-subject-unknown"} end
      elif ($v.digest | IN($kept[])) then
        $v + {action: "keep", reason: (([$seed[] | select(.digest == $v.digest) | .why] | first) // "child-of-kept-image")}
      else
        $v + {action: "delete", reason: (if ($v.tags | length) == 0 then "stale-untagged" else "stale-tag" end)}
      end)
| sort_by(.created) | reverse'

# select_versions <versions.json> <now> <min-age> <keep-latest> <children.json>
#                 <protect-regexes.json> <ref-tags.json> <ref-digests.json>
select_versions() {
  jq --argjson now "$2" --argjson minAgeDays "$3" --argjson keepLatest "$4" \
     --argjson children "$5" --argjson protectRegexes "$6" \
     --argjson refTags "$7" --argjson refDigests "$8" \
     "${PLAN_JQ}" "$1"
}

# normalize_versions: Packages API response (array of arrays or array) -> {id,digest,created,tags}
normalize_versions() {
  jq -s 'add // [] | map({id, digest: .name, created: .created_at, tags: (.metadata.container.tags // [])})'
}

# ── Repo references ──────────────────────────────────────────────────────────

# scan_references <repo-root> <package> -> JSON {"tags": [...], "digests": [...]}
# Collects tags and digests of <package> that the repo itself deploys or pins.
scan_references() {
  local root="$1" pkg_base="${2##*/}" dirs=() d files
  for d in charts config examples; do
    [[ -d "${root}/${d}" ]] && dirs+=("${root}/${d}")
  done
  if [[ ${#dirs[@]} -eq 0 ]]; then
    echo '{"tags":[],"digests":[]}'
    return 0
  fi
  files="$(find "${dirs[@]}" -type f \( -name '*.yaml' -o -name '*.yml' \) \
    -not -path '*/tests/*' -not -path '*/node_modules/*' | sort)"
  {
    # The chart deploys appVersion when image.tag is empty.
    while IFS= read -r f; do
      [[ -n "${f}" && "$(basename "${f}")" == "Chart.yaml" ]] || continue
      awk '/^appVersion:/ { gsub(/["\047 ]/, "", $2); if ($2 != "") { print "tag\t" $2; if ($2 !~ /^v/) print "tag\tv" $2 } }' "${f}"
    done <<<"${files}"
    # <registry>/<owner>/<package>:<tag> and ...@sha256:<digest> references.
    while IFS= read -r f; do
      [[ -n "${f}" ]] || continue
      grep -hoE "[A-Za-z0-9._/-]*/${pkg_base}(:[A-Za-z0-9._-]+|@sha256:[0-9a-f]{64})" "${f}" 2>/dev/null \
        | sed -E "s#^.*/${pkg_base}:#tag\t#; s#^.*/${pkg_base}@#digest\t#" || true
    done <<<"${files}"
    # Split forms: `repository: .../<package>` then `tag: x`, or kustomize
    # `name`/`newName: .../<package>` then `newTag: x`.
    while IFS= read -r f; do
      [[ -n "${f}" ]] || continue
      awk -v pkg="${pkg_base}" '
        $0 ~ "(repository|name|newName|image):[[:space:]]*[\"\047]?[^#]*" pkg "[\"\047]?[[:space:]]*(#.*)?$" { n = 5; next }
        n > 0 {
          n--
          if (match($0, /^[[:space:]]*(tag|newTag):[[:space:]]*["\047]?[A-Za-z0-9._-]+/)) {
            v = substr($0, RSTART, RLENGTH); sub(/^[[:space:]]*(tag|newTag):[[:space:]]*["\047]?/, "", v)
            print "tag\t" v
          }
        }' "${f}"
    done <<<"${files}"
  } | jq -Rn '[inputs | split("\t") | select(length == 2)]
              | {tags: (map(select(.[0] == "tag") | .[1]) | unique),
                 digests: (map(select(.[0] == "digest") | .[1]) | unique)}'
}

# ── Registry lookups (live mode) ─────────────────────────────────────────────

registry_token() {
  local repo_path="$1" url token_json
  url="https://${REGISTRY_HOST}/token?service=${REGISTRY_HOST}&scope=repository:${repo_path}:pull"
  if [[ -n "${GH_TOKEN:-${GITHUB_TOKEN:-}}" ]]; then
    token_json="$(printf 'user = "%s:%s"\n' "${GITHUB_ACTOR:-token}" "${GH_TOKEN:-${GITHUB_TOKEN:-}}" \
      | curl -fsS --config - "${url}")" || return 1
  else
    token_json="$(curl -fsS "${url}")" || return 1
  fi
  jq -er '.token' <<<"${token_json}"
}

# registry_children <repo-path> <token> <digest> -> child manifest digests, one per line
# A 404 means the manifest has no children we can see; any other failure is an error.
registry_children() {
  local out rc=0 status body
  out="$(curl -fsS -w '\n%{http_code}' -H "Authorization: Bearer ${2}" \
    -H "Accept: ${MANIFEST_ACCEPT}" \
    "https://${REGISTRY_HOST}/v2/${1}/manifests/${3}")" || rc=$?
  status="${out##*$'\n'}"
  body="${out%$'\n'*}"
  case "${status}" in
    200) jq -r '.manifests[]?.digest' <<<"${body}" ;;
    404) echo "warning: manifest ${3} not found in the registry, treating it as having no children" >&2 ;;
    *) echo "error: registry lookup for manifest ${3} failed (HTTP '${status}', curl exit ${rc})" >&2; return 1 ;;
  esac
}

# resolve_children <repo-path> <digest>... -> JSON object {index-digest: [children]}
# Follows children of children, and fails (so nothing is deleted) if any lookup fails.
resolve_children() {
  local repo_path="$1" token map='{}' d kids queue seen=$'\n'
  shift
  [[ $# -gt 0 ]] || { echo '{}'; return 0; }
  token="$(registry_token "${repo_path}")" || die "could not get a registry token for ${repo_path}"
  queue=""
  for d in "$@"; do queue+="${d}"$'\n'; done
  while [[ -n "${queue}" ]]; do
    d="${queue%%$'\n'*}"
    if [[ "${queue}" == *$'\n'* ]]; then queue="${queue#*$'\n'}"; else queue=""; fi
    [[ -n "${d}" && "${seen}" != *$'\n'"${d}"$'\n'* ]] || continue
    seen+="${d}"$'\n'
    kids="$(registry_children "${repo_path}" "${token}" "${d}")" || die "registry lookup failed for ${d}"
    [[ -n "${kids}" ]] || continue
    map="$(jq -c --arg d "${d}" --arg k "${kids}" '. + {($d): ($k | split("\n") | map(select(length > 0)))}' <<<"${map}")"
    queue+="${queue:+$'\n'}${kids}"
  done
  echo "${map}"
}

# ── Main ─────────────────────────────────────────────────────────────────────

# require_uint <value> <flag>
require_uint() { [[ "$1" =~ ^[0-9]+$ ]] || usage_error "$2 expects a non-negative integer, got '$1'"; }

main() {
  local owner package owner_type="org" min_age=30 keep_latest=10 max_delete=100
  local repo_root="${DEFAULT_REPO_ROOT}" now="" versions_file="" manifests_file="" delete=0
  local -a protect=("${DEFAULT_PROTECT_REGEXES[@]}")

  if [[ -n "${GITHUB_REPOSITORY:-}" ]]; then
    owner="${GITHUB_REPOSITORY%%/*}"
    package="$(tr '[:upper:]' '[:lower:]' <<<"${GITHUB_REPOSITORY#*/}")"
  else
    owner="OtowoOrg"
    package="stellar-k8s"
  fi

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --owner) owner="${2:?--owner needs a value}"; shift 2 ;;
      --package) package="${2:?--package needs a value}"; shift 2 ;;
      --owner-type) owner_type="${2:?--owner-type needs a value}"; shift 2 ;;
      --min-age-days) min_age="${2:?--min-age-days needs a value}"; shift 2 ;;
      --keep-latest) keep_latest="${2:?--keep-latest needs a value}"; shift 2 ;;
      --protect-tag) protect+=("${2:?--protect-tag needs a value}"); shift 2 ;;
      --max-delete) max_delete="${2:?--max-delete needs a value}"; shift 2 ;;
      --repo-root) repo_root="${2:?--repo-root needs a value}"; shift 2 ;;
      --now) now="${2:?--now needs a value}"; shift 2 ;;
      --versions-file) versions_file="${2:?--versions-file needs a value}"; shift 2 ;;
      --manifests-file) manifests_file="${2:?--manifests-file needs a value}"; shift 2 ;;
      --delete) delete=1; shift ;;
      -h|--help) usage; return 0 ;;
      *) usage_error "unknown argument: $1" ;;
    esac
  done

  require_uint "${min_age}" --min-age-days
  require_uint "${keep_latest}" --keep-latest
  require_uint "${max_delete}" --max-delete
  [[ -z "${now}" ]] || require_uint "${now}" --now
  [[ "${min_age}" -ge 1 ]] || usage_error "--min-age-days must be at least 1"
  [[ "${owner_type}" == "org" || "${owner_type}" == "user" ]] || usage_error "--owner-type must be org or user"
  [[ -z "${manifests_file}" || -n "${versions_file}" ]] || usage_error "--manifests-file needs --versions-file"
  [[ -z "${versions_file}" || "${delete}" -eq 0 ]] || usage_error "--delete cannot be combined with --versions-file (fixtures are never deleted)"
  [[ -z "${versions_file}" || -r "${versions_file}" ]] || usage_error "cannot read --versions-file ${versions_file}"
  [[ -z "${manifests_file}" || -r "${manifests_file}" ]] || usage_error "cannot read --manifests-file ${manifests_file}"
  command -v jq >/dev/null 2>&1 || die "jq is required"
  [[ -n "${versions_file}" ]] || command -v gh >/dev/null 2>&1 || die "gh is required (or use --versions-file)"
  [[ -n "${now}" ]] || now="$(date -u +%s)"

  local owner_lc package_enc api_base repo_path
  owner_lc="$(tr '[:upper:]' '[:lower:]' <<<"${owner}")"
  package_enc="${package//\//%2F}"
  repo_path="${owner_lc}/${package}"
  if [[ "${owner_type}" == "org" ]]; then api_base="orgs/${owner}"; else api_base="users/${owner}"; fi
  api_base+="/packages/container/${package_enc}/versions"

  tmp="$(mktemp -d)"
  trap '[[ -z "${tmp}" ]] || rm -rf "${tmp}"' EXIT

  # 1. versions
  if [[ -n "${versions_file}" ]]; then
    normalize_versions <"${versions_file}" >"${tmp}/versions.json"
  else
    gh api --paginate "${api_base}?per_page=100" | normalize_versions >"${tmp}/versions.json" \
      || die "could not list versions of ${REGISTRY_HOST}/${repo_path} (needs read:packages)"
  fi
  local total
  total="$(jq 'length' "${tmp}/versions.json")"

  # 2. what the repo references and which tags are protected
  local refs protect_json ref_tags ref_digests
  refs="$(scan_references "${repo_root}" "${package}")"
  ref_tags="$(jq -c '.tags' <<<"${refs}")"
  ref_digests="$(jq -c '.digests' <<<"${refs}")"
  protect_json="$(printf '%s\n' "${protect[@]}" | jq -Rn '[inputs]')"

  # 3. first pass without children, then resolve the children of what is kept
  local children='{}'
  if [[ -n "${manifests_file}" ]]; then
    children="$(jq -c '.' "${manifests_file}")"
  elif [[ -z "${versions_file}" && "${total}" -gt 0 ]]; then
    local -a kept_digests=()
    while IFS= read -r line; do kept_digests+=("${line}"); done < <(
      select_versions "${tmp}/versions.json" "${now}" "${min_age}" "${keep_latest}" '{}' \
        "${protect_json}" "${ref_tags}" "${ref_digests}" \
        | jq -r '.[] | select(.action == "keep" and .subject == null) | .digest')
    if [[ ${#kept_digests[@]} -gt 0 ]]; then
      children="$(resolve_children "${repo_path}" "${kept_digests[@]}")"
    fi
  fi

  # 4. final plan
  select_versions "${tmp}/versions.json" "${now}" "${min_age}" "${keep_latest}" "${children}" \
    "${protect_json}" "${ref_tags}" "${ref_digests}" >"${tmp}/plan.json"

  local n_keep n_delete mode="DRY RUN (nothing will be deleted)"
  n_keep="$(jq '[.[] | select(.action == "keep")] | length' "${tmp}/plan.json")"
  n_delete="$(jq '[.[] | select(.action == "delete")] | length' "${tmp}/plan.json")"
  [[ "${delete}" -eq 0 ]] || mode="DELETE"

  echo "Registry prune plan for ${REGISTRY_HOST}/${repo_path}"
  echo "  mode:          ${mode}"
  echo "  min age:       ${min_age} days, keep latest: ${keep_latest}, max delete: ${max_delete}"
  echo "  versions:      ${total}"
  echo "  repo-pinned:   $(jq -r '(.tags + .digests) | length' <<<"${refs}") tag/digest reference(s) found under charts/, config/, examples/"
  echo
  printf '%-7s %-12s %5s  %-26s %s\n' ACTION ID AGE REASON "TAGS / DIGEST"
  jq -r '.[] | [.action, (.id | tostring), (.age_days | tostring), .reason,
               (if (.tags | length) > 0 then (.tags | join(",")) else .digest end)] | @tsv' "${tmp}/plan.json" \
    | while IFS=$'\t' read -r action id age reason label; do
        printf '%-7s %-12s %4sd  %-26s %s\n' "$(tr '[:lower:]' '[:upper:]' <<<"${action}")" "${id}" "${age}" "${reason}" "${label}"
      done
  echo
  echo "Summary: keep ${n_keep}, delete ${n_delete} of ${total} version(s)"

  if [[ "${delete}" -eq 0 ]]; then
    if [[ "${n_delete}" -gt "${max_delete}" ]]; then
      echo "note: this plan exceeds --max-delete ${max_delete}; --delete would refuse to run it"
    fi
    echo "Dry run: no versions were deleted. Re-run with --delete to apply this plan."
    return 0
  fi

  if [[ "${n_delete}" -gt "${max_delete}" ]]; then
    echo "error: plan deletes ${n_delete} versions, more than --max-delete ${max_delete}; refusing to continue" >&2
    echo "       review the plan above and raise --max-delete to prune that many in one run" >&2
    return 3
  fi

  # 5. delete
  local id failed=0 deleted=0
  while IFS= read -r id; do
    [[ -n "${id}" ]] || continue
    if gh api --method DELETE "${api_base}/${id}" >/dev/null; then
      echo "deleted version ${id}"
      deleted=$((deleted + 1))
    else
      echo "error: failed to delete version ${id}" >&2
      failed=$((failed + 1))
    fi
  done < <(jq -r '.[] | select(.action == "delete") | .id' "${tmp}/plan.json")
  echo "Deleted ${deleted} version(s), ${failed} failure(s)"
  [[ "${failed}" -eq 0 ]]
}

# Run only when executed, so tests can source the selection functions.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  main "$@"
fi
