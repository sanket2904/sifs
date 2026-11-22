// Removes one scoring rule at a time from score.mjs in a copy of sample/ and
// shows that score.test.mjs fails without it. Exits 1 if any removal goes
// unnoticed.
//   node sample/tools/score-mutants.mjs
import { spawnSync } from 'node:child_process';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const sample = fileURLToPath(new URL('../', import.meta.url));

// [rule, text in score.mjs, what replaces it]
const MUTANTS = [
	[
		'a link must name the labelled organisation',
		'labelLinks.find((l) => l.org === t && same(l.ident, oi))',
		'labelLinks.find((l) => same(l.ident, oi))'
	],
	[
		'a not_link is reported as one',
		'const notLink =\n\t\t\t\tli && label.not_links',
		'const notLink =\n\t\t\t\tfalse && label.not_links'
	],
	[
		'an invalid identifier is never linked',
		": li.valid === false\n\t\t\t\t\t\t? 'the identifier is invalid'",
		": false\n\t\t\t\t\t\t? 'the identifier is invalid'"
	],
	[
		'a key over two organisations is a merge',
		'.filter((k) => targetsOf(k).size > 1)',
		'.filter(() => false)'
	],
	[
		'an organisation over two keys is a split',
		'.filter(([, ks]) => ks.length > 1)',
		'.filter(() => false)'
	],
	[
		'a missed link is left out',
		"add('links', lang, blocked ? 'impossible' : 'left_out', id, what);",
		''
	],
	['a passport expiry is compared', "op.expiry === lp.expiry ? 'right' : 'wrong'", "'right'"],
	[
		'a latest statement must be the labelled document',
		'const bad = hits.filter((g) => g.document !== want.document);',
		'const bad = [];'
	],
	[
		'an unlabelled identifier is wrong',
		'for (const o of w.idents.unmatched) {',
		'for (const o of []) {'
	],
	['a wrong link fails the run', "const FAILING = new Set(['links', ", 'const FAILING = new Set(['],
	[
		'with the directory off no directory id is right',
		"const want =\n\t\t\tmode === 'off'\n\t\t\t\t? null\n\t\t\t\t:",
		'const want ='
	],
	[
		'paired-code holds only through the labelled code',
		"(r !== 'paired-code' || (link.via && same(w.identById.get(right.via), link.via)))",
		'true'
	]
];

let missed = 0;
for (const [rule, from, to] of MUTANTS) {
	const dir = mkdtempSync(join(tmpdir(), 'score-mutant-'));
	try {
		cpSync(sample, dir, { recursive: true });
		const path = join(dir, 'tools', 'score.mjs');
		const source = readFileSync(path, 'utf8');
		if (source.split(from).length !== 2)
			throw new Error(`"${rule}": its text is not in score.mjs exactly once`);
		writeFileSync(path, source.replace(from, to));
		const run = spawnSync(
			process.execPath,
			['--test', '--test-reporter=tap', join(dir, 'tools', 'score.test.mjs')],
			{
				encoding: 'utf8'
			}
		);
		const failed = [...run.stdout.matchAll(/^not ok \d+ - (.*)$/gm)].map((m) => m[1]);
		if (run.status === 0) {
			missed++;
			console.log(`MISSED  ${rule}`);
		} else console.log(`caught  ${rule}\n        by: ${failed.join('; ')}`);
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
}
console.log(
	missed
		? `\n${missed} of ${MUTANTS.length} removals went unnoticed`
		: `\nall ${MUTANTS.length} removals caught`
);
process.exit(missed ? 1 : 0);
