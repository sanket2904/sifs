#!/bin/sh
# Builds the Rust M1a extractor, runs it four times (directory on and off,
# text and text-ocr) into sample/runs/, and scores each run.
# Exit status: the highest scorer exit status of the four runs.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
cargo build --quiet --release --no-default-features --manifest-path "$root/src-tauri/Cargo.toml" --bin m1a_extract
bin="$root/src-tauri/target/release/m1a_extract"
mkdir -p "$root/sample/runs"
worst=0
for set in text text-ocr; do
	for dir in on off; do
		out="$root/sample/runs/rust-m1a-$dir-$set.json"
		if [ "$dir" = on ]; then
			"$bin" "$root/sample/$set" "$out" --directory "$root/sample/directory/banks.json"
		else
			"$bin" "$root/sample/$set" "$out"
		fi
		status=0
		node "$root/sample/tools/score.mjs" "$out" || status=$?
		[ "$status" -gt "$worst" ] && worst=$status
	done
done
exit "$worst"
