#!/bin/bash
# Usage: seed-home.sh <dir>
# Writes the fake home folder a verification instance runs in. Every file
# is generated here, so runs are identical on any Mac.
set -euo pipefail

home=${1:?usage: seed-home.sh <dir>}
[[ -e $home ]] && { echo "seed-home: $home already exists" >&2; exit 1; }
mkdir -p "$home"
cd "$home"

# An Office file is a zip of XML parts. $1 is the output, the rest are
# "part=content" pairs.
office() {
	local out=$1 work
	shift
	work=$(mktemp -d)
	for pair in "$@"; do
		mkdir -p "$work/$(dirname "${pair%%=*}")"
		printf '%s' "${pair#*=}" >"$work/${pair%%=*}"
	done
	(cd "$work" && zip -qX -r "$OLDPWD/$out" .)
	rm -rf "$work"
}

docx() {
	local out=$1 body=""
	shift
	for para in "$@"; do body+="<w:p><w:r><w:t>$para</w:t></w:r></w:p>"; done
	office "$out" \
		'[Content_Types].xml=<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>' \
		'_rels/.rels=<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>' \
		"word/document.xml=<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>$body</w:body></w:document>"
}

# One sheet per "Name|cell text|cell text..." argument, cells in column A.
xlsx() {
	local out=$1 sheets="" rels="" types="" parts=() n=0
	shift
	for spec in "$@"; do
		n=$((n + 1))
		IFS='|' read -r -a cells <<<"$spec"
		local rows="" r=0
		for cell in "${cells[@]:1}"; do
			r=$((r + 1))
			rows+="<row r=\"$r\"><c r=\"A$r\" t=\"inlineStr\"><is><t>$cell</t></is></c></row>"
		done
		sheets+="<sheet name=\"${cells[0]}\" sheetId=\"$n\" r:id=\"rId$n\"/>"
		rels+="<Relationship Id=\"rId$n\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet$n.xml\"/>"
		types+="<Override PartName=\"/xl/worksheets/sheet$n.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
		parts+=("xl/worksheets/sheet$n.xml=<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>$rows</sheetData></worksheet>")
	done
	office "$out" \
		"[Content_Types].xml=<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>$types</Types>" \
		'_rels/.rels=<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>' \
		"xl/workbook.xml=<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>$sheets</sheets></workbook>" \
		"xl/_rels/workbook.xml.rels=<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">$rels</Relationships>" \
		"${parts[@]}"
}

mkdir -p Documents/recipes Documents/sifs-verify-open-me
docx Documents/project-brief.docx 'Project Zanzibar launch plan' 'The pelican milestone is due in March.'
xlsx Documents/quarterly-budget.xlsx 'Budget|Item|marmalade imports|1200' 'Notes|Approved by the finance team'
printf 'Weekly sync.\nWe agreed the zanzibar timeline and the pelican budget.\n' >Documents/meeting-notes.txt
printf '# Lemon tart\n\nZest two lemons. Bake for 25 minutes.\n' >Documents/recipes/lemon-tart.md
printf 'A folder the open journey opens in Finder.\n' >Documents/sifs-verify-open-me/placeholder.txt
printf 'this is not a zip file\n' >Documents/broken-report.docx
printf 'Draft numbers for next year.\n' >Documents/budget-draft-2025.txt
printf 'Kept from SIFS on purpose.\n' >Documents/locked-notes.txt
chmod 000 Documents/locked-notes.txt
printf 'firmware\0\0\0\x01\x02 dump\n' >Documents/firmware-dump.txt
{
	printf '['
	for i in $(seq 1 30000); do printf '{"row":%d,"value":"export line"},' "$i"; done
	printf '{"row":0}]\n'
} >Documents/huge-export.json

mkdir -p Desktop/sifs-demo-code/src Desktop/sifs-demo-code/node_modules/left-pad Desktop/sifs-demo-code/generated
printf 'fn main() {\n    println!("pelican server starting");\n}\n' >Desktop/sifs-demo-code/src/main.rs
printf 'def serve():\n    return "zanzibar"\n' >Desktop/sifs-demo-code/src/server.py
printf '{ "name": "sifs-demo-code", "version": "1.0.0" }\n' >Desktop/sifs-demo-code/package.json
printf '# Demo code\n\nA small project for SIFS verification.\n' >Desktop/sifs-demo-code/README.md
printf 'module.exports = (s) => s;\n' >Desktop/sifs-demo-code/node_modules/left-pad/index.js
printf 'SECRET=hidden-by-default\n' >Desktop/sifs-demo-code/.env.local
printf 'team,amount\nplatform,4000\n' >Desktop/sifs-demo-code/team-budget.csv
printf 'var generatedbundle=1;\n' >Desktop/sifs-demo-code/generated/bundle-output.js

mkdir -p Downloads Projects/side-notes
printf 'Invoice 2041 for marmalade.\n' >Downloads/invoice-2041.txt
printf 'Ideas for the weekend.\n' >Projects/side-notes/ideas.txt

for f in Documents/project-brief.docx Documents/quarterly-budget.xlsx; do
	unzip -tq "$f" >/dev/null || { echo "seed-home: $f is not a valid zip" >&2; exit 1; }
done
echo "seeded $home"
