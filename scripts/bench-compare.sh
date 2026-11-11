#!/usr/bin/env bash
# Compares the `format` benchmark (benches/format.rs) of this checkout with
# that of a base commit, both built and run here, and fails when this
# checkout is slower by more than a threshold.
#
# Usage: scripts/bench-compare.sh BASE [THRESHOLD_PERCENT]
#
# BASE is a git revision, checked out into a temporary worktree, or a
# directory that holds the base's sources. THRESHOLD_PERCENT defaults to 10.
#
# Shared CI runners are noisy, so a slowdown counts only when it shows four
# ways: both commits run in the same job on the same machine; the base runs
# before and after this checkout, and only a slowdown against both runs
# counts, which cancels a drift in the machine's speed during the job; a
# slowdown counts only when the lower end of criterion's 95% confidence
# interval for the change of the median is above the threshold; and a
# benchmark that fails is run once more, and fails the script only if it
# fails again, so that a burst of load during one run is not enough.
#
# Nothing is compared when the base has no `format` benchmark, or when the
# benchmark or its input generator differ: the timings would not be of the
# same work. The `fixtures` benchmark is left out when the fixtures differ.
set -euo pipefail
# printf and awk read and print decimal points in the C locale only.
export LC_ALL=C

if [[ $# -lt 1 ]]; then
  echo "usage: $0 BASE [THRESHOLD_PERCENT]" >&2
  exit 2
fi
threshold=${2:-10}
root=$(cd "$(dirname "$0")/.." && pwd)
work=${CARGO_TARGET_DIR:-$root/target}/bench-compare
export CRITERION_HOME=$work/criterion

notice() {
  if [[ -n ${GITHUB_ACTIONS:-} ]]; then
    echo "::notice::$1"
  else
    echo "$1"
  fi
}

rm -rf "$work"
mkdir -p "$work"
if [[ -d $1 ]]; then
  base_dir=$(cd "$1" && pwd)
else
  if ! base=$(git -C "$root" rev-parse --quiet --verify "$1^{commit}"); then
    notice "the base $1 is not a commit in this clone; nothing to compare"
    exit 0
  fi
  base_dir=$work/base
  git -C "$root" worktree prune
  git -C "$root" worktree add --quiet --detach "$base_dir" "$base"
  trap 'git -C "$root" worktree remove --force "$base_dir"' EXIT
fi

for file in benches/format.rs tests/support/doc_sql.rs; do
  if [[ ! -f $base_dir/$file ]]; then
    notice "the base has no $file; nothing to compare"
    exit 0
  fi
  if ! cmp -s "$base_dir/$file" "$root/$file"; then
    notice "$file differs from the base; the timings would not be comparable"
    exit 0
  fi
done

# A criterion filter: a regular expression over `format/<benchmark>`.
filter='^format/'
if ! diff -rq "$base_dir/tests/fixtures" "$root/tests/fixtures" >/dev/null; then
  notice "tests/fixtures differs from the base; comparing generated_1mb only"
  filter='^format/generated_1mb$'
fi

# Both builds come first, so that no build runs beside a measurement.
(cd "$base_dir" && cargo bench --bench format --no-run)
(cd "$root" && cargo bench --bench format --no-run)

# bench DIR FILTER CRITERION_ARGS…
bench() {
  local dir=$1 only=$2
  shift 2
  (cd "$dir" && cargo bench --bench format -- --noplot "$@" "$only")
}

# One `NAME LOWER_BOUND_PERCENT` line per benchmark: the change of this
# checkout's saved baseline `$1` against the baseline `$2`, at the low end
# of its confidence interval. `--load-baseline` compares saved results
# without measuring again.
changes() {
  bench "$root" "$filter" --load-baseline "$1" --baseline "$2" >/dev/null
  local estimates name
  for estimates in "$CRITERION_HOME"/format/*/change/estimates.json; do
    name=$(basename "$(dirname "$(dirname "$estimates")")")
    if [[ "format/$name" =~ $filter ]]; then
      jq -r --arg name "$name" \
        '"\($name) \(.median.confidence_interval.lower_bound * 100)"' "$estimates"
    fi
  done
}

# The benchmarks whose run `head` is slower than both base runs, printing
# what was measured.
slower() {
  local before after name lower_before lower_after
  before=$(changes "$1" base-before)
  after=$(changes "$1" base-after)
  while read -r name lower_before; do
    lower_after=$(awk -v n="$name" '$1 == n { print $2 }' <<<"$after")
    printf '%s (%s): at least %+.1f%% against the base run before, %+.1f%% against the run after\n' \
      "$name" "$1" "$lower_before" "$lower_after" >&2
    if awk -v a="$lower_before" -v b="$lower_after" -v t="$threshold" \
      'BEGIN { exit !(a > t && b > t) }'; then
      echo "$name"
    fi
  done <<<"$before"
}

bench "$base_dir" "$filter" --save-baseline base-before
bench "$root" "$filter" --save-baseline head
bench "$base_dir" "$filter" --save-baseline base-after

suspects=$(slower head)
if [[ -z $suspects ]]; then
  exit 0
fi
failed=0
for name in $suspects; do
  echo "$name looks more than $threshold% slower; running it again" >&2
  bench "$root" "^format/$name\$" --save-baseline head-again
  if [[ -n $(filter="^format/$name\$" slower head-again) ]]; then
    echo "error: $name is more than $threshold% slower than the base" >&2
    failed=1
  fi
done
exit "$failed"
