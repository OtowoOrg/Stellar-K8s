#!/bin/bash
# Generate a compatibility badge in SVG format
# Usage: generate-badge.sh "42/42" "compatibility"

set -euo pipefail

LABEL="${1:-passed/total}"
SUBJECT="${2:-compatibility}"
COLOR="green"

# Parse label (e.g., "42/42")
if [[ "$LABEL" =~ ^([0-9]+)/([0-9]+)$ ]]; then
    PASSED="${BASH_REMATCH[1]}"
    TOTAL="${BASH_REMATCH[2]}"
    
    if [[ $TOTAL -gt 0 ]]; then
        PCT=$((PASSED * 100 / TOTAL))
        if [[ $PCT -lt 50 ]]; then
            COLOR="red"
        elif [[ $PCT -lt 80 ]]; then
            COLOR="orange"
        else
            COLOR="green"
        fi
    fi
fi

# Generate SVG badge
cat << EOF
<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="120" height="20" role="img" aria-label="$SUBJECT: $LABEL">
  <title>$SUBJECT: $LABEL</title>
  <linearGradient id="s" x2="0" y2="100%">
    <stop offset="0" stop-color="#bbb"/>
    <stop offset="1" stop-color="#999"/>
  </linearGradient>
  <clipPath id="r">
    <rect width="120" height="20" rx="3" fill="#fff"/>
  </clipPath>
  <g clip-path="url(#r)">
    <rect width="87" height="20" fill="#555"/>
    <rect x="87" width="33" height="20" fill="$COLOR"/>
    <rect width="120" height="20" fill="url(#s)"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" text-rendering="geometricPrecision" font-size="11">
    <text aria-hidden="true" x="445" y="150" fill="#010101" fill-opacity=".3" transform="scale(.1)" textLength="770">$SUBJECT</text>
    <text x="445" y="140" transform="scale(.1)" fill="#fff" textLength="770">$SUBJECT</text>
    <text aria-hidden="true" x="1025" y="150" fill="#010101" fill-opacity=".3" transform="scale(.1)" textLength="230">$LABEL</text>
    <text x="1025" y="140" transform="scale(.1)" fill="#fff" textLength="230">$LABEL</text>
  </g>
</svg>
EOF
