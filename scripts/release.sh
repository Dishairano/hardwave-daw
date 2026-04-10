#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/release.sh "commit message"
# Bumps patch version in tauri.conf.json, commits, pushes, and tags.
# Generates a changelog from git commits since the last tag.

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

# Generate changelog from commits since last tag
LAST_TAG=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
if [ -n "$LAST_TAG" ]; then
  CHANGELOG=$(git log "${LAST_TAG}..HEAD" --pretty=format:"- %s" --no-merges | grep -v "^- Release\|^- v[0-9]" || true)
else
  CHANGELOG=$(git log --pretty=format:"- %s" --no-merges -10 || true)
fi

# Write changelog to file for CI to pick up
CHANGELOG_FILE="RELEASE_CHANGELOG.md"
{
  echo "## Changes"
  if [ -n "$CHANGELOG" ]; then
    echo "$CHANGELOG"
  else
    echo "- Bug fixes and improvements"
  fi
} > "$CHANGELOG_FILE"

# Bump version
sed -i "s/\"version\": \"$CURRENT\"/\"version\": \"$NEW_VERSION\"/" "$CONF"

# Commit message from arg or default
MSG="${1:-Release v$NEW_VERSION}"

# Stage all changes, commit, push, tag
git add -A
git commit -m "$MSG"
git push origin master
git tag -a "v$NEW_VERSION" -m "$(cat "$CHANGELOG_FILE")"
git push origin "v$NEW_VERSION"

# Clean up changelog file
rm -f "$CHANGELOG_FILE"

echo "Released v$NEW_VERSION — CI building at https://github.com/Dishairano/hardwave-daw/actions"
