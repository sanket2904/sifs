// Builds banks.json from Wikidata and codes.json:
//   node sample/directory/build.mjs
// Wikidata changes daily, so a rebuild differs from the committed file; bump
// VERSION when you commit a rebuild.
import { readFileSync, writeFileSync } from 'node:fs';
import { matchKey } from '../tools/lib/key.mjs';

const VERSION = 1;
const ENDPOINT = 'https://query.wikidata.org/sparql';
const LANGS = ['en', 'mul', 'de', 'fr', 'es', 'hi', 'ja', 'zh', 'ar'];
const BIC_COUNTRIES = ['Q145', 'Q27', 'Q55', 'Q17', 'Q142', 'Q29', 'Q39'];
const IBAN_CODE_IS_BIC_PREFIX = new Set(['GB', 'IE', 'NL']);
const SEEDS = {
	Q631047: 'HDFC Bank',
	Q1446319: 'Housing Development Finance Corporation: carries the alias HDFC',
	Q5629189: 'HDFC Life: not a bank, has a bank name inside',
	Q5629187: 'HDFC ERGO: not a bank, has a bank name inside',
	Q5969591: 'ICICI Lombard: not a bank, has a bank alias inside',
	Q66048: 'Deutsche Bank',
	Q157617: 'Commerzbank',
	Q162222: 'Deutsche Bundesbank: shares words with Deutsche Bank',
	Q1203255: 'Deutsche Kreditbank: shares words with Deutsche Bank',
	Q857063: 'Citibank',
	Q6883178: 'Mitsubishi UFJ Trust and Banking: shares words with MUFG Bank',
	Q806689: 'Bank of Ireland: shares words with Bank of India',
	Q28457664: 'The Bank of Industry: shares the alias BOI with Bank of India',
	Q1113213: 'SBI Holdings: shares the alias SBI with State Bank of India'
};

const here = new URL('.', import.meta.url);
const codes = JSON.parse(readFileSync(new URL('codes.json', here), 'utf8'));

async function sparql(query) {
	const response = await fetch(ENDPOINT, {
		method: 'POST',
		headers: {
			'User-Agent': 'sample-directory-builder/1.0',
			Accept: 'application/sparql-results+json',
			'Content-Type': 'application/x-www-form-urlencoded'
		},
		body: new URLSearchParams({ query })
	});
	if (!response.ok)
		throw new Error(
			`Wikidata answered ${response.status}: ${(await response.text()).slice(0, 300)}`
		);
	return (await response.json()).results.bindings;
}

const qid = (uri) => uri.slice(uri.lastIndexOf('/') + 1);
const values = (ids) => ids.map((id) => `wd:${id}`).join(' ');

export const SELECTION = [
	`SELECT DISTINCT ?item WHERE { ?item wdt:P31/wdt:P279* wd:Q22687 ; wdt:P17 wd:Q668 .
    FILTER NOT EXISTS { ?item wdt:P31/wdt:P279* wd:Q1880737 } }`,
	`SELECT DISTINCT ?item WHERE { VALUES ?country { ${values(BIC_COUNTRIES)} } ?item wdt:P2627 [] ; wdt:P17 ?country . }`
];

const namesQuery = (ids) => `SELECT ?item ?name ?type WHERE {
  VALUES ?item { ${values(ids)} }
  { ?item rdfs:label ?name BIND("label" AS ?type) } UNION { ?item skos:altLabel ?name BIND("alias" AS ?type) }
  FILTER(LANG(?name) IN (${LANGS.map((l) => `"${l}"`).join(', ')}))
}`;

const factsQuery = (ids) => `SELECT ?item ?bic ?iso ?dissolved ?isBank WHERE {
  VALUES ?item { ${values(ids)} }
  BIND(EXISTS { ?item wdt:P31/wdt:P279* wd:Q22687 } AS ?isBank)
  OPTIONAL { ?item wdt:P2627 ?bic }
  OPTIONAL { ?item wdt:P17/wdt:P297 ?iso }
  OPTIONAL { ?item wdt:P576 ?dissolved }
}`;

const selected = new Set([
	...Object.keys(SEEDS),
	...Object.keys(codes.ifsc_prefix.codes),
	...Object.keys(codes.iban_bank_code.codes)
]);
for (const query of SELECTION)
	for (const row of await sparql(query)) selected.add(qid(row.item.value));
const ids = [...selected];
const entries = new Map(
	ids.map((id) => [
		id,
		{
			id,
			labels: [],
			aliases: [],
			bic: new Set(),
			country: new Set(),
			dissolved: null,
			isBank: false
		}
	])
);

for (let i = 0; i < ids.length; i += 150) {
	const batch = ids.slice(i, i + 150);
	for (const row of await sparql(namesQuery(batch))) {
		const entry = entries.get(qid(row.item.value));
		(row.type.value === 'label' ? entry.labels : entry.aliases).push({
			text: row.name.value,
			lang: row.name['xml:lang']
		});
	}
	for (const row of await sparql(factsQuery(batch))) {
		const entry = entries.get(qid(row.item.value));
		entry.isBank = row.isBank.value === 'true';
		if (row.bic) entry.bic.add(row.bic.value.trim().toUpperCase());
		if (row.iso) entry.country.add(row.iso.value);
		if (row.dissolved) entry.dissolved = row.dissolved.value.slice(0, 10);
	}
}

const curated = codes.aliases.names;
const langOrder = (a, b) => LANGS.indexOf(a) - LANGS.indexOf(b);

