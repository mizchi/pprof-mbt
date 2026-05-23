#!/usr/bin/env bash
# verify.sh — drive `moon test` and `moon fmt` against upstream main + one
# of the PR drafts in this directory. Run from a checkout of
# moonbitlang/core (not from pprof-mbt).
#
# Usage:
#   cd <core checkout>
#   /path/to/pprof-mbt/notes/pr-drafts/verify.sh 01     # tests PR-01
#   /path/to/pprof-mbt/notes/pr-drafts/verify.sh 02
#   /path/to/pprof-mbt/notes/pr-drafts/verify.sh all    # all four PRs sequentially

set -euo pipefail

# Resolve where this script lives so we can find the patch files.
DRAFTS_DIR="$(cd "$(dirname "$0")" && pwd)"

run_one() {
  local prnum="$1"
  local dir patch branch
  dir=$(echo "$DRAFTS_DIR"/${prnum}-* 2>/dev/null | head -1)
  if [ ! -d "$dir" ]; then
    echo "verify.sh: no draft for PR-$prnum (looked under $DRAFTS_DIR/${prnum}-*)" >&2
    exit 1
  fi
  patch=$(ls "$dir"/0001-*.patch 2>/dev/null | head -1)
  if [ ! -f "$patch" ]; then
    echo "verify.sh: no patch in $dir" >&2
    exit 1
  fi
  branch="$(basename "$patch" .patch | sed 's/^0001-//')"

  echo "==> PR-$prnum: $branch"
  git checkout -B "verify-pr-$prnum" main >/dev/null
  git am < "$patch"

  echo "==> moon fmt (should be a no-op)"
  moon fmt
  if ! git diff --quiet; then
    echo "verify.sh: moon fmt changed files in PR-$prnum — fix the patch and try again" >&2
    git diff --stat
    exit 1
  fi

  echo "==> moon test (all four backends)"
  for t in wasm wasm-gc js native; do
    printf "  %-7s " "$t"
    moon test --target "$t" 2>&1 | tail -1
  done
  echo
}

case "${1:-}" in
  all)
    for n in 01 02 03 04; do run_one "$n"; done
    ;;
  ""|-h|--help)
    cat <<EOF
usage: verify.sh <01|02|03|04|all>

Run from a clean checkout of https://github.com/moonbitlang/core
(on or rebased to main). Creates branch verify-pr-NN, applies the
matching patch, then runs moon fmt + moon test for each target.
EOF
    ;;
  *)
    run_one "$1"
    ;;
esac
