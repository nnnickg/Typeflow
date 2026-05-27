#!/usr/bin/env bash
set -euo pipefail

release_tag="${1:-${RELEASE_TAG:-}}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
packages=(
  typeclaw-core
  typeclaw-host-config
  typeclaw-data
  typeclaw-ffi
  typeclaw-cli
)

workspace_version=""

for package in "${packages[@]}"; do
  package_version="$("$script_dir/typeclaw-version.sh" "$package")"

  if [[ -z "$workspace_version" ]]; then
    workspace_version="$package_version"
  elif [[ "$package_version" != "$workspace_version" ]]; then
    echo "error: $package is $package_version, expected $workspace_version" >&2
    exit 1
  fi
done

if [[ -n "$release_tag" && "$release_tag" != "v$workspace_version" ]]; then
  echo "error: release tag is $release_tag, expected v$workspace_version" >&2
  exit 1
fi

echo "release version verified: v$workspace_version"
