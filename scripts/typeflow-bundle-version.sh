#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root_dir="$(cd "$script_dir/.." && pwd)"

count="$(git -C "$root_dir" rev-list --count HEAD)"
if [[ -z "$count" || "$count" == "0" ]]; then
  echo "error: refusing CFBundleVersion=$count" >&2
  exit 1
fi

version="$("$script_dir/typeflow-version.sh")"
major="${version%%.*}"
if [[ -z "$major" || ! "$major" =~ ^[0-9]+$ ]]; then
  echo "error: cannot derive numeric CFBundleVersion prefix from version '$version'" >&2
  exit 1
fi

printf '%s.%s\n' "$major" "$count"
