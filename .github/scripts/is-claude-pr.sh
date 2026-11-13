#!/usr/bin/env bash
# Exits 0 if PR $1 is one Claude looks after (claude-sync, claude-ci-fix,
# claude-merge): an open PR that Claude opened, or one where a person has
# mentioned @claude in the description, a comment or a review.
# Needs GH_TOKEN and REPO.
set -euo pipefail
pr="$1"

pull=$(gh api "repos/$REPO/pulls/$pr")
[ "$(jq -r .state <<<"$pull")" = open ] || exit 1
[ "$(jq -r .user.login <<<"$pull")" = "claude[bot]" ] && exit 0

text=$(jq -r '.body // ""' <<<"$pull")
for endpoint in "issues/$pr/comments" "pulls/$pr/reviews" "pulls/$pr/comments"; do
  text+=$'\n'$(gh api --paginate "repos/$REPO/$endpoint" --jq '.[] | select(.user.type == "User") | .body // ""')
done
grep -qi '@claude' <<<"$text"
