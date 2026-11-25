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
# scripts/apply-branch-protection.sh — apply the committed main-branch ruleset.
#
# Repository settings cannot be changed by a pull request, so the branch
# protection for main is kept as a repository ruleset definition in
# .github/rulesets/main.json and applied by a maintainer with this script.
# It creates the ruleset, or updates it in place when one with the same name
# already exists, so it is safe to run again after the file changes.
#
# Usage:
#   scripts/apply-branch-protection.sh [OPTIONS]            # dry run (the default)
#   scripts/apply-branch-protection.sh --apply [OPTIONS]    # create or update the ruleset
#   scripts/apply-branch-protection.sh --check [OPTIONS]    # read-only: is it live and unchanged?
#
# Options:
#   --repo OWNER/REPO       target repository (default: $GITHUB_REPOSITORY, else OtowoOrg/Stellar-K8s)
#   --ruleset FILE          ruleset definition (default: .github/rulesets/main.json)
#   --workflows-dir DIR     workflows the required check names are validated against
#                           (default: .github/workflows)
#   --apply                 send the ruleset to GitHub (needs repository admin)
#   --check                 compare the live ruleset with FILE; exit 1 if it is missing or differs
#   -h, --help              show this help
#
# The dry run prints what would be sent and, when `gh` is logged in, whether it
# would create or update. It never writes. Every required status check must match
# a job name in the workflows directory: --apply refuses a check that no workflow
# reports, because a required check that never runs blocks every merge.
# Requires `gh` and `jq`. Exit codes: 0 ok, 1 failure or mismatch, 2 bad usage.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
live_tmp=""
trap '[[ -z "${live_tmp}" ]] || rm -f "${live_tmp}"' EXIT

die() { echo "error: $*" >&2; exit 1; }
usage_error() { echo "error: $*" >&2; echo "Run with --help for usage." >&2; exit 2; }
usage() {
  awk '/^# Usage:/ {p=1} p && /^[^#]/ {exit} p {sub(/^# ?/, ""); print}' "${BASH_SOURCE[0]}"
}

