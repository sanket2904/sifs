#!/bin/sh
# Builds the engine's tools into the folder the app looks in (SIFS_TOOLS_DIR,
# or tools in its data folder): m1a-extract, sifs-load and sifs-show, with
# the bank directory and the IBAN registry the extractor reads. All or
# nothing: when a build fails, the folder is left as it was.
#
#	sample/tools/build-tools.sh <folder>
#
# sifs-load and sifs-show are built from engine/link, which needs a private
# module; where that cannot be fetched, nothing is put in the folder.
set -eu
if [ $# -ne 1 ] || [ -z "$1" ]; then
	echo "usage: $0 <folder>" >&2
	exit 2
fi
root=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "$(dirname "$1")"
out=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
aside=$(mktemp -d "$(dirname "$out")/.build-tools.XXXXXX")
trap 'rm -rf "$aside"' EXIT

failed() {
	echo "$0: could not build $1: $(tail -n 1 "$aside/log")" >&2
	exit 1
}
(cd "$root/engine" && GOWORK=off go build -o "$aside/" ./cmd/m1a-extract) >"$aside/log" 2>&1 ||
	failed m1a-extract
(cd "$root/engine/link" && GOWORK=off GOPRIVATE=github.com/sanket2904/ontology-framework \
	go build -o "$aside/" ./cmd/sifs-load ./cmd/sifs-show) >"$aside/log" 2>&1 ||
	failed "sifs-load and sifs-show"
rm "$aside/log"
cp "$root/sample/directory/banks.json" "$root/sample/iban-registry.json" "$aside/"
chmod 755 "$aside"

if [ -e "$out" ]; then
	for piece in m1a-extract sifs-load sifs-show banks.json iban-registry.json; do
		mv -f "$aside/$piece" "$out/$piece"
	done
else
	mv "$aside" "$out"
fi
echo "the tools are in $out"
