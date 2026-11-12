#!/usr/bin/env bash
# Packages the crate as `cargo publish` would, and runs the library's unit
# tests from the package, with the default features and with none: the
# package has only the files its `include` list names, and none of the
# repository's dev-dependency on itself that turns on `unstable`.
set -euo pipefail

cargo package --allow-dirty --no-verify --quiet

metadata=$(cargo metadata --format-version 1 --no-deps)
target=$(jq -r .target_directory <<<"$metadata")
version=$(jq -r '.packages[] | select(.name == "chairlift") | .version' <<<"$metadata")
unpacked="$target/package-test"
rm -rf "$unpacked"
mkdir -p "$unpacked"
tar -xzf "$target/package/chairlift-$version.crate" -C "$unpacked"
manifest="$unpacked/chairlift-$version/Cargo.toml"

# The repository's target directory, so that the dependencies built for it
# are reused.
export CARGO_TARGET_DIR="$target"
cargo test --manifest-path "$manifest" --lib
cargo test --manifest-path "$manifest" --lib --no-default-features
