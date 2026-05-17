#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root_dir="$(cd "$script_dir/.." && pwd)"
package="${1:-typeflow-cli}"

package_id="$(cargo pkgid --manifest-path "$root_dir/Cargo.toml" -p "$package")"

if [[ "$package_id" == *@* ]]; then
  version="${package_id##*@}"
elif [[ "$package_id" == *#* ]]; then
  version="${package_id##*#}"
else
  echo "error: failed to parse Cargo version from: $package_id" >&2
  exit 1
fi

printf '%s\n' "$version"
