#!/usr/bin/env sh
# Bump every authoritative version surface to the version passed by semantic-release.
#
# Called from the repository root by @semantic-release/exec:
#   sh scripts/release/bump-version.sh <version>
#
# Authoritative surfaces:
#   - Cargo.toml   [workspace.package] version   (both crates inherit via version.workspace = true)
#   - Cargo.lock   the nasfiles-core and nasfiles-server package entries
#
# The edits are offline and deterministic (awk only); no registry access, so this
# is safe inside the container-based Forgejo release job.
set -eu

VERSION="${1:?usage: bump-version.sh <version>}"

# Run from the repository root regardless of caller cwd.
ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
cd "$ROOT"

# 1) Cargo.toml — replace the version only inside the [workspace.package] section.
awk -v ver="$VERSION" '
  /^\[/ { in_pkg = ($0 == "[workspace.package]") }
  in_pkg && /^[[:space:]]*version[[:space:]]*=/ {
    sub(/"[^"]*"/, "\"" ver "\"")
    in_pkg = 0
  }
  { print }
' Cargo.toml > Cargo.toml.tmp
mv Cargo.toml.tmp Cargo.toml

# 2) Cargo.lock — replace the version in the two local package entries.
awk -v ver="$VERSION" '
  /^name = "nasfiles-core"$/ || /^name = "nasfiles-server"$/ { hit = 1 }
  hit && /^version = / {
    sub(/"[^"]*"/, "\"" ver "\"")
    hit = 0
  }
  { print }
' Cargo.lock > Cargo.lock.tmp
mv Cargo.lock.tmp Cargo.lock

echo "bump-version: set workspace version to ${VERSION} in Cargo.toml and Cargo.lock"
