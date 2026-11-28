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
# A kill that cannot be caught (SIGKILL) mid-swap can leave the folder missing
# beside .build-tools.* or .build-tools.*.old; run it again to build it afresh.
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
prev=""
interrupted=""
cleanup() {
	trap - EXIT
	set +e
	case "$aside" in
		"$parent/.build-tools."*) ;;
		*) aside="" ;;
	esac
	if [ -z "$interrupted" ]; then
		if [ -n "$aside" ] && [ -e "$aside" ]; then
			if ! rm_err=$(rm -rf -- "$aside" 2>&1); then
				msg=$(printf '%s\n' "$rm_err" | tail -n 1)
				if [ -n "$msg" ]; then
					echo "$0: could not remove staging folder $aside: $msg" >&2
				else
					echo "$0: could not remove staging folder $aside" >&2
				fi
			fi
		fi
		return 0
	fi

	if [ -n "$prev" ] && [ -e "$prev" ]; then
		if [ ! -e "$out" ]; then
			if mv_err=$(mv -- "$prev" "$out" 2>&1); then
				if [ -n "$aside" ] && [ -e "$aside" ]; then
					if ! rm_err=$(rm -rf -- "$aside" 2>&1); then
						echo "$0: interrupted; restored previous tools in $out, but could not remove staging folder $aside: $(printf '%s\n' "$rm_err" | tail -n 1)" >&2
						return 0
					fi
				fi
				echo "$0: interrupted; restored previous tools in $out" >&2
			else
				mv_msg=$(printf '%s\n' "$mv_err" | tail -n 1)
				if [ -n "$aside" ] && [ -e "$aside" ]; then
					if ! rm_err=$(rm -rf -- "$aside" 2>&1); then
						echo "$0: interrupted; could not restore $out from $prev ($mv_msg) and could not remove staging folder $aside ($(printf '%s\n' "$rm_err" | tail -n 1))" >&2
						return 0
					fi
				fi
				echo "$0: interrupted; could not restore $out from $prev: $mv_msg" >&2
			fi
		else
			if ! rm_err=$(rm -rf -- "$prev" 2>&1); then
				echo "$0: interrupted; $out holds the new tools, but could not remove old folder $prev: $(printf '%s\n' "$rm_err" | tail -n 1)" >&2
				return 0
			fi
			echo "$0: interrupted; $out holds the new tools" >&2
		fi
	elif [ -n "$aside" ] && [ -e "$aside" ]; then
		if [ -e "$out" ]; then
			state="$out still holds the previous tools"
		else
			state="$out was not created"
		fi
		if ! rm_err=$(rm -rf -- "$aside" 2>&1); then
			echo "$0: interrupted; $state, but could not remove staging folder $aside: $(printf '%s\n' "$rm_err" | tail -n 1)" >&2
			return 0
		fi
		echo "$0: interrupted; $state" >&2
	elif [ -n "$aside" ] && [ -e "$out" ]; then
		echo "$0: interrupted; $out holds the new tools" >&2
	elif [ -e "$out" ]; then
		echo "$0: interrupted; $out still holds the previous tools" >&2
	else
		echo "$0: interrupted; $out was not created" >&2
	fi
}
on_signal() {
	interrupted=$1
	trap '' INT TERM HUP
	cleanup
	exit "$2"
}
trap cleanup EXIT
trap 'on_signal INT 130' INT
trap 'on_signal TERM 143' TERM
trap 'on_signal HUP 129' HUP

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
	backup="${aside}.old"
	if [ -e "$backup" ] || [ -L "$backup" ]; then
		echo "$0: backup path $backup already exists" >&2
		exit 1
	fi
	prev="$backup"
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
	if ! err=$(rm -rf -- "$prev" 2>&1); then
		echo "$0: updated $out, but could not remove old folder $prev: $(printf '%s\n' "$err" | tail -n 1)" >&2
		exit 1
	fi
else
	if ! err=$(mv -- "$aside" "$out" 2>&1); then
		echo "$0: could not put new folder $out in place: $(printf '%s\n' "$err" | tail -n 1)" >&2
		exit 1
	fi
fi
echo "the tools are in $out"
