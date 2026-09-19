#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
version=$(awk -F '"' '/^version = / { print $2; exit }' "$root/Cargo.toml")
target=${1:-$(rustc -vV | sed -n 's/^host: //p')}
archive="run-cli-v${version}-${target}"
dist="$root/dist"
staging=$(mktemp -d "${TMPDIR:-/tmp}/run-cli-package.XXXXXX")
trap 'rm -rf -- "$staging"' EXIT

cargo build --locked --release --target "$target" --manifest-path "$root/Cargo.toml"
mkdir -p -- "$dist" "$staging/$archive"
install -m 0755 "$root/target/$target/release/run-cli" "$staging/$archive/run-cli"
install -m 0644 "$root/README.md" "$root/CHANGELOG.md" "$root/LICENSE" "$staging/$archive/"
tar -C "$staging" -czf "$dist/$archive.tar.gz" "$archive"
sha256sum "$dist/$archive.tar.gz" > "$dist/$archive.tar.gz.sha256"
printf 'Created %s\n' "$dist/$archive.tar.gz"
