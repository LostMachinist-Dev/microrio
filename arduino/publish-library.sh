#!/usr/bin/env bash
# Push arduino/MicroRIO to its own repository, where the Arduino IDE can
# install it from.
#
#   arduino/publish-library.sh
#
# The library has to be the root of a repository of its own for the IDE and
# the library manager to accept it, but it must not become a second copy
# anybody edits. So this is the only way it gets there: the station repo
# stays the source, and this copies the current tree over the published one
# and commits the difference. Nothing is force-pushed and no history is lost.
#
# Needs `gh` signed in as the account that owns the repository.
set -euo pipefail
cd "$(dirname "$0")/.."

REPO=${MICRORIO_LIB_REPO:-LostMachinist-Dev/microrio-arduino}
SRC=arduino/MicroRIO
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

VER=$(sed -n 's/^version=//p' "$SRC/library.properties")
STAMP=$(git describe --always --dirty=+ 2>/dev/null || echo unknown)

# What gets published is the committed tree, not the working one, so that
# what is public is always something with a commit behind it. Which means an
# uncommitted change here would be skipped in silence - and it said "nothing
# to publish" once while a new file sat unstaged beside it.
if [ -n "$(git status --porcelain -- "$SRC")" ]; then
    echo "$SRC has uncommitted changes. Commit them first:" >&2
    git status --short -- "$SRC" >&2
    exit 1
fi

git clone -q "https://github.com/$REPO.git" "$WORK/repo"
cd "$WORK/repo"

# Everything the published repo holds that this script manages. Its own
# .git stays; nothing else is kept, so a file deleted upstairs is deleted
# here too.
find . -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
cd - >/dev/null
git archive HEAD "$SRC" | tar -x -C "$WORK/repo" --strip-components=2
install -m644 LICENSE LICENSE-APACHE LICENSE-MIT "$WORK/repo/"
printf 'wifi_secrets.h\nwifi.txt\n' > "$WORK/repo/.gitignore"

cd "$WORK/repo"
git add -A
if git diff --cached --quiet; then
    echo "nothing to publish: $REPO already matches $SRC at $STAMP"
    exit 0
fi
git -c user.name=lostmachinist \
    -c user.email=315170983+lostmachinist@users.noreply.github.com \
    commit -q -m "MicroRIO Arduino library $VER (station $STAMP)

Copied from arduino/MicroRIO in the station repository, which is where
this is written and reviewed. See its history for why any of it is the
way it is.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
git push -q origin HEAD:main
echo "pushed $VER to $REPO (from station $STAMP)"
