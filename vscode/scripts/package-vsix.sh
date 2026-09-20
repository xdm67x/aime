#!/usr/bin/env bash
# Packages the extension .vsix for the current platform, embedding the
# platform-matching pulse-node addon built by `pnpm --dir pulse-node run build`.
# Usage: package-vsix.sh <vsce-target> [version]
#   <vsce-target>  e.g. linux-x64, darwin-arm64, win32-x64
#   [version]      overrides the manifest version (defaults to package.json),
#                  used to stamp the .vsix with the release tag version.
set -euo pipefail

target="${1:?usage: package-vsix.sh <vsce-target> [version]}"
version="${2:-$(node -p "require('./package.json').version")}"
version="${version#v}"
cd "$(dirname "$0")/.."

pnpm build
pnpm vendor:native
pnpm exec vsce package --no-dependencies --target "$target" "$version" \
  --no-update-package-json --no-git-tag-version \
  -o "pulse-vscode-v${version}-${target}.vsix"
