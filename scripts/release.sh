#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/release.sh "commit message"
# Bumps patch version in tauri.conf.json, commits, pushes, and tags.

CONF="src-tauri/tauri.conf.json"
cd "$(git rev-parse --show-toplevel)"

# Build frontend first
echo "Building frontend..."
cd packages/daw-ui && npm run build && cd ../..

# Get current version
CURRENT=$(grep '"version"' "$CONF" | head -1 | sed 's/.*"\([0-9]*\.[0-9]*\.[0-9]*\)".*/\1/')
MAJOR=$(echo "$CURRENT" | cut -d. -f1)
MINOR=$(echo "$CURRENT" | cut -d. -f2)
PATCH=$(echo "$CURRENT" | cut -d. -f3)
NEW_PATCH=$((PATCH + 1))
NEW_VERSION="$MAJOR.$MINOR.$NEW_PATCH"

echo "Version: $CURRENT -> $NEW_VERSION"

# Bump version
sed -i "s/\"version\": \"$CURRENT\"/\"version\": \"$NEW_VERSION\"/" "$CONF"

# Commit message from arg or default
MSG="${1:-Release v$NEW_VERSION}"

# Stage all changes, commit, push, tag
git add -A
git commit -m "$MSG"
git push origin master
git tag "v$NEW_VERSION"
git push origin "v$NEW_VERSION"

echo "Released v$NEW_VERSION — CI building at https://github.com/Dishairano/hardwave-daw/actions"