# unknown_checks <ruleset> <workflows-dir> -> required check names no workflow job defines
unknown_checks() {
  local ctx
  while IFS= read -r ctx; do
    [[ -n "${ctx}" ]] || continue
    if ! grep -rhxF -e "    name: ${ctx}" -e "    name: \"${ctx}\"" "$2" >/dev/null 2>&1; then
      printf '%s\n' "${ctx}"
    fi
  done < <(jq -r '.rules[] | select(.type == "required_status_checks")
                  | .parameters.required_status_checks[].context' "$1")
}

# differences <desired.json> <live.json> -> one line per way the live ruleset falls short
# Live rulesets carry extra fields (ids, links, defaulted parameters), so rules are
# compared as "every desired parameter is present with the same value".
# shellcheck disable=SC2016  # jq variables, not shell expansions
DIFF_JQ='
def norm: if type == "object" and has("required_status_checks")
          then .required_status_checks |= (map({context, integration_id}) | sort_by(.context)) else . end;
def actors: map({actor_id, actor_type, bypass_mode}) | sort_by([.actor_type, .actor_id]);
($d[0]) as $want | ($l[0]) as $have
| (
    (if $want.enforcement != $have.enforcement
       then "enforcement is \($have.enforcement), want \($want.enforcement)" else empty end),
    (if $want.target != $have.target then "target is \($have.target), want \($want.target)" else empty end),
    (if ($want.conditions.ref_name.include | sort) != ($have.conditions.ref_name.include | sort)
       then "ref include is \($have.conditions.ref_name.include), want \($want.conditions.ref_name.include)" else empty end),
    (if ($want.conditions.ref_name.exclude | sort) != ($have.conditions.ref_name.exclude | sort)
       then "ref exclude differs" else empty end),
    (if ($want.bypass_actors // [] | actors) != ($have.bypass_actors // [] | actors)
       then "bypass actors differ" else empty end),
    ($want.rules[] as $r
      | ([$have.rules[] | select(.type == $r.type)] | first) as $h
      | if $h == null then "rule \($r.type) is missing"
        else (($r.parameters // {}) | norm | to_entries[] as $e
              | select((($h.parameters // {}) | norm)[$e.key] != $e.value)
              | "rule \($r.type): \($e.key) is \(($h.parameters // {}) | norm | .[$e.key] | tojson), want \($e.value | tojson)")
        end)
  )'

differences() {
  jq -nr --slurpfile d "$1" --slurpfile l "$2" "${DIFF_JQ}"
}

main() {
  local repo ruleset="${REPO_ROOT}/.github/rulesets/main.json" workflows="${REPO_ROOT}/.github/workflows"
  local mode="dry-run"

  repo="${GITHUB_REPOSITORY:-OtowoOrg/Stellar-K8s}"

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --repo) repo="${2:?--repo needs a value}"; shift 2 ;;
      --ruleset) ruleset="${2:?--ruleset needs a value}"; shift 2 ;;
      --workflows-dir) workflows="${2:?--workflows-dir needs a value}"; shift 2 ;;
      --apply) [[ "${mode}" == "dry-run" ]] || usage_error "--apply and --check are mutually exclusive"; mode="apply"; shift ;;
      --check) [[ "${mode}" == "dry-run" ]] || usage_error "--apply and --check are mutually exclusive"; mode="check"; shift ;;
      -h|--help) usage; return 0 ;;
      *) usage_error "unknown argument: $1" ;;
    esac
  done

  [[ "${repo}" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || usage_error "--repo must look like OWNER/REPO, got '${repo}'"
  [[ -r "${ruleset}" ]] || usage_error "cannot read ruleset file ${ruleset}"
  [[ -d "${workflows}" ]] || usage_error "workflows directory not found: ${workflows}"
  command -v jq >/dev/null 2>&1 || die "jq is required"

  jq -e '.name and .target == "branch" and (.rules | type == "array" and length > 0)' "${ruleset}" >/dev/null \
    || die "${ruleset} is not a branch ruleset definition (needs name, target \"branch\" and rules)"

  local name unknown
  name="$(jq -r '.name' "${ruleset}")"
  unknown="$(unknown_checks "${ruleset}" "${workflows}")"

  echo "Branch protection ruleset for ${repo}"
  case "${mode}" in
    dry-run) echo "  mode:            DRY RUN (nothing will be sent)" ;;
    apply)   echo "  mode:            APPLY" ;;
    check)   echo "  mode:            CHECK (read-only)" ;;
  esac
  echo "  file:            ${ruleset}"
  echo "  name:            ${name}"
  echo "  applies to:      $(jq -r '.conditions.ref_name.include | join(", ")' "${ruleset}")"
  echo "  enforcement:     $(jq -r '.enforcement' "${ruleset}")"
  echo "  rules:           $(jq -r '[.rules[].type] | join(", ")' "${ruleset}")"
  echo "  approvals:       $(jq -r '[.rules[] | select(.type == "pull_request") | .parameters.required_approving_review_count][0] // "none"' "${ruleset}")"
  echo "  bypass:          $(jq -r '[.bypass_actors[]? | "\(.actor_type) \(.actor_id) (\(.bypass_mode))"] | join(", ") | if . == "" then "nobody" else . end' "${ruleset}")"
  echo "  required checks:"
  jq -r '.rules[] | select(.type == "required_status_checks") | .parameters.required_status_checks[].context | "    - " + .' "${ruleset}"
  if [[ -n "${unknown}" ]]; then
    echo "  WARNING: no job in ${workflows} is named:" >&2
    while IFS= read -r line; do echo "    - ${line}" >&2; done <<<"${unknown}"
  else
    echo "  check names:     every required check matches a job name in ${workflows}"
  fi
  echo

  command -v gh >/dev/null 2>&1 || {
    [[ "${mode}" == "dry-run" ]] || die "gh is required for --${mode}"
    echo "gh not found: cannot tell whether the ruleset exists, it would be created or updated by name."
    echo "Payload:"; jq . "${ruleset}"; return 0
  }

  # Look the ruleset up by name (read-only).
  local id="" listing lookup_ok=1
  if listing="$(gh api --paginate "repos/${repo}/rulesets" 2>&1)"; then
    id="$(jq -rs --arg n "${name}" 'add // [] | map(select(.name == $n)) | .[0].id // empty' <<<"${listing}")"
  else
    lookup_ok=0
    [[ "${mode}" == "dry-run" ]] || die "could not list rulesets of ${repo}: ${listing}"
  fi

  case "${mode}" in
    dry-run)
      if [[ "${lookup_ok}" -eq 0 ]]; then
        echo "Would create or update ruleset \"${name}\" on ${repo} (could not read existing rulesets: run 'gh auth login')."
      elif [[ -n "${id}" ]]; then
        echo "Would update ruleset \"${name}\" (id ${id}): PUT repos/${repo}/rulesets/${id}"
      else
        echo "Would create ruleset \"${name}\": POST repos/${repo}/rulesets"
      fi
      echo "Payload:"
      jq . "${ruleset}"
      echo
      echo "Dry run: nothing was sent. Re-run with --apply (repository admin required) to apply it."
      ;;
    check)
      if [[ -z "${id}" ]]; then
        echo "MISSING: ruleset \"${name}\" does not exist on ${repo}" >&2
        return 1
      fi
      local diffs
      live_tmp="$(mktemp)"
      gh api "repos/${repo}/rulesets/${id}" >"${live_tmp}" || die "could not read ruleset ${id}"
      diffs="$(differences "${ruleset}" "${live_tmp}")"
      if [[ -n "${diffs}" ]]; then
        echo "DIFFERS: ruleset \"${name}\" (id ${id}) does not match ${ruleset}:" >&2
        while IFS= read -r line; do echo "  - ${line}" >&2; done <<<"${diffs}"
        return 1
      fi
      echo "OK: ruleset \"${name}\" (id ${id}) is present on ${repo} and matches ${ruleset}"
      ;;
    apply)
      if [[ -n "${unknown}" ]]; then
        die "refusing to apply: a required check that no workflow reports would block every merge"
      fi
      if [[ -n "${id}" ]]; then
        gh api --method PUT "repos/${repo}/rulesets/${id}" --input "${ruleset}" >/dev/null
        echo "Updated ruleset \"${name}\" (id ${id}) on ${repo}."
      else
        gh api --method POST "repos/${repo}/rulesets" --input "${ruleset}" >/dev/null
        echo "Created ruleset \"${name}\" on ${repo}."
      fi
      echo "Verify with: scripts/apply-branch-protection.sh --check --repo ${repo}"
      ;;
  esac
}

# Run only when executed, so tests can source the helpers.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  main "$@"
fi
