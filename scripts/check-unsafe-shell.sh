#!/usr/bin/env bash
# scripts/check-unsafe-shell.sh — Static analysis gate for unsafe shell patterns.
#
# Scans every shell script in the repository for a deny-list of patterns that
# are dangerous regardless of what ShellCheck reports (ShellCheck focuses on
# correctness/portability, not security). This is a *complement* to
# ShellCheck, not a replacement — run both.
#
# Detected patterns (all fail the gate unless explicitly allowlisted):
#   - eval of a dynamically constructed string
#   - piping curl/wget output directly into a shell (remote code execution)
#   - chmod granting world-write permissions (777, a+w, o+w)
#   - SSH/git host-key verification disabled (StrictHostKeyChecking=no)
#   - curl/wget with TLS certificate verification disabled (-k/--insecure)
#   - unquoted `rm -rf $VAR` (word-splitting / empty-variable footgun)
#
# A line that is a genuine, reviewed exception can be allowlisted by adding
# `# unsafe-shell-allow: <reason>` on the same line or the line directly
# above it. The reason is required and is echoed in the report so allowlisted
# lines stay auditable.
#
# Usage:
#   scripts/check-unsafe-shell.sh [PATH ...]   # defaults to the whole repo
#   make check-unsafe-shell

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=scripts/lib/errors.sh
source "${SCRIPT_DIR}/lib/errors.sh"
cd "${REPO_ROOT}"
readonly SCRIPT_DIR REPO_ROOT

# Directories that intentionally hold archived/generated one-off scripts and
# are not part of the maintained surface area.
EXCLUDE_DIRS_REGEX='(^|/)(scripts/archive|target|node_modules|\.git)(/|$)'

SEARCH_PATHS=("$@")
if [[ "${#SEARCH_PATHS[@]}" -eq 0 ]]; then
  SEARCH_PATHS=(".")
fi

mapfile -t SHELL_FILES < <(
  find "${SEARCH_PATHS[@]}" -type f -name '*.sh' 2>/dev/null \
    | grep -Ev "${EXCLUDE_DIRS_REGEX}" \
    | sort -u
)

if [[ "${#SHELL_FILES[@]}" -eq 0 ]]; then
  echo "No shell scripts found under: ${SEARCH_PATHS[*]}"
  exit 0
fi

# Pattern name -> extended regex. Order matches the doc comment above.
declare -a PATTERN_NAMES=(
  "eval-usage"
  "curl-pipe-shell"
  "curl-subshell-exec"
  "world-writable-chmod"
  "ssh-host-key-check-disabled"
  "tls-verification-disabled"
  "unquoted-rm-rf-var"
)
declare -a PATTERN_REGEXES=(
  '(^|[;&(]|&&|\|\||\$\()[[:space:]]*eval\b'
  '(curl|wget)[^|#]*\|[[:space:]]*(sudo[[:space:]]+)?(ba|z)?sh([[:space:]]|$)'
  '(ba|z)?sh[[:space:]]+-c[[:space:]]+"?\$\([[:space:]]*(curl|wget)'
  'chmod[[:space:]]+(-R[[:space:]]+)?(777|a\+w|o\+w)([[:space:]]|$)'
  'StrictHostKeyChecking[=[:space:]]+no'
  '(curl|wget)[^#]*(--insecure|--no-check-certificate|[[:space:]]-k([[:space:]]|$))'
  'rm[[:space:]]+(-[A-Za-z]*r[A-Za-z]*f[A-Za-z]*|-[A-Za-z]*f[A-Za-z]*r[A-Za-z]*)[[:space:]]+\$\{?[A-Za-z_][A-Za-z0-9_]*\}?([[:space:]]|$)'
)

declare -a VIOLATIONS=()
declare -a ALLOWLISTED=()

is_allowlisted() {
  local file="$1" line_no="$2"
  local this_line prev_line
  this_line="$(sed -n "${line_no}p" "${file}")"
  if [[ "${this_line}" == *"unsafe-shell-allow:"* ]]; then
    return 0
  fi
  if ((line_no > 1)); then
    prev_line="$(sed -n "$((line_no - 1))p" "${file}")"
    [[ "${prev_line}" == *"unsafe-shell-allow:"* ]]
  else
    return 1
  fi
}

allow_reason() {
  local file="$1" line_no="$2"
  local this_line prev_line
  this_line="$(sed -n "${line_no}p" "${file}")"
  if [[ "${this_line}" == *"unsafe-shell-allow:"* ]]; then
    echo "${this_line#*unsafe-shell-allow:}" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//'
    return
  fi
  if ((line_no > 1)); then
    prev_line="$(sed -n "$((line_no - 1))p" "${file}")"
    echo "${prev_line#*unsafe-shell-allow:}" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//'
  fi
}

sk8s_step "unsafe shell patterns" "Scanning ${#SHELL_FILES[@]} shell script(s)"

for file in "${SHELL_FILES[@]}"; do
  for i in "${!PATTERN_NAMES[@]}"; do
    name="${PATTERN_NAMES[$i]}"
    regex="${PATTERN_REGEXES[$i]}"

    while IFS=: read -r line_no content; do
      [[ -z "${line_no}" ]] && continue
      # Skip matches inside comments (line starts with # after whitespace).
      trimmed="${content#"${content%%[![:space:]]*}"}"
      [[ "${trimmed}" == \#* ]] && continue

      if is_allowlisted "${file}" "${line_no}"; then
        ALLOWLISTED+=("${file}:${line_no} [${name}] $(allow_reason "${file}" "${line_no}")")
      else
        VIOLATIONS+=("${file}:${line_no} [${name}] ${trimmed}")
      fi
    done < <(grep -nE "${regex}" "${file}" || true)
  done
done

if [[ "${#ALLOWLISTED[@]}" -gt 0 ]]; then
  sk8s_info "${#ALLOWLISTED[@]} allowlisted pattern(s):"
  for entry in "${ALLOWLISTED[@]}"; do
    echo "    ${entry}"
  done
fi

if [[ "${#VIOLATIONS[@]}" -gt 0 ]]; then
  echo -e "\n${SK8S_RED}${SK8S_BOLD}Unsafe shell patterns found:${SK8S_RESET}" >&2
  for entry in "${VIOLATIONS[@]}"; do
    echo "    ${entry}" >&2
  done
  echo "" >&2
  sk8s_fail \
    "${#VIOLATIONS[@]} unsafe pattern(s) detected" \
    "Fix the pattern, or if it is a reviewed exception add '# unsafe-shell-allow: <reason>' on the same or preceding line"
fi

sk8s_pass "No unsafe shell patterns detected across ${#SHELL_FILES[@]} script(s)"
