#!/usr/bin/env bash
set -euo pipefail

# First paragraph: env setup
export KEBAB_HOME="${KEBAB_HOME:-$HOME/.local/share/kebab}"
mkdir -p "$KEBAB_HOME"
cd "$KEBAB_HOME"

# Second paragraph: ingest
echo "ingesting workspace..."
kebab ingest --config /etc/kebab/config.toml

# Third paragraph: report
echo "done"
kebab schema --json | jq '.stats'
