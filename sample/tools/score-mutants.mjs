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
		"...(organisations.length > 1 ? [{ key: k, why: 'organisations', organisations }] : []),",
		''
	],
	[
		'tools/score.mjs',
		"a link by its code's own reason goes to the code's key",
		'if (own && canonical !== null && link.org !== canonical)',
		'if (false)'
	],
	[
		'tools/lib/tiers.mjs',
		"a BIC's institution is its first eight characters",
		'return `BIC ${v.slice(0, 8)}`;',
		'return `BIC ${v.slice(0, 6)}`;'
	],
	[
		'tools/score.mjs',
		'a key holding a code and a printed name is a merge',
		"...(codeKeys.has(k) && named.has(k) ? [{ key: k, why: 'name-and-code' }] : [])",
		''
	],
	[
		'tools/score.mjs',
		"only a code's own reason gives a key its code",
		'if (l.reasons.some((r) => CODE_REASONS.get(r) === l.identifier.kind) && codeKey(l.identifier))',
		'if (codeKey(l.identifier))'
	],
	[
		'tools/score.mjs',
		'an IBAN whose check digits pass is held at its code, whatever the label lists',
		'ownIban(r) ||',
		''
	],
	[
		'tools/score.mjs',
		'an identifier is certain only where its kind may be',
		"if (tier === 'certain' && !mayBeCertainIdentifier(identifier, output.directory, entries))",
		'if (false)'
	],
	[
		'tools/lib/tiers.mjs',
		'a card number is never certain',
		'\treturn false;\n}\n\n// A link may be',
		"\treturn kind === 'card' && KINDS.card.valid(value);\n}\n\n// A link may be"
	],
	[
		'tools/score.mjs',
		'an identifier is counted in its tier',
		': what,\n\t\t\t\t\to.tier\n',
		': what\n'
	],
	[
		'tools/score.mjs',
		'a certain join stands a card up',
		"for (const r of d.sameOrg) if (r.tier === 'certain') r.keys.forEach((k) => certainKeys.add(k));",
		''
	],
	[
		'tools/score.mjs',
		'cards on likely rows only are listed',
		'likely_only_cards: keys.filter((k) => !certainKeys.has(k)).sort(),',
		'likely_only_cards: [],'
	],
	[
		'tools/score.mjs',
		'only a merge of two organisations makes rows wrong',
		"merges.filter((m) => m.why === 'organisations').map((m) => m.key)",
		'merges.map((m) => m.key)'
	],
	['tools/score.mjs', 'a merge fails the run', 'merges.length > 0 ||', ''],
	[
		'tools/score.mjs',
		'a wrong certain row fails the run',
		"items.wrong.some((i) => i.tier === 'certain')",
		'false'
	],
	[
		'tools/score.mjs',
		'a wrong likely row fails nothing',
		"items.wrong.some((i) => i.tier === 'certain')",
		'items.wrong.some((i) => i.tier !== null)'
	],
	[
		'tools/score.mjs',
		'one certain row makes a link certain',
		"if (link.tier === 'certain') g.tier = 'certain';",
		''
	],
	[
		'tools/score.mjs',
		'each measure counts right and wrong per tier',
		'if (tier) (row.by_tier[tier] ??= { right: 0, wrong: 0 })[outcome]++;',
		''
	],
	[
		'tools/score.mjs',
		'format 1 is refused with what to add',
		'if (output.format === 1)',
		'if (false)'
	],
	[
		'tools/score.mjs',
		'a tier is certain or likely',
		'TIERS.has(v) ? v : fail(',
		'true ? v : fail('
	],
	[
		'tools/score.mjs',
		'a statement and a passport give their tiers',
		'if (!isObject(tiers))',
		'if (false)'
	],
	[
		'tools/score.mjs',
		'a certain link needs a reason that passes its test',
		"if (tier === 'certain' && !mayBeCertainLink(test, output.directory, entries))",
		'if (false)'
	],
	[
		'tools/lib/tiers.mjs',
		'an IBAN or a zone is certain only when its check digits pass',
		"if (kind === 'iban' || kind === 'mrz') return KINDS[kind].valid(value) === true;",
		"if (kind === 'iban' || kind === 'mrz') return true;"
	],
	[
		'tools/lib/tiers.mjs',
		'an IFSC or BIC is certain only with the directory on',
		"return mode === 'on' && KINDS[kind].valid(value) && codeLookup(entries, identifier).length > 0;",
		'return KINDS[kind].valid(value) && codeLookup(entries, identifier).length > 0;'
	],
	[
		'tools/lib/tiers.mjs',
		"an IFSC or BIC is certain only when the card's entry holds it",
		'if (!entry) return false;',
		'return true;'
	],
	[
		'tools/lib/tiers.mjs',
		'a BIC is certain only on its first eight characters',
		'b.slice(0, 8) === v.slice(0, 8)',
		'b.slice(0, 6) === v.slice(0, 6)'
	],
	[
		'tools/score.mjs',
		'no statement fact is certain',
		'const tiers = factTiers(s.tiers, facts, w, () => false);',
		'const tiers = factTiers(s.tiers, facts, w, () => true);'
	],
	[
		'tools/lib/tiers.mjs',
		'passport current is never certain',
		"if (fact === 'current') return false;",
		''
	],
	[
		'tools/lib/tiers.mjs',
		'a passport fact is certain only through a passport zone whose check digits pass',
		"if (zone?.type !== 'passport' || zone.failed.length) return false;",
		'if (zone === null) return false;'
	],
	[
		'tools/lib/tiers.mjs',
		"a certain passport number is the zone's number",
		"if (fact === 'number') return number?.value === zone.number;",
		"if (fact === 'number') return true;"
	],
	[
		'tools/lib/tiers.mjs',
		'a certain passport fact rests on a certain zone',
		"if (mrz?.tier !== 'certain') return false;",
		''
	],
	[
		'tools/lib/tiers.mjs',
		"a certain passport holder is the zone's holder",
		'return name(holder.surname) === name(zone.surname) && name(holder.given) === name(zone.given);',
		'return true;'
	],
	[
		'tools/lib/tiers.mjs',
		"a certain passport expiry is the zone's expiry",
		'return printedExpiry(expiryAt.text, zone.expiry) === expiry;',
		'return true;'
	],
	[
		'iban-registry.json',
		'an IBAN of a country in the registry table is certain',
		'\t\t"AT": {"length": 20, "bank_identifier": "1-5"},\n',
		''
	],
	[
		'tools/lib/ids.mjs',
		"an IBAN's bank code sits where the registry table puts it",
		'bank: [4 + first - 1, 4 + last]',
		'bank: [4, 4 + last - first + 1]'
	],
	[
		'tools/lib/ids.mjs',
		"an IBAN has its country's length",
		'if (country.length !== v.length)',
		'if (false)'
	],
	[
		'tools/score.mjs',
		'refusing a certain IBAN says why',
		"return problem ? `: ${problem}` : '';",
		"return '';"
	],
	[
		'tools/score.mjs',
		'a same_org row is right only when both keys are one organisation',
		'if (ta !== null && ta === tb) {',
		'if (ta !== null) {'
	],
	[
		'tools/score.mjs',
		'a missing join is left out',
		'for (let n = 1; n < apart; n++)',
		'for (let n = 1; n < 1; n++)'
	],
	[
		'tools/score.mjs',
		'joins chain through each other',
		'if (root(a) !== root(b)) joinedTo.set(root(a), root(b));',
		''
	],
	[
		'tools/score.mjs',
		'same_org rows with one id are one claim',
		'const k = JSON.stringify(row.keys);',
		'const k = JSON.stringify([...row.keys, row.reason]);'
	],
	[
		'tools/score.mjs',
		'two same_org claims never share an id',
		'const k = JSON.stringify(row.keys);',
		"const k = row.keys.join('\\0');"
	],
	[
		'tools/score.mjs',
		'same_org claims are scored in one order',
		'[...claims.keys()].sort()',
		'[...claims.keys()]'
	],
	[
		'tools/score.mjs',
		'a same_org row joins keys its document uses',
		'if (!used.has(k))',
		'if (false)'
	],
	[
		'tools/score.mjs',
		'a same_org row joins two different keys',
		'row.keys.length !== 2 || row.keys[0] === row.keys[1]',
		'row.keys.length !== 2'
	],
	[
		'tools/score.mjs',
		'a same_org reason is known',
		'if (!SAME_ORG_REASONS.has(row.reason))',
		'if (false)'
	],
	[
		'tools/score.mjs',
		'only a directory row may be certain',
		"if (tier === 'certain' && !join(keys[0], keys[1]) && !join(keys[1], keys[0]))",
		'if (false)'
	],
	[
		'tools/score.mjs',
		'a certain join reads the directory, never the ids the cards give',
		'!join(keys[0], keys[1]) && !join(keys[1], keys[0])',
		'cards.get(keys[0]).directory !== cards.get(keys[1]).directory'
	],
	[
		'tools/lib/tiers.mjs',
		'a certain directory row needs the directory on',
		"if (reason !== 'directory' || mode !== 'on' ||",
		"if (reason !== 'directory' ||"
	],
	[
		'tools/lib/tiers.mjs',
		'a certain join needs exactly one entry for the name and for the code',
		'ids.length === 1 && ids[0] === found[0][0]',
		'ids.includes(found[0][0])'
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
	[
		'FORMAT.md',
		'the worked example must hold',
		'"value": "50100417263958"',
		'"value": "4512369874"'
	],
	[
		'FORMAT.md',
		'the worked example joins its two keys',
		'"same_org": [{"keys": ["name:hdfc-statement-sep:hdfcbank", "IFSC HDFC"], "reason": "directory", "tier": "certain"}]',
		'"same_org": []'
	],
	['FORMAT.md', 'the sort code example must hold', '"value": "601613"', '"value": "60-16-13"'],
	[
		'tools/score.mjs',
		'a key is a code key or a document key',
		'if (!isCodeKey(key) && !textSet.docs.has(documentOfKey(key)))',
		'if (false)'
	],
	[
		'tools/lib/tiers.mjs',
		"an IBAN code key has the length of its country's bank identifier",
		'return iban !== null && ibanBankCodeLength(iban[1]) === iban[2].length;',
		'return iban !== null;'
	],
	[
		'tools/score.mjs',
		"a printed name's key is for one document",
		'if (owner !== null && owner !== doc.id)',
		'if (false)'
	],
	[
		'tools/score.mjs',
		'the same-name report groups document keys by their name',
		'const name = nameOfKey(k);',
		'const name = documentOfKey(k) && k;'
	],
	[
		'tools/score.mjs',
		'the same-name report counts one organisation and two apart',
		'x !== null && y !== null && (x === y) === one',
		'x !== null && y !== null'
	],
	[
		'tools/score.mjs',
		'a format 2 output is refused with what changed',
		'if (output.format === 2)',
		'if (false)'
	],
	[
		'tools/lib/tiers.mjs',
		'a certain expiry cites a printed date, not the zone alone',
		'if (!expiryAt) return false;\n\treturn printedExpiry(expiryAt.text, zone.expiry) === expiry;',
		"return expiry.slice(2).replaceAll('-', '') === zone.expiry;"
	],
	[
		'tools/lib/tiers.mjs',
		"a printed expiry's year ends in the zone's two digits",
		'if (years.length !== 1 || years[0].slice(2) !== yymmdd.slice(0, 2)) return null;',
		'if (years.length !== 1) return null;'
	],
	[
		'tools/lib/tiers.mjs',
		"a printed expiry's day is the zone's",
		'if (given.length !== want.length || given.some((n, i) => n !== want[i])) return null;',
		'if (given.length !== want.length) return null;'
	],
	[
		'tools/reference.mjs',
		'the reference cites the printed expiry',
		'				expiryAt: seen\n',
		'				expiryAt: null\n'
	]
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
