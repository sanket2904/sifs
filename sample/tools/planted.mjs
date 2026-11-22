// Lists, per document, every organisation and identifier spec.json says was
// put in the text, with the label that holds each, and fails on any planted
// token without an exact label, any label that is not planted, and any
// organisation- or identifier-shaped text that nothing accounts for.
// check.mjs runs the same proof.
//   node sample/tools/planted.mjs
import { loadSpec, pageText, SAMPLE } from './generate.mjs';
import { readLabels } from './lib/labels.mjs';
import { planted } from './lib/planted.mjs';

const labels = readLabels(new URL('labels.json', SAMPLE));
const byId = new Map(labels.documents.map((doc) => [doc.id, doc]));
let failures = 0;
let tokens = 0;
for (const source of loadSpec().documents) {
	const doc = byId.get(source.id) ?? {};
	const { rows, problems } = planted(
		source,
		doc,
		source.pages.map((page) => [...pageText(page)])
	);
	console.log(source.id);
	for (const row of rows) {
		tokens++;
		console.log(
			`  ${row.kind.padEnd(10)} ${JSON.stringify(row.text).padEnd(52)} p${row.page} ${row.start}..${row.end}  ${row.label ?? 'UNLABELLED'}`
		);
	}
	for (const problem of problems) console.log(`  FAIL ${problem}`);
	failures += problems.length;
}
console.log(failures ? `${failures} failures` : `ok: ${tokens} planted tokens, each labelled`);
process.exit(failures ? 1 : 0);