function names(entry) {
	const byText = new Map();
	const add = (text, lang, type, source) => {
		const name = byText.get(text) ?? { text, langs: [], type, source, key: matchKey(text) };
		if (!name.langs.includes(lang)) name.langs.push(lang);
		if (type === 'label') name.type = 'label';
		byText.set(text, name);
	};
	for (const { text, lang } of entry.labels) add(text, lang, 'label', 'wikidata');
	for (const { text, lang } of entry.aliases) add(text, lang, 'alias', 'wikidata');
	for (const { text, lang } of curated[entry.id] ?? []) add(text, lang, 'alias', 'curated');
	return [...byText.values()]
		.filter((name) => name.key)
		.map((name) => ({ ...name, langs: name.langs.sort(langOrder) }))
		.sort(
			(a, b) =>
				(a.type === b.type ? 0 : a.type === 'label' ? -1 : 1) ||
				langOrder(a.langs[0], b.langs[0]) ||
				a.text.localeCompare(b.text, 'en')
		);
}

function canonical(entry) {
	for (const lang of LANGS) {
		const label = entry.labels.find((l) => l.lang === lang);
		if (label) return label.text;
	}
	return entry.id;
}

// Wikidata puts some banks' BICs on their customers too (an energy company
// carrying its bank's BIC). A BIC's bank and country (its first six
// characters) held by entries classed as banks stays only on those entries.
const institutions = Map.groupBy(
	[...entries.values()].flatMap((entry) => [...entry.bic].map((bic) => ({ entry, bic }))),
	({ bic }) => bic.slice(0, 6)
);
const droppedCodes = [];
for (const holders of institutions.values()) {
	if (!holders.some(({ entry }) => entry.isBank)) continue;
	for (const { entry, bic } of holders) {
		if (entry.isBank) continue;
		entry.bic.delete(bic);
		droppedCodes.push({
			id: entry.id,
			bic,
			kept_on: [...new Set(holders.filter((h) => h.entry.isBank).map((h) => h.entry.id))].sort()
		});
	}
}
droppedCodes.sort(
	(a, b) => Number(a.id.slice(1)) - Number(b.id.slice(1)) || a.bic.localeCompare(b.bic)
);

const out = [...entries.values()]
	.map((entry) => {
		const bic = [...entry.bic].sort();
		const ibanFromBic = bic
			.filter((b) => IBAN_CODE_IS_BIC_PREFIX.has(b.slice(4, 6)))
			.map((b) => ({ country: b.slice(4, 6), code: b.slice(0, 4) }));
		const ibanCodes = [...ibanFromBic, ...(codes.iban_bank_code.codes[entry.id] ?? [])].filter(
			(c, i, all) => all.findIndex((d) => d.country === c.country && d.code === c.code) === i
		);
		return {
			id: entry.id,
			name: canonical(entry),
			kind: entry.isBank ? 'bank' : 'other',
			country: [...entry.country].sort(),
			dissolved: entry.dissolved,
			names: names(entry),
			codes: {
				bic,
				ifsc_prefix: codes.ifsc_prefix.codes[entry.id] ?? [],
				iban_bank_code: ibanCodes
			}
		};
	})
	.filter((entry) => entry.names.length > 0)
	.sort((a, b) => Number(a.id.slice(1)) - Number(b.id.slice(1)));

for (const id of [
	...Object.keys(SEEDS),
	...Object.keys(codes.ifsc_prefix.codes),
	...Object.keys(codes.iban_bank_code.codes)
]) {
	if (!out.some((entry) => entry.id === id))
		throw new Error(`${id} is named in build.mjs or codes.json but is not in the directory`);
}

const holders = new Map();
for (const entry of out)
	for (const name of entry.names)
		holders.set(name.key, new Set([...(holders.get(name.key) ?? []), entry.id]));
const clashes = [...holders]
	.filter(([, holding]) => holding.size > 1)
	.map(([key, holding]) => ({
		key,
		ids: [...holding].sort((a, b) => Number(a.slice(1)) - Number(b.slice(1)))
	}))
	.sort((a, b) => a.key.localeCompare(b.key, 'en'));
for (const entry of out) {
	for (const name of entry.names) {
		const other = [...holders.get(name.key)].filter((id) => id !== entry.id);
		if (other.length) name.clash_with = other.sort();
	}
}

const doc = {
	version: VERSION,
	built: new Date().toISOString().slice(0, 10),
	source:
		'Wikidata (query.wikidata.org), CC0 1.0. Selection: the seeds and the two SELECTION queries in build.mjs.',
	licence:
		'Wikidata data is CC0 1.0. IFSC prefixes and German bank codes come from codes.json, whose sources and licences it states.',
	code_sources: {
		ifsc_prefix: codes.ifsc_prefix.source,
		iban_bank_code: codes.iban_bank_code.source,
		curated_names: codes.aliases.source
	},
	match_key:
		'NFKC, lower case, the text 株式会社 removed, split on every character that is not a letter, mark or digit, the words the, ltd, limited, plc, inc, ag, sa and nv dropped, the rest joined. Same key, same name.',
	clashes,
	code_rule:
		'A BIC whose first six characters (bank and country) are also held by an entry Wikidata classes as a bank is kept only on the bank entries. dropped_codes lists each BIC removed this way.',
	dropped_codes: droppedCodes
};
const lines = JSON.stringify(doc, null, 2).split('\n');
lines.pop();
lines[lines.length - 1] += ',';
lines.push(
	'  "entries": [',
	out.map((entry) => '    ' + JSON.stringify(entry)).join(',\n'),
	'  ]',
	'}'
);
writeFileSync(new URL('banks.json', here), lines.join('\n') + '\n');
console.log(
	`${out.length} entries, ${clashes.length} clashes, ${droppedCodes.length} BICs dropped`
);
