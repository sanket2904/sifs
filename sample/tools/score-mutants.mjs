// Removes one scoring rule at a time, from score.mjs or a file it depends
// on, in a copy of sample/ and shows that score.test.mjs fails without it.
// Exits 1 if any removal goes unnoticed.
//   node sample/tools/score-mutants.mjs
import { spawnSync } from 'node:child_process';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const sample = fileURLToPath(new URL('../', import.meta.url));

// [file in sample/, rule, text in the file, what replaces it]
const MUTANTS = [
	[
		'tools/score.mjs',
		'a link must name the labelled organisation',
		'labelLinks.find((l) => l.org === g.t && same(l.ident, g.ident))',
		'labelLinks.find((l) => same(l.ident, g.ident))'
	],
	[
		'tools/score.mjs',
		'a not_link is reported as one',
		'const notLink = label.not_links.find(',
		'const notLink = [].find('
	],
	[
		'tools/score.mjs',
		'an invalid identifier is never linked',
		"g.ident.valid === false\n\t\t\t\t\t? 'the identifier is invalid'",
		"false\n\t\t\t\t\t? 'the identifier is invalid'"
	],
	[
		'tools/score.mjs',
		'a key over two organisations is a merge',
		'.filter((k) => targetsOf(k).size > 1)',
		'.filter(() => false)'
	],
	[
		'tools/score.mjs',
		'an organisation over two keys is a split',
		'.filter(([, ks]) => ks.length > 1)',
		'.filter(() => false)'
	],
	[
		'tools/score.mjs',
		'a missed link is left out',
		"add('links', lang, blocked ? 'impossible' : 'left_out', id, what);",
		''
	],
	[
		'tools/score.mjs',
		'a passport expiry is compared',
		"op.expiry === lp.expiry ? 'right' : 'wrong'",
		"'right'"
	],
	[
		'tools/score.mjs',
		'a latest statement must be the labelled document',
		'const bad = hits.filter((g) => g.document !== want.document);',
		'const bad = [];'
	],
	[
		'tools/score.mjs',
		'an unlabelled identifier is wrong',
		'for (const o of w.idents.unmatched) {',
		'for (const o of []) {'
	],
	[
		'tools/score.mjs',
		'a wrong link fails the run',
		"const FAILING = new Set(['links', ",
		'const FAILING = new Set(['
	],
	[
		'tools/score.mjs',
		'with the directory off no directory id is right',
		"const want = mode === 'off' ? null : directoryIdOf(t);",
		'const want = directoryIdOf(t);'
	],
	[
		'tools/score.mjs',
		'paired-code holds only through the labelled code',
		"(r !== 'paired-code' || (via && g.vias.some((v) => same(v, via))))",
		'true'
	],
	[
		'tools/score.mjs',
		'only a code, never an account, says who a key no mention lands on is',
		'if (!code || !CODE_KINDS.has(code.kind)) continue;',
		'if (!code) continue;'
	],
	[
		'tools/score.mjs',
		'a key its mentions settle is not remapped by its codes',
		'const byCode = new Map();',
		'const byCode = byMention;'
	],
	[
		'tools/score.mjs',
		"a card's name must be its organisation's",
		'if (name !== null && !nameFits(t, name))',
		'if (false)'
	],
	[
		'tools/score.mjs',
		"a card's directory id must be its organisation's",
		'if (id !== null && id !== directoryIdOf(t))',
		'if (false)'
	],
	[
		'tools/score.mjs',
		"a directory entry's names count only with the directory on",
		"mode === 'on'\n\t\t\t? directory.entries",
		'true\n\t\t\t? directory.entries'
	],
	[
		'tools/score.mjs',
		'a link to a wrong card is wrong',
		'!merged.has(key) && !wrongCard.has(key) ? single(key)',
		'!merged.has(key) ? single(key)'
	],
	['tools/score.mjs', 'a wrong card fails the run', 'wrongCards.length > 0 ||', ''],
	[
		'tools/score.mjs',
		'a key is a non-empty string',
		"if (!key) fail(where, 'a key must be a non-empty string');",
		''
	],
	[
		'tools/score.mjs',
		'a link counts only from an identifier that matched a labelled one',
		'const ident = w.labelOf.get(link.identifier) ?? null;',
		'const ident =\n\t\t\t\tw.labelOf.get(link.identifier) ??\n\t\t\t\tlabel.identifiers.find((l) => same(l, link.identifier)) ??\n\t\t\t\tnull;'
	],
	[
		'tools/score.mjs',
		'an identifier matches only with its kind and value',
		'if (same(l, o)) labelOf.set(o, l);',
		'labelOf.set(o, l);'
	],
	[
		'tools/score.mjs',
		'a via counts only when it matched a labelled code',
		'const via = link.via && w.labelOf.get(link.via);',
		'const via = link.via;'
	],
	[
		'tools/score.mjs',
		'rows for one link hold the union of their reasons',
		'for (const r of link.reasons) g.reasons.add(r);',
		'if (!g.reasons.size) for (const r of link.reasons) g.reasons.add(r);'
	],
	[
		'tools/score.mjs',
		'equal texts pair by position on the page',
		'for (const o of [...output].sort(byPosition)) {',
		'for (const o of output) {'
	],
	[
		'tools/score.mjs',
		'two mentions on one span break the format',
		'oneRowPerSpan(mentions, `${where}.orgs`);',
		''
	],
	[
		'tools/score.mjs',
		'two identifiers on one span break the format',
		'oneRowPerSpan([...identifiers.values()], `${where}.identifiers`);',
		''
	],
	['tools/score.mjs', 'a span starts inside its page', 's.start < 0 ||', ''],
	['tools/score.mjs', 'a span ends inside its page', 's.end > length', 'false'],
	[
		'tools/score.mjs',
		'validity is not scored where the label gives null',
		"const fields = l.valid === null ? ['kind', 'value'] : ['kind', 'value', 'valid'];",
		"const fields = ['kind', 'value', 'valid'];"
	],
	[
		'tools/score.mjs',
		'a date is a day of the calendar',
		'return day.getUTCFullYear() === y && day.getUTCMonth() === m - 1 && day.getUTCDate() === d;',
		'return true;'
	],
	[
		'tools/score.mjs',
		'a passport issued date is checked',
		'if (p.issued !== undefined && p.issued !== null && !isDate(p.issued))',
		'if (false)'
	],
	[
		'tools/score.mjs',
		'a passport mrz names an identifier',
		'const mrz = optionalIdent(p.mrz, `${w}.mrz`);',
		'const mrz = null;'
	],
	[
		'tools/score.mjs',
		'a passport expiry_at reads back',
		': span(p.expiry_at, pages, `${w}.expiry_at`);',
		': p.expiry_at;'
	],
	[
		'tools/score.mjs',
		'on the OCR text set, a page the OCR did not read matches by span',
		"out.textSet !== 'text' && pagesOf(docId)[page - 1].scanned",
		"out.textSet !== 'text'"
	],
	[
		'tools/score.mjs',
		'a keyed mention on a not_orgs span is a wrong attachment',
		'if (excluded)\n\t\t\t\twrongAttachments.push',
		'if (false)\n\t\t\t\twrongAttachments.push'
	],
	[
		'tools/score.mjs',
		'only the enclosing name’s own card excuses a mention on a not_orgs span',
		'(n) => t === null || !enclosing(n).some((l) => target(l) === t)',
		'() => true'
	],
	[
		'tools/score.mjs',
		'a wrong attachment does not depend on directory ids',
		'(n) => t === null || !enclosing(n).some((l) => target(l) === t)',
		'(n) => [...targetsOf(o.org)].some((x) => n.entry === directoryIdOf(x))'
	],
	['tools/score.mjs', 'a wrong attachment fails the run', 'wrongAttachments.length > 0 ||', ''],
	[
		'tools/score.mjs',
		'a card name ignores all whitespace',
		".toUpperCase().replace(/\\s/gu, '');",
		".toUpperCase().replace(/\\s+/gu, ' ').trim();"
	],
	[
		'tools/score.mjs',
		'a root that is not an object breaks the format',
		'TEXT_SETS.includes(output?.text_set)',
		'TEXT_SETS.includes(output.text_set)'
	],
	[
		'tools/score.mjs',
		'paired-code needs a via',
		"if (link.reasons.includes('paired-code') && (link.via === undefined || link.via === null))",
		'if (false)'
	],
	[
		'tools/lib/main.mjs',
		'a tool started through a symlinked path runs',
		'realpathSync(process.argv[1]) === fileURLToPath(url)',
		'process.argv[1] === fileURLToPath(url)'
	],
	[
		'tools/text.mjs',
		'--check hashes the files, not SHA256SUMS',
		'sha256: fileHash(file),',
		"sha256: readFileSync(new URL('SHA256SUMS', FILES), 'utf8')\n\t\t\t\t.split('\\n')\n\t\t\t\t.find((line) => line.endsWith(`  ${file}`))\n\t\t\t\t.slice(0, 64),"
	],
	['FORMAT.md', 'the worked example must hold', '"value": "1234567890"', '"value": "4512369874"'],
	['FORMAT.md', 'the sort code example must hold', '"value": "601613"', '"value": "60-16-13"']
];

let missed = 0;
for (const [file, rule, from, to] of MUTANTS) {
	const dir = mkdtempSync(join(tmpdir(), 'score-mutant-'));
	try {
		cpSync(sample, dir, { recursive: true });
		const path = join(dir, file);
		const source = readFileSync(path, 'utf8');
		if (source.split(from).length !== 2)
			throw new Error(`"${rule}": its text is not in ${file} exactly once`);
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
