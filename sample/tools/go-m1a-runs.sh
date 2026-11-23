#!/bin/sh
# Builds the Go extractor in engine/, makes its four runs (directory on and
# off, on text/ and text-ocr/) under sample/runs/, and scores each one.
# Exits non-zero when any run fails the bar.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
bin=$(mktemp -d)/m1a-extract
(cd "$root/engine" && GOWORK=off go build -o "$bin" ./cmd/m1a-extract)
mkdir -p "$root/sample/runs"
status=0
for set in text text-ocr; do
	for mode in on off; do
		out="$root/sample/runs/go-m1a-$mode-$set.json"
		if [ "$mode" = on ]; then
			"$bin" -text "$root/sample/$set" -directory "$root/sample/directory/banks.json" -out "$out"
		else
			"$bin" -text "$root/sample/$set" -out "$out"
		fi
		code=0
		node "$root/sample/tools/score.mjs" "$out" || code=$?
		echo "go-m1a-$mode-$set: score exit $code"
		[ "$code" -eq 0 ] || status=$code
	done
done
exit $status
