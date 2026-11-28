#!/bin/sh
# Builds the engine's tools into the folder the app looks in (SIFS_TOOLS_DIR,
# or tools in its data folder): m1a-extract, sifs-load and sifs-show, with
# the bank directory and the IBAN registry the extractor reads.
#
# Whole folder or nothing: it builds into a sibling staging folder, then
# swaps the whole folder into place and removes the old one. If any build or
# move fails, the folder is left as it was. Because it removes the old
# folder, it refuses an existing path that is not a folder or holds anything
# other than those five files (exit 2, before building).
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
root=$(cd -- "$(dirname -- "$0")/../.." && pwd)
dir=$(dirname -- "$1")
name=$(basename -- "$1")
if [ "$name" = "/" ] || [ "$name" = "." ] || [ "$name" = ".." ]; then
	echo "$0: $1 is not a folder name; give the tools a folder of their own" >&2
	exit 2
fi
if ! err=$(mkdir -p -- "$dir" 2>&1); then
	echo "$0: could not create parent folder $dir: $(printf '%s\n' "$err" | tail -n 1)" >&2
	exit 1
fi
parent=$(cd -- "$dir" && pwd)
case "$parent" in
	/) out="/$name" ;;
	*) out="$parent/$name" ;;
esac

if [ -L "$out" ] || { [ -e "$out" ] && [ ! -d "$out" ]; }; then
	echo "$0: $out is not a folder; give the tools a folder of their own" >&2
	exit 2
fi
if [ -d "$out" ]; then
	if [ ! -r "$out" ] || [ ! -x "$out" ]; then
		echo "$0: cannot inspect $out" >&2
		exit 1
	fi
	for entry in "$out"/* "$out"/.[!.]* "$out"/..?*; do
		[ -e "$entry" ] || [ -L "$entry" ] || continue
		piece=${entry##*/}
		case "$piece" in
			m1a-extract|sifs-load|sifs-show|banks.json|iban-registry.json)
				if [ -L "$entry" ] || [ ! -f "$entry" ]; then
					echo "$0: $out holds something other than the tools; give the tools a folder of their own" >&2
					exit 2
				fi
				;;
			*)
				echo "$0: $out holds something other than the tools; give the tools a folder of their own" >&2
				exit 2
				;;
		esac
	done
fi

aside=""
cleanup() {
	if [ -n "$aside" ] && [ -e "$aside" ]; then
		rm -rf -- "$aside" >/dev/null 2>&1 || true
	fi
}
trap cleanup EXIT

if ! aside=$(mktemp -d "$parent/.build-tools.XXXXXX" 2>&1); then
	echo "$0: could not create staging folder in $parent: $(printf '%s\n' "$aside" | tail -n 1)" >&2
	aside=""
	exit 1
fi

failed() {
	msg=$(tail -n 1 "$aside/log" 2>/dev/null || true)
	if [ -n "$msg" ]; then
		echo "$0: could not build $1: $msg" >&2
	else
		echo "$0: could not build $1" >&2
	fi
	exit 1
}
(cd -- "$root/engine" && GOWORK=off go build -o "$aside/" ./cmd/m1a-extract) >"$aside/log" 2>&1 ||
	failed m1a-extract
(cd -- "$root/engine/link" && GOWORK=off GOPRIVATE=github.com/sanket2904/ontology-framework \
	go build -o "$aside/" ./cmd/sifs-load ./cmd/sifs-show) >"$aside/log" 2>&1 ||
	failed "sifs-load and sifs-show"
rm -- "$aside/log"
if ! err=$(cp -- "$root/sample/directory/banks.json" "$root/sample/iban-registry.json" "$aside/" 2>&1); then
	echo "$0: could not copy data files: $(printf '%s\n' "$err" | tail -n 1)" >&2
	exit 1
fi
chmod 755 "$aside"

if [ -d "$out" ]; then
	prev="${aside}.old"
	if [ -e "$prev" ] || [ -L "$prev" ]; then
		echo "$0: backup path $prev already exists" >&2
		exit 1
	fi
	if ! err=$(mv -- "$out" "$prev" 2>&1); then
		echo "$0: could not move old folder $out aside: $(printf '%s\n' "$err" | tail -n 1)" >&2
		exit 1
	fi
	if ! err=$(mv -- "$aside" "$out" 2>&1); then
		if ! restore=$(mv -- "$prev" "$out" 2>&1); then
			echo "$0: could not put new folder $out in place ($(printf '%s\n' "$err" | tail -n 1)) and could not restore from $prev ($(printf '%s\n' "$restore" | tail -n 1))" >&2
			exit 1
		fi
		echo "$0: could not put new folder $out in place: $(printf '%s\n' "$err" | tail -n 1)" >&2
		exit 1
	fi
	aside=""
	if ! err=$(rm -rf -- "$prev" 2>&1); then
		echo "$0: updated $out, but could not remove old folder $prev: $(printf '%s\n' "$err" | tail -n 1)" >&2
		exit 1
	fi
else
	if ! err=$(mv -- "$aside" "$out" 2>&1); then
		echo "$0: could not put new folder $out in place: $(printf '%s\n' "$err" | tail -n 1)" >&2
		exit 1
	fi
	aside=""
fi
echo "the tools are in $out"
