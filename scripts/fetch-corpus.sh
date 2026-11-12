#!/usr/bin/env bash
# Fetches the public corpora chairlift is tested against.
#
# Everything lands in $CHAIRLIFT_CORPUS_DIR, by default `corpus/` in the
# repository, so several checkouts can share one copy:
#
#   ClickHouse/   ClickHouse's source at $CLICKHOUSE_TAG, sparse: the stateless
#                 tests, src/Parsers (keyword list and lexer) and the
#                 reference documentation
#   dbt/<repo>/   dbt-clickhouse projects, pinned by commit
#
# The docs-examples corpus, docs/, is extracted from the documentation by
# `corpus --extract-docs`, which needs clickhouse-format; `mise run
# corpus:fetch` runs both.
#
# The dbt projects carry no licence that allows redistribution, so they are
# fetched, never vendored. Re-running is a no-op when everything is in place;
# changing a pin below moves the existing checkout to it.
set -euo pipefail

CLICKHOUSE_URL=https://github.com/ClickHouse/ClickHouse
# Keep in step with `CLICKHOUSE_VERSION` in src/lib.rs and the clickhouse pin
# in mise.toml.
CLICKHOUSE_TAG=v26.10.2.103-stable
CLICKHOUSE_PATHS=(docs/reference src/Parsers tests/queries/0_stateless)

# owner/repo and full commit hash.
DBT_REPOS=(
    "kzzzr/dbt_clickhouse_lab e63605285f151a7eca144fea7182826dc65e32d1"
    "SaiPurushoth/dbt-clickhouse-accelerator deba7469d7fa402dd1b722420378a68ddbf8315d"
    "tysee/norn 86a29a176ebaf469756783b0af98d129d9d80823"
)

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
corpus_dir=${CHAIRLIFT_CORPUS_DIR:-$repo_root/corpus}
mkdir -p "$corpus_dir"

fetch_clickhouse() {
    local dir=$corpus_dir/ClickHouse
    if [[ ! -d $dir/.git ]]; then
        echo "Cloning ClickHouse $CLICKHOUSE_TAG into $dir"
        git -c advice.detachedHead=false clone --quiet --depth 1 --filter=blob:none --sparse \
            --branch "$CLICKHOUSE_TAG" "$CLICKHOUSE_URL" "$dir"
    elif [[ $(git -C "$dir" describe --tags --exact-match HEAD 2>/dev/null) != "$CLICKHOUSE_TAG" ]]; then
        echo "Moving ClickHouse to $CLICKHOUSE_TAG"
        git -C "$dir" fetch --quiet --depth 1 --filter=blob:none origin tag "$CLICKHOUSE_TAG"
        git -C "$dir" checkout --quiet --detach "$CLICKHOUSE_TAG"
    fi
    # Paths are added, never removed, so a checkout shared between
    # checkouts of different revisions keeps what each of them needs.
    local listed missing=()
    listed=$(git -C "$dir" sparse-checkout list)
    for path in "${CLICKHOUSE_PATHS[@]}"; do
        if ! grep -qxF "$path" <<<"$listed"; then
            missing+=("$path")
        fi
    done
    if ((${#missing[@]})); then
        echo "Adding ${missing[*]} to the ClickHouse sparse checkout"
        git -C "$dir" sparse-checkout add "${missing[@]}"
    fi
}

fetch_pinned() {
    local slug=$1 commit=$2
    local dir=$corpus_dir/dbt/${slug#*/}
    if [[ $(git -C "$dir" rev-parse HEAD 2>/dev/null) == "$commit" ]]; then
        return
    fi
    echo "Fetching $slug at $commit into $dir"
    if [[ ! -d $dir/.git ]]; then
        git init --quiet "$dir"
        git -C "$dir" remote add origin "https://github.com/$slug"
    fi
    git -C "$dir" fetch --quiet --depth 1 --filter=blob:none origin "$commit"
    git -C "$dir" checkout --quiet --detach FETCH_HEAD
}

fetch_clickhouse
for entry in "${DBT_REPOS[@]}"; do
    # shellcheck disable=SC2086 # split "owner/repo commit" into two arguments
    fetch_pinned $entry
done

stateless=$corpus_dir/ClickHouse/tests/queries/0_stateless
if [[ ! -d $stateless ]]; then
    echo "error: $stateless is missing after the fetch" >&2
    exit 1
fi
echo "Corpus in $corpus_dir:"
echo "  stateless tests: $(find "$stateless" -maxdepth 1 -name '*.sql' | wc -l) .sql, $(find "$stateless" -maxdepth 1 -name '*.sql.j2' | wc -l) .sql.j2"
echo "  dbt projects:    $(find "$corpus_dir/dbt" -name '*.sql' -not -path '*/.git/*' | wc -l) .sql"
echo "  documentation:   $(find "$corpus_dir/ClickHouse/docs/reference" -name '*.mdx' | wc -l) .mdx"
