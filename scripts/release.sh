#!/usr/bin/env bash
# One-command release. Computes the next version from the gitmoji history
# with git-cliff (see cliff.toml), syncs the three version files plus
# Cargo.lock, prepends CHANGELOG.md, describes @, pushes main, then tags
# and pushes the tag — which fires the signed build in release.yml.
#
# Expects an empty @ sitting directly on top of main (i.e. run it right
# after `jj new`, when there is nothing uncommitted).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

command -v git-cliff >/dev/null || {
  echo "git-cliff not found — brew install git-cliff" >&2
  exit 1
}

# jj only exports to .git on demand; git-cliff reads git, so sync first or
# it walks a stale HEAD and misses the newest commits.
jj git export

if ! git tag -l 'v[0-9]*' | grep -q .; then
  echo "No release tags yet. Cut the first one by hand:" >&2
  echo "  git tag v0.1.0 <commit> && git push origin v0.1.0" >&2
  exit 1
fi

if [ -n "$(jj log -r @ --no-graph -T 'if(empty, "", "x")')" ]; then
  echo "@ is not empty — describe or shelve your work first, then run this with a clean working copy." >&2
  exit 1
fi

if [ "$(jj log -r main --no-graph -T commit_id)" != "$(jj log -r @- --no-graph -T commit_id)" ]; then
  echo "@ is not sitting on top of main — jj new main first." >&2
  exit 1
fi

CURRENT=$(sed -n 's/^  "version": "\(.*\)",$/\1/p' package.json)
NEXT=$(git-cliff --bumped-version 2>/dev/null)   # e.g. v0.2.0
VERSION=${NEXT#v}

if [ "$VERSION" = "$CURRENT" ]; then
  echo "Nothing to release — no commits since $NEXT that change the version."
  exit 1
fi

echo "v$CURRENT → $NEXT"
read -r -p "Bump, update CHANGELOG.md, push main and tag $NEXT? [y/N] " ans
[[ $ans == [yY]* ]] || { echo "Aborted — nothing was changed."; exit 1; }

# Three files carry the version; Cargo.lock follows via cargo.
sed -i '' "s/^version = \"$CURRENT\"$/version = \"$VERSION\"/" Cargo.toml
sed -i '' "s/\"version\": \"$CURRENT\"/\"version\": \"$VERSION\"/" package.json src-tauri/tauri.conf.json
cargo metadata --format-version=1 --offline >/dev/null 2>&1 \
  || cargo metadata --format-version=1 >/dev/null

git-cliff --unreleased --tag "$NEXT" --prepend CHANGELOG.md

jj describe -m ":bookmark: (release): v$VERSION"
jj bookmark set main -r @
jj git push --bookmark main

# describe rewrote the commit, so resolve @ only now.
git tag "$NEXT" "$(jj log -r @ --no-graph -T commit_id)"
git push origin "$NEXT"

jj new
echo "Released $NEXT — the signed build is running: gh run watch"
