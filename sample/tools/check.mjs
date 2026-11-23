// Proves labels.json against spec.json, the generated files and the
// directory, with no extractor. Exits 1 and lists every failure.
//   node sample/tools/check.mjs            the check
//   node sample/tools/check.mjs --pdfkit   also reads every PDF through macOS PDFKit
import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { build, FILES, loadSpec, pageText, SAMPLE, SUMS, sumsFile } from './generate.mjs';
import { ibanBankCode, KINDS, parseMrz, ribKey } from './lib/ids.mjs';
import { matchKey } from './lib/key.mjs';
import { LISTS, readLabels, spans } from './lib/labels.mjs';
import { planted } from './lib/planted.mjs';

const REASONS = new Set([
	'iban-bank-code',
	'bic-institution',
	'ifsc-prefix',
	'paired-code',
	'inside-iban',
	'same-account',
	'statement-header'
]);
const CODE_REASONS = new Set([...REASONS].filter((r) => r !== 'statement-header'));
const NOT_ORG_WHY = new Set(['inside-other-name', 'word-not-name']);
const NOT_LINK_WHY = new Set([
	'no-tie',
	'code-names-other-bank',
	'held-at-other',
	'invalid-identifier',
	'name-not-this-bank'
]);
const NOT_IDENTIFIER_WHY = new Set([
	'phone-number',
	'invoice-number',
	'reference-number',
	'policy-number',
	'consumer-number',
	'tax-account-number',
	'visa-number'
]);
// Kinds that are held at an organisation: each one gets a link or a not_link
// to every organisation its document names.
const HELD = new Set([
	'account',
	'iban',
	'card',
	'bic',
	'ifsc',
	'gb_sort_code',
	'fr_bank_code',
	'fr_branch_code'
]);
// Where a domestic identifier sits inside its country's IBAN.
const IBAN_PARTS = {
	GB: { gb_sort_code: [8, 14], account: [14, 22] },
	FR: { fr_bank_code: [4, 9], fr_branch_code: [9, 14], account: [14, 25], fr_rib_key: [25, 27] }
};
const PERIOD_FIELD = /^(Statement period|Period|Zeitraum|अवधि|期間): $/u;
const ISSUE_FIELD = /^Date of issue: /iu;
const EXPIRY_FIELD = /^Date of expiry: $/iu;
const FIELDS = {
	doc: ['id', 'note', 'nothing_to_find', ...LISTS, 'statement', 'passport'],
	org: ['id', 'page', 'start', 'end', 'text', 'resolved_by_document', 'directory'],
	organisation: ['name', 'directory', 'names'],
	not_org: ['page', 'start', 'end', 'text', 'entry', 'why'],
	identifier: ['id', 'page', 'start', 'end', 'text', 'kind', 'value', 'valid', 'same_account_as'],
	not_identifier: ['page', 'start', 'end', 'text', 'looks_like', 'why'],
	link: ['identifier', 'org', 'mention', 'reasons', 'via'],
	not_link: ['identifier', 'org', 'why'],
	statement: ['page', 'start', 'end', 'text', 'accounts', 'issuer', 'from', 'to'],
	passport: [
		'holder',
		'number',
		'mrz',
		'issued',
		'expiry',
		'expiry_candidates',
		'expiry_at',
		'current',
		'why'
	]
};

const ibanPart = (iban, kind) => {
	const [from, to] = IBAN_PARTS[iban.slice(0, 2)]?.[kind] ?? [];
	return from === undefined ? null : iban.slice(from, to);
};
const bicMatches = (entry, bic) => entry.codes.bic.some((b) => b.slice(0, 8) === bic.slice(0, 8));

const errors = [];
const fail = (where, message) => errors.push(`${where}: ${message}`);
const fields = (where, object, allowed) => {
	for (const key of Object.keys(object))
		if (!allowed.includes(key)) fail(where, `unknown field ${key}`);
};
const isDate = (s) =>
	typeof s === 'string' &&
	/^\d{4}-\d\d-\d\d$/.test(s) &&
	new Date(s + 'T00:00:00Z').toISOString().startsWith(s);
const holderKey = (h) => `${h.surname}, ${h.given}`;
// One key for a pair of strings, which no two different pairs share.
const pairKey = (a, b) => JSON.stringify([a, b]);
const word = (c) =>
	c !== undefined &&
	/[\p{L}\p{M}\p{N}]/u.test(c) &&
	!/[\p{scx=Han}\p{scx=Hiragana}\p{scx=Katakana}]/u.test(c);
// The words of a name as the match key splits them, before it joins them.
const keyWords = (name) =>
	name
		.normalize('NFKC')
		.toLowerCase()
		.split(/[^\p{L}\p{M}\p{N}]+/u)
		.filter((w) => w && matchKey(w));
// A short form of a name: a prefix of its key, or its initials.
const shortFormOf = (short, name) => {
	const key = matchKey(short);
	const initials = keyWords(name)
		.map((w) => [...w][0])
		.join('');
	return (
		key.length > 0 &&
		key.length < matchKey(name).length &&
		(matchKey(name).startsWith(key) || initials === key)
	);
};

const MONTHS = ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec'];
const HINDI_MONTHS = [
	'जनवरी',
	'फ़रवरी',
	'मार्च',
	'अप्रैल',
	'मई',
	'जून',
	'जुलाई',
	'अगस्त',
	'सितंबर',
	'अक्टूबर',
	'नवंबर',
	'दिसंबर'
];
const DATE_FORMATS = [
	[
		/(\d{1,2}) (jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]* (\d{4})/gi,
		(m) => [m[3], MONTHS.indexOf(m[2].toLowerCase()) + 1, m[1]]
	],
	[/(\d{2})[.-](\d{2})[.-](\d{4})/g, (m) => [m[3], m[2], m[1]]],
	[/(\d{4})年(\d{1,2})月(\d{1,2})日/g, (m) => [m[1], m[2], m[3]]],
	[
		new RegExp(`(\\d{1,2}) (${HINDI_MONTHS.join('|')}) (\\d{4})`, 'g'),
		(m) => [m[3], HINDI_MONTHS.indexOf(m[2]) + 1, m[1]]
	]
];
function datesIn(text) {
	const found = [];
	for (const [pattern, parts] of DATE_FORMATS) {
		for (const m of text.matchAll(pattern)) {
			const [y, mo, d] = parts(m);
			found.push([m.index, `${y}-${String(mo).padStart(2, '0')}-${String(d).padStart(2, '0')}`]);
		}
	}
	return found.sort((a, b) => a[0] - b[0]).map(([, date]) => date);
}

// Files: the generator's output, byte for byte, and SHA256SUMS.
const spec = loadSpec();
const built = build(spec);
const onDisk = new Set(readdirSync(FILES));
for (const { doc, bytes } of built) {
	if (!onDisk.has(doc.file))
		fail(`files/${doc.file}`, 'missing; run node sample/tools/generate.mjs');
	else if (!readFileSync(new URL(doc.file, FILES)).equals(bytes))
		fail(`files/${doc.file}`, 'differs from what the generator writes');
	onDisk.delete(doc.file);
}
onDisk.delete(SUMS);
for (const name of onDisk) fail(`files/${name}`, 'is not in spec.json');
let sums = '';
try {
	sums = readFileSync(new URL(SUMS, FILES), 'utf8');
} catch {
	fail(`files/${SUMS}`, 'missing');
}
if (sums && sums !== sumsFile(built)) fail(`files/${SUMS}`, 'does not match the generated files');

// Directory: match keys, clashes, version.
const directory = JSON.parse(readFileSync(new URL('directory/banks.json', SAMPLE), 'utf8'));
const entries = new Map();
const holders = new Map();
for (const entry of directory.entries) {
	if (entries.has(entry.id)) fail(`directory ${entry.id}`, 'listed twice');
	entries.set(entry.id, entry);
	for (const name of entry.names) {
		if (name.key !== matchKey(name.text))
			fail(
				`directory ${entry.id}`,
				`key of ${JSON.stringify(name.text)} is ${JSON.stringify(name.key)}, the rule gives ${JSON.stringify(matchKey(name.text))}`
			);
		holders.set(name.key, new Set([...(holders.get(name.key) ?? []), entry.id]));
	}
}
const clashKeys = new Set(directory.clashes.map((c) => c.key));
for (const [key, ids] of holders) {
	if (ids.size > 1 !== clashKeys.has(key))
		fail('directory clashes', `key ${JSON.stringify(key)} is held by ${[...ids].join(', ')}`);
}

const bicHolders = new Map();
for (const entry of directory.entries) {
	for (const bic of entry.codes.bic) {
		const institution = bic.slice(0, 6);
		bicHolders.set(institution, new Set([...(bicHolders.get(institution) ?? []), entry]));
	}
}
for (const [institution, holding] of bicHolders) {
	const kinds = new Set([...holding].map((entry) => entry.kind));
	if (kinds.size > 1)
		fail('directory codes', `BIC ${institution} is held by a bank and by an entry that is not one`);
}

const nameIndex = new Map();
const nameHolders = new Map();
for (const entry of directory.entries) {
	for (const n of entry.names) {
		const name = n.text.toLowerCase();
		if ([...name].length < 3 || /^\d+$/.test(name)) continue;
		if (!nameHolders.has(name)) {
			const chars = [...name];
			nameIndex.set(chars[0], [...(nameIndex.get(chars[0]) ?? []), chars]);
			nameHolders.set(name, new Set());
		}
		nameHolders.get(name).add(entry.id);
	}
}
const NUMBER_SHAPES = [
	/\d(?: ?\d){7,}/g,
	/\b[A-Z]{4}0[A-Z0-9]{6}\b/g,
	/\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]{4}){3,7}(?: ?[A-Z0-9]{1,3})?\b/g,
	/\b[A-Z]{5}\d{4}[A-Z]\b/g,
	/[A-Z0-9<]{30,}/g,
	/\b\d\d-\d\d-\d\d\b/g
];

// Labels.
const labels = readLabels(new URL('labels.json', SAMPLE));
if (labels.version !== 2) fail('labels.json', `version ${labels.version}, this check reads 2`);
if (labels.directory_version !== directory.version)
	fail(
		'labels.json',
		`directory_version ${labels.directory_version}, directory is version ${directory.version}`
	);
if (!isDate(labels.reference_date)) fail('labels.json', 'reference_date is not a date');

// Organisations: one identity each, whether or not a directory holds it.
const organisations = new Map(Object.entries(labels.organisations ?? {}));
const byEntry = new Map();
const nameOwner = new Map();
for (const [id, o] of organisations) {
	const at = `organisation ${id}`;
	fields(at, o, FIELDS.organisation);
	if (!/^[a-z0-9-]+$/.test(id)) fail(at, 'id must be lower case, digits and hyphens');
	if (typeof o.name !== 'string' || !o.name) fail(at, 'name is missing');
	if (o.directory !== null) {
		if (!entries.has(o.directory)) fail(at, `directory ${o.directory} is not in the directory`);
		if (byEntry.has(o.directory))
			fail(at, `directory ${o.directory} is also ${byEntry.get(o.directory)}`);
		byEntry.set(o.directory, id);
	}
	if (!Array.isArray(o.names) || !o.names.length) fail(at, 'names must be a list');
	for (const name of o.names ?? []) {
		const key = matchKey(name);
		if (nameOwner.has(key) && nameOwner.get(key) !== id)
			fail(at, `name ${JSON.stringify(name)} has the key of a name of ${nameOwner.get(key)}`);
		nameOwner.set(key, id);
	}
}
const usedOrganisations = new Set();

const specDocs = new Map(spec.documents.map((doc) => [doc.id, doc]));
const labelled = new Set(labels.documents.map((doc) => doc.id));
for (const id of specDocs.keys())
	if (!labelled.has(id)) fail(`labels ${id}`, 'document in spec.json has no labels');

const statements = [];
const passports = [];
const counts = { spans: 0, links: 0, not_links: 0, mentions: 0, planted: 0 };

for (const doc of labels.documents) {
	const where = `labels ${doc.id}`;
	const source = specDocs.get(doc.id);
	if (!source) {
		fail(where, 'not in spec.json');
		continue;
	}
	fields(where, doc, FIELDS.doc);
	if (doc.nothing_to_find === true) {
		fields(where, doc, ['id', 'note', 'nothing_to_find']);
	} else {
		for (const list of LISTS) if (!Array.isArray(doc[list])) fail(where, `${list} must be a list`);
		if (LISTS.some((list) => !Array.isArray(doc[list]))) continue;
	}
	const pages = source.pages.map((page) => [...pageText(page)]);
	const proved = planted(source, doc, pages);
	counts.planted += proved.rows.length;
	for (const problem of proved.problems) fail(where, problem);

	for (const span of spans(doc)) {
		const at = `${where} ${JSON.stringify(span.text)}`;
		const page = pages[span.page - 1];
		if (!page) fail(at, `page ${span.page} does not exist`);
		else if (!(
			Number.isInteger(span.start) &&
			Number.isInteger(span.end) &&
			span.start >= 0 &&
			span.start < span.end &&
			span.end <= page.length
		))
			fail(at, `span ${span.start}..${span.end} is out of the page`);
		else if (page.slice(span.start, span.end).join('') !== span.text)
			fail(
				at,
				`span ${span.start}..${span.end} reads ${JSON.stringify(page.slice(span.start, span.end).join(''))}`
			);
		counts.spans++;
	}
	if (doc.nothing_to_find === true) {
		lint(doc, pages, where);
		continue;
	}
	// A label holds a whole token: it does not start or end inside a word.
	for (const span of [...doc.orgs, ...doc.identifiers, ...doc.not_identifiers]) {
		const page = pages[span.page - 1];
		if (!page) continue;
		const chars = [...span.text];
		if (
			(word(chars[0]) && word(page[span.start - 1])) ||
			(word(chars.at(-1)) && word(page[span.end]))
		)
			fail(`${where} ${JSON.stringify(span.text)}`, 'covers part of a token, not the whole token');
	}

	const ids = new Map();
	for (const i of doc.identifiers) {
		const at = `${where} identifier ${i.id}`;
		fields(at, i, FIELDS.identifier);
		if (ids.has(i.id)) fail(at, 'id used twice');
		ids.set(i.id, i);
		const kind = KINDS[i.kind];
		if (!kind) {
			fail(at, `kind ${i.kind} is unknown`);
			continue;
		}
		if (kind.normalize(i.text) !== i.value)
			fail(
				at,
				`value ${i.value} is not ${JSON.stringify(i.text)} normalized (${kind.normalize(i.text)})`
			);
		const valid = kind.valid ? kind.valid(i.value) : null;
		if (i.valid !== valid) fail(at, `marked valid ${i.valid}, its check gives ${valid}`);
	}
	const lineOf = (span) =>
		pages[span.page - 1].slice(0, span.start).filter((c) => c === '\n').length;
	const sameLine = (a, b) => a.page === b.page && lineOf(a) === lineOf(b);
	const ibans = doc.identifiers.filter((v) => v.kind === 'iban' && v.valid);
	for (const i of doc.identifiers) {
		const at = `${where} identifier ${i.id}`;
		if (i.kind === 'account') {
			const holding = ibans.filter((v) => ibanPart(v.value, 'account') === i.value);
			if (i.same_account_as === undefined && holding.length)
				fail(
					at,
					`is the account inside ${holding[0].value}, so same_account_as is ${holding[0].id}`
				);
			if (i.same_account_as !== undefined && !holding.some((v) => v.id === i.same_account_as))
				fail(at, `same_account_as ${i.same_account_as} is not an IBAN here that holds ${i.value}`);
		} else if (i.same_account_as !== undefined) fail(at, 'only an account has same_account_as');
		if (i.kind === 'fr_rib_key') {
			const onLine = (kind) => doc.identifiers.find((v) => v.kind === kind && sameLine(v, i));
			const [bank, branch, account] = ['fr_bank_code', 'fr_branch_code', 'account'].map(onLine);
			const key =
				bank && branch && account ? ribKey(bank.value, branch.value, account.value) : null;
			if ((key === i.value) !== (i.valid === true))
				fail(at, `marked valid ${i.valid}, the codes and account on its line give the key ${key}`);
		}
	}
	for (const n of doc.not_identifiers) {
		const at = `${where} not_identifier ${JSON.stringify(n.text)}`;
		fields(at, n, FIELDS.not_identifier);
		if (!KINDS[n.looks_like]) fail(at, `looks_like ${n.looks_like} is not a kind`);
		if (!NOT_IDENTIFIER_WHY.has(n.why)) fail(at, `why ${n.why} is not in the list`);
	}

	const orgs = new Map();
	for (const o of doc.orgs) {
		const at = `${where} org ${o.id}`;
		fields(at, o, FIELDS.org);
		if (orgs.has(o.id)) fail(at, 'id used twice');
		orgs.set(o.id, o);
		counts.mentions++;
		const held = [...(holders.get(matchKey(o.text)) ?? [])].sort();
		const lookup = o.directory ?? {};
		if (lookup.lookup === 'exact') {
			if (held.length !== 1 || held[0] !== lookup.id)
				fail(
					at,
					`directory exact ${lookup.id}, but the directory gives ${held.join(', ') || 'nothing'} for ${JSON.stringify(o.text)}`
				);
			if (lookup.candidates) fail(at, 'only a clash has candidates');
		} else if (lookup.lookup === 'clash') {
			const candidates = [...(lookup.candidates ?? [])].sort();
			if (held.length < 2 || JSON.stringify(candidates) !== JSON.stringify(held))
				fail(
					at,
					`directory candidates ${candidates.join(', ')}, the directory gives ${held.join(', ') || 'nothing'}`
				);
			if (lookup.id) fail(at, 'a clash has no id');
		} else if (lookup.lookup === 'none') {
			if (held.length) fail(at, `directory none, but the directory gives ${held.join(', ')}`);
		} else fail(at, `directory lookup ${lookup.lookup} is not exact, clash or none`);
	}
	const resolvedOrg = (o) => o?.resolved_by_document?.org ?? null;
	const entryOf = (o) => organisations.get(resolvedOrg(o))?.directory ?? null;
	const resolved = new Set(doc.orgs.map(resolvedOrg).filter(Boolean));
	const codeOrg = (identifier) => doc.links.find((l) => l.identifier === identifier.id)?.org;
	const prefix = (v) => v.value.slice(0, 4).toLowerCase();

	// Why evidence e does not settle mention o as org, or null when it does.
	const evidenceProblem = (o, org, e) => {
		if (e.by === 'name')
			return nameOwner.get(matchKey(o.text)) === org
				? null
				: `${JSON.stringify(o.text)} is not a name of ${org}`;
		if (e.by === 'mention') {
			const other = orgs.get(e.mention);
			if (!other || other === o || resolvedOrg(other) !== org)
				return `evidence mention ${e.mention} is not another mention of ${org}`;
			if (other.resolved_by_document.evidence?.some((x) => x.by === 'mention'))
				return `evidence mention ${e.mention} is itself settled by another mention`;
			if (!shortFormOf(o.text, other.text))
				return `${JSON.stringify(o.text)} is not a short form of ${JSON.stringify(other.text)}`;
			const rivals = doc.orgs.filter(
				(x) => resolvedOrg(x) && resolvedOrg(x) !== org && shortFormOf(o.text, x.text)
			);
			return rivals.length
				? `${JSON.stringify(o.text)} is also a short form of ${JSON.stringify(rivals[0].text)}`
				: null;
		}
		if (e.by === 'code') {
			const code = ids.get(e.identifier);
			if (!code || !['ifsc', 'bic'].includes(code.kind))
				return `evidence identifier ${e.identifier} is not an IFSC or BIC here`;
			if (prefix(code) !== matchKey(o.text) || codeOrg(code) !== org)
				return `${code.value} does not spell ${JSON.stringify(o.text)} and tie it to ${org}`;
			return doc.identifiers.some(
				(v) =>
					['ifsc', 'bic'].includes(v.kind) && prefix(v) === prefix(code) && codeOrg(v) !== org
			)
				? `another code spelling ${JSON.stringify(o.text)} is tied elsewhere`
				: null;
		}
		return `evidence by ${e.by} is not name, mention or code`;
	};
	// Every organisation and evidence the document gives for mention o.
	const settles = (o) =>
		[
			[nameOwner.get(matchKey(o.text)), { by: 'name' }],
			...doc.orgs.map((x) => [resolvedOrg(x), { by: 'mention', mention: x.id }]),
			...doc.identifiers.map((v) => [codeOrg(v), { by: 'code', identifier: v.id }])
		].filter(([org, e]) => org && !evidenceProblem(o, org, e));

	for (const o of doc.orgs) {
		const at = `${where} org ${o.id}`;
		const r = o.resolved_by_document;
		if (r === null) {
			if (o.directory?.lookup === 'exact' && !byEntry.has(o.directory.id))
				fail(at, `the directory settles it as ${o.directory.id}, which no organisation carries`);
			const settled = settles(o);
			if (settled.length)
				fail(
					at,
					`the document settles it as ${[...new Set(settled.map(([org, e]) => `${org} by ${e.by}`))].join(', ')}, so resolved_by_document is not null`
				);
			continue;
		}
		if (!r || !organisations.has(r.org)) {
			fail(at, `resolved_by_document ${JSON.stringify(r)} names no organisation`);
			continue;
		}
		usedOrganisations.add(r.org);
		if (o.directory?.lookup === 'exact' && o.directory.id !== organisations.get(r.org).directory)
			fail(at, `the document settles it as ${r.org}, the directory as ${o.directory.id}`);
		if (!Array.isArray(r.evidence) || !r.evidence.length) fail(at, 'evidence must be a list');
		for (const e of r.evidence ?? []) {
			const problem = evidenceProblem(o, r.org, e);
			if (problem) fail(at, problem);
		}
	}

	for (const n of doc.not_orgs) {
		const at = `${where} not_org ${JSON.stringify(n.text)}`;
		fields(at, n, FIELDS.not_org);
		if (!entries.has(n.entry)) fail(at, `entry ${n.entry} is not in the directory`);
		const positive = doc.orgs.find(
			(o) =>
				o.page === n.page &&
				o.start < n.end &&
				n.start < o.end &&
				(entryOf(o) === n.entry ||
					o.directory?.id === n.entry ||
					(o.directory?.candidates ?? []).includes(n.entry))
		);
		if (positive) fail(at, `overlaps mention ${positive.id}, which is a mention of ${n.entry}`);
		const inside = doc.orgs.some(
			(o) =>
				o.page === n.page &&
				o.start <= n.start &&
				n.end <= o.end &&
				o.end - o.start > n.end - n.start &&
				entryOf(o) !== n.entry
		);
		if (!NOT_ORG_WHY.has(n.why)) fail(at, `why ${n.why} is not in the list`);
		else if ((n.why === 'inside-other-name') !== inside)
			fail(
				at,
				inside
					? 'lies inside a longer mention, so why is inside-other-name'
					: 'inside-other-name, but no longer mention of another organisation holds it'
			);
	}

	// Every organisation the document names or ties an identifier to: settled
	// mentions, links, the candidates of a clash, and an organisation whose
	// name sits inside another's.
	const named = new Set([...resolved, ...doc.links.map((l) => l.org)]);
	const nameEntries = [
		...doc.orgs.flatMap((o) => o.directory?.candidates ?? []),
		...doc.not_orgs.filter((n) => n.why === 'inside-other-name').map((n) => n.entry)
	];
	const heldHere = doc.identifiers.some((i) => HELD.has(i.kind));
	for (const entry of nameEntries) {
		if (byEntry.has(entry)) named.add(byEntry.get(entry));
		else if (heldHere) fail(where, `names ${entry}, which no organisation carries`);
	}

	const statementValues = new Set((doc.statement?.accounts ?? []).map((id) => ids.get(id)?.value));
	const issuer = orgs.get(doc.statement?.issuer);
	const codeLinked = (v, org) =>
		doc.links.some(
			(l) =>
				l.identifier === v.id &&
				l.org === org &&
				(l.reasons.includes('ifsc-prefix') || l.reasons.includes('bic-institution'))
		);
	const linkedIbanHolding = (id, org) =>
		ibans.some(
			(v) =>
				ibanPart(v.value, id.kind) === id.value &&
				doc.links.some((l) => l.identifier === v.id && l.org === org)
		);
	const lineHas = (span, kinds) =>
		doc.identifiers.some((v) => v !== span && kinds.includes(v.kind) && sameLine(v, span));
	// Why a reason does not hold for identifier id at org, or null when it holds.
	const reasonProblem = (reason, id, org, via) => {
		const entry = entries.get(organisations.get(org)?.directory);
		if (['iban-bank-code', 'bic-institution', 'ifsc-prefix'].includes(reason) && !entry)
			return `${org} is not in the directory`;
		if (reason === 'iban-bank-code') {
			if (id.kind !== 'iban') return 'not an IBAN';
			const code = ibanBankCode(id.value);
			return entry.codes.iban_bank_code.some(
				(c) => c.country === code.country && c.code === code.code
			)
				? null
				: `${code.country} ${code.code} is not a bank code of ${org}`;
		}
		if (reason === 'bic-institution') {
			if (id.kind !== 'bic') return 'not a BIC';
			return bicMatches(entry, id.value) ? null : `${id.value} is not a BIC of ${org}`;
		}
		if (reason === 'ifsc-prefix') {
			if (id.kind !== 'ifsc') return 'not an IFSC';
			return entry.codes.ifsc_prefix.includes(id.value.slice(0, 4))
				? null
				: `${id.value.slice(0, 4)} is not an IFSC prefix of ${org}`;
		}
		if (reason === 'paired-code') {
			if (!['account', 'iban'].includes(id.kind)) return `a ${id.kind} is not paired`;
			if (!via || !['ifsc', 'bic'].includes(via.kind))
				return 'via is not an IFSC or BIC in this document';
			if (!codeLinked(via, org)) return `${via.value} is not tied to ${org} by its own code`;
			if (sameLine(via, id)) return null;
			if (via.page !== id.page || Math.abs(lineOf(via) - lineOf(id)) > 1)
				return `${via.value} is not on the same or the next line`;
			if (lineHas(id, ['ifsc', 'bic'])) return `${id.value} has a code of its own on its line`;
			if (lineHas(via, ['account', 'iban']))
				return `${via.value} belongs to the account on its own line`;
			return null;
		}
		if (reason === 'inside-iban') {
			if (!['gb_sort_code', 'fr_bank_code', 'fr_branch_code'].includes(id.kind))
				return `a ${id.kind} is not part of an IBAN`;
			return linkedIbanHolding(id, org) ? null : `no IBAN tied to ${org} holds ${id.value}`;
		}
		if (reason === 'same-account') {
			const iban = ids.get(id.same_account_as);
			return iban && doc.links.some((l) => l.identifier === iban.id && l.org === org)
				? null
				: `${id.value} is not the same account as an IBAN tied to ${org}`;
		}
		if (!statementValues.has(id.value)) return "the identifier is not the statement's account";
		if (!issuer || resolvedOrg(issuer) !== org) return `the statement's issuer is not ${org}`;
		return null;
	};
	const holdingReasons = (id, org) =>
		id.valid === false
			? []
			: [...REASONS].filter((reason) =>
					reason === 'paired-code'
						? doc.identifiers.some((v) => !reasonProblem(reason, id, org, v))
						: !reasonProblem(reason, id, org)
				);

	const linked = new Map();
	for (const link of doc.links) {
		const at = `${where} link ${link.identifier}->${link.org}`;
		fields(at, link, FIELDS.link);
		counts.links++;
		const id = ids.get(link.identifier);
		if (!id) fail(at, 'identifier is not in this document');
		if (!organisations.has(link.org)) fail(at, 'org is not an organisation');
		if (link.mention !== null && resolvedOrg(orgs.get(link.mention)) !== link.org)
			fail(at, `mention ${link.mention} is not a mention of ${link.org} in this document`);
		if (link.mention === null && resolved.has(link.org))
			fail(at, `the document mentions ${link.org}, so mention is not null`);
		if (!id || !organisations.has(link.org)) continue;
		usedOrganisations.add(link.org);
		if (id.valid === false) fail(at, 'links an identifier that fails its check');
		const key = pairKey(id.value, link.org);
		if (linked.has(key)) fail(at, 'linked twice');
		linked.set(key, link);
		if (
			!Array.isArray(link.reasons) ||
			!link.reasons.length ||
			new Set(link.reasons).size !== link.reasons.length
		)
			fail(at, 'reasons must be a list without repeats');
		if (link.reasons.includes('paired-code') !== (link.via !== undefined))
			fail(at, 'via goes with paired-code, and only with it');
		for (const reason of link.reasons)
			if (!REASONS.has(reason)) fail(at, `reason ${reason} is not in the list`);
		const holding = holdingReasons(id, link.org);
		for (const reason of REASONS) {
			const listed = link.reasons.includes(reason);
			let problem = holding.includes(reason)
				? null
				: (reasonProblem(reason, id, link.org) ?? 'no code beside it');
			if (reason === 'paired-code' && listed)
				problem = reasonProblem(reason, id, link.org, ids.get(link.via));
			if (listed && problem) fail(at, `${reason}: ${problem}`);
			if (!listed && !problem) fail(at, `${reason} holds but is not listed`);
		}
	}
	for (const i of doc.identifiers) {
		if (!['iban', 'bic', 'ifsc'].includes(i.kind) || i.valid !== true) continue;
		for (const entry of directory.entries) {
			const code = ibanBankCode(i.value);
			const names =
				(i.kind === 'iban' &&
					entry.codes.iban_bank_code.some(
						(c) => c.country === code.country && c.code === code.code
					)) ||
				(i.kind === 'bic' && bicMatches(entry, i.value)) ||
				(i.kind === 'ifsc' && entry.codes.ifsc_prefix.includes(i.value.slice(0, 4)));
			if (names && !linked.has(pairKey(i.value, byEntry.get(entry.id))))
				fail(
					`${where} identifier ${i.id}`,
					`its code names ${entry.id}, but there is no link to it`
				);
		}
	}
	const linkedTo = new Map();
	for (const key of linked.keys()) {
		const [value, org] = JSON.parse(key);
		if (linkedTo.has(value) && linkedTo.get(value) !== org)
			fail(where, `${value} is linked to both ${linkedTo.get(value)} and ${org}`);
		linkedTo.set(value, org);
	}

	const excluded = new Set();
	for (const n of doc.not_links) {
		const at = `${where} not_link ${n.identifier}->${n.org}`;
		fields(at, n, FIELDS.not_link);
		counts.not_links++;
		const id = ids.get(n.identifier);
		if (!id) fail(at, 'identifier is not in this document');
		if (!organisations.has(n.org)) fail(at, 'org is not an organisation');
		if (!id || !organisations.has(n.org)) continue;
		usedOrganisations.add(n.org);
		if (!named.has(n.org) && n.why !== 'invalid-identifier')
			fail(at, `the document does not name ${n.org}`);
		if (excluded.has(pairKey(n.identifier, n.org))) fail(at, 'listed twice');
		excluded.add(pairKey(n.identifier, n.org));
		if (linked.has(pairKey(id.value, n.org))) fail(at, 'the same pair is also a link');
		const holds = holdingReasons(id, n.org);
		if (holds.length) fail(at, `${holds.join(', ')} holds, so the pair is a link`);
		const other = linkedTo.get(id.value);
		const otherByCode = [...linked.values()].some(
			(l) => ids.get(l.identifier)?.value === id.value && l.reasons.some((r) => CODE_REASONS.has(r))
		);
		if (!NOT_LINK_WHY.has(n.why)) fail(at, `why ${n.why} is not in the list`);
		else if (n.why === 'invalid-identifier' && id.valid !== false)
			fail(at, 'invalid-identifier, but the identifier passes its check');
		else if (n.why === 'code-names-other-bank' && !(other && other !== n.org && otherByCode))
			fail(at, 'code-names-other-bank, but no code ties the identifier to another bank');
		else if (n.why === 'held-at-other' && !(other && other !== n.org && !otherByCode))
			fail(
				at,
				otherByCode
					? 'a code ties the identifier to another bank, so why is code-names-other-bank'
					: 'held-at-other, but the identifier is not held at another organisation'
			);
		else if (n.why === 'no-tie' && other)
			fail(at, `no-tie, but the identifier is linked to ${other}`);
		else if (n.why === 'name-not-this-bank' && resolved.has(n.org))
			fail(at, 'name-not-this-bank, but the document names this bank');
	}
	// Every held identifier has a link or a not_link to every named organisation.
	for (const i of doc.identifiers) {
		if (!HELD.has(i.kind)) continue;
		for (const org of named) {
			if (!linked.has(pairKey(i.value, org)) && !excluded.has(pairKey(i.id, org)))
				fail(`${where} identifier ${i.id}`, `has neither a link nor a not_link to ${org}`);
		}
	}

	if (doc.statement) {
		const s = doc.statement;
		const at = `${where} statement`;
		fields(at, s, FIELDS.statement);
		const accounts = (s.accounts ?? []).map((id) => ids.get(id));
		if (!accounts.length || accounts.some((a) => !a))
			fail(at, 'accounts must name identifiers of this document');
		const one = accounts.every(
			(a) =>
				a &&
				accounts.every(
					(b) => a.value === b?.value || a.same_account_as === b?.id || b?.same_account_as === a.id
				)
		);
		if (!one) fail(at, 'accounts are not all spellings of one account');
		if (!issuer || !resolvedOrg(issuer)) fail(at, 'issuer must name a settled mention');
		else if (
			doc.orgs.some(
				(o) => o.page < issuer.page || (o.page === issuer.page && o.start < issuer.start)
			)
		)
			fail(at, `issuer ${s.issuer} is not the first organisation the document names`);
		if (!isDate(s.from) || !isDate(s.to) || s.from > s.to)
			fail(at, `period ${s.from}..${s.to} is not a pair of dates in order`);
		const read = datesIn(s.text);
		if (read[0] !== s.from || read[1] !== s.to)
			fail(at, `the text gives ${read.join(' and ') || 'no dates'}, not ${s.from} and ${s.to}`);
		const page = pages[s.page - 1] ?? [];
		const lineStart = page.lastIndexOf('\n', s.start - 1) + 1;
		const lineEnd = page.indexOf('\n', s.end);
		if (
			!PERIOD_FIELD.test(page.slice(lineStart, s.start).join('')) ||
			(lineEnd === -1 ? page.length : lineEnd) !== s.end
		)
			fail(at, `${JSON.stringify(s.text)} is not the statement's own period field`);
		if (one && accounts.length) {
			const iban = accounts.find((a) => a.kind === 'iban');
			statements.push({
				doc: doc.id,
				account: (iban ?? accounts[0]).value,
				spellings: new Set(accounts.map((a) => a.value)),
				from: s.from,
				to: s.to,
				created: source.created
			});
		}
	}

	if (doc.passport) {
		const p = doc.passport;
		const at = `${where} passport`;
		fields(at, p, FIELDS.passport);
		const number = ids.get(p.number);
		const mrzId = ids.get(p.mrz);
		if (number?.kind !== 'passport_number')
			fail(at, 'number must name a passport_number identifier');
		if (mrzId?.kind !== 'mrz') fail(at, 'mrz must name an mrz identifier');
		if (!isDate(p.issued)) fail(at, 'issued is not a date');
		const issueFields = pages
			.flatMap((page) => page.join('').split('\n'))
			.filter((line) => ISSUE_FIELD.test(line))
			.map((line) => datesIn(line.replace(ISSUE_FIELD, ''))[0]);
		if (issueFields.length !== 1 || issueFields[0] !== p.issued)
			fail(at, `the date of issue field reads ${issueFields.join(' and ') || 'nothing'}, not ${p.issued}`);
		const expiryPage = pages[p.expiry_at.page - 1] ?? [];
		const expiryLine = expiryPage.lastIndexOf('\n', p.expiry_at.start - 1) + 1;
		if (!EXPIRY_FIELD.test(expiryPage.slice(expiryLine, p.expiry_at.start).join('')))
			fail(at, `expiry_at ${JSON.stringify(p.expiry_at.text)} is not the date of expiry field`);
		const printed = datesIn(p.expiry_at.text);
		const mrz = mrzId && parseMrz(mrzId.value);
		const mrzExpiry =
			mrz && `20${mrz.expiry.slice(0, 2)}-${mrz.expiry.slice(2, 4)}-${mrz.expiry.slice(4, 6)}`;
		if (mrz?.type !== 'passport') fail(at, "the machine-readable zone is not a passport's");
		else if (
			mrz.surname !== p.holder.surname ||
			mrz.given !== p.holder.given ||
			mrz.number !== number?.value
		)
			fail(at, 'holder or number differs from the machine-readable zone');
		if (p.expiry === 'cannot-tell') {
			const candidates = p.expiry_candidates ?? [];
			if (mrzId?.valid !== false)
				fail(at, 'cannot-tell, but the machine-readable zone passes its checks');
			if (candidates.length < 2 || !candidates.every(isDate))
				fail(at, 'cannot-tell needs two or more candidate dates');
			if (!candidates.includes(printed[0]) || !candidates.includes(mrzExpiry))
				fail(at, `candidates must hold the printed ${printed[0]} and the zone's ${mrzExpiry}`);
		} else {
			if (!isDate(p.expiry) || p.expiry_candidates)
				fail(at, 'expiry is a date, or cannot-tell with candidates');
			if (printed[0] !== p.expiry) fail(at, `expiry_at reads ${printed[0]}, not ${p.expiry}`);
			if (mrzId?.valid !== true || mrzExpiry !== p.expiry)
				fail(
					at,
					`the machine-readable zone gives ${mrzExpiry}${mrzId?.valid ? '' : ' and fails its check'}`
				);
		}
		passports.push({
			doc: doc.id,
			holder: holderKey(p.holder),
			issued: p.issued,
			dates: p.expiry === 'cannot-tell' ? (p.expiry_candidates ?? []) : [p.expiry],
			current: p.current
		});
	}
	lint(doc, pages, where);
}
for (const id of organisations.keys())
	if (!usedOrganisations.has(id)) fail(`organisation ${id}`, 'no label uses it');

// Answers, derived from the labels above.
const ref = labels.reference_date;
for (const p of passports) {
	const unexpired = (q) =>
		q.dates.every((d) => d >= ref) ? true : q.dates.every((d) => d < ref) ? false : 'cannot-tell';
	const newer = passports.some(
		(q) => q !== p && q.holder === p.holder && q.issued > p.issued && unexpired(q) !== false
	);
	const current = newer ? false : unexpired(p);
	if (p.current !== current)
		fail(`labels ${p.doc} passport`, `current is ${p.current}, the dates give ${current}`);
}
const answer = (list) => new Set(list.map((a) => JSON.stringify(a)));
const sameSet = (a, b) => a.size === b.size && [...a].every((x) => b.has(x));
const currentWant = answer(
	passports.filter((p) => p.current === true).map((p) => ({ holder: p.holder, document: p.doc }))
);
if (!sameSet(currentWant, answer(labels.answers.current_passport)))
	fail('labels answers', `current_passport should be ${[...currentWant].join(' ')}`);
// Statements that share a spelling are statements of one account, keyed by
// its IBAN when any statement gives one.
const accounts = [];
for (const s of statements) {
	const into = { spellings: new Set(s.spellings), list: [s] };
	for (const a of accounts.filter((a) => [...s.spellings].some((v) => a.spellings.has(v)))) {
		accounts.splice(accounts.indexOf(a), 1);
		a.spellings.forEach((v) => into.spellings.add(v));
		into.list.push(...a.list);
	}
	accounts.push(into);
}
for (const a of accounts)
	a.account = a.list.find((s) => /^[A-Z]{2}\d\d/.test(s.account))?.account ?? a.list[0].account;
const latestWant = answer(
	accounts.map((a) => ({
		account: a.account,
		spellings: [...a.spellings].sort(),
		document: a.list.reduce((x, y) => (y.to > x.to ? y : x)).doc
	}))
);
if (!sameSet(latestWant, answer(labels.answers.latest_statement)))
	fail('labels answers', `latest_statement should be ${[...latestWant].join(' ')}`);
for (const { account, list } of accounts) {
	if (list.length < 2) continue;
	const byPeriod = [...list]
		.sort((a, b) => a.to.localeCompare(b.to))
		.map((s) => s.doc);
	const byFileDate = [...list]
		.sort((a, b) => a.created.localeCompare(b.created))
		.map((s) => s.doc);
	if (JSON.stringify(byPeriod) === JSON.stringify(byFileDate))
		fail(`statements of ${account}`, 'file dates agree with the periods; they must not');
}

// Completeness: every directory name on a page is covered, for each entry
// that holds it, by a mention of that entry or a not_org of it; every
// identifier-shaped number lies whole inside one label.
function lint(doc, pages, where) {
	const overlaps = (s, page, start, end) => s.page === page && s.start < end && start < s.end;
	const entryOf = (o) => organisations.get(o.resolved_by_document?.org)?.directory;
	pages.forEach((chars, index) => {
		const page = index + 1;
		const lower = chars.map((c) => c.toLowerCase());
		for (const [first, names] of nameIndex) {
			for (let i = 0; i < lower.length; i++) {
				if (lower[i] !== first) continue;
				for (const name of names) {
					if (!name.every((c, j) => lower[i + j] === c)) continue;
					const end = i + name.length;
					if ((word(name[0]) && word(chars[i - 1])) || (word(name.at(-1)) && word(chars[end])))
						continue;
					for (const entry of nameHolders.get(name.join(''))) {
						const ofEntry = (o) =>
							entryOf(o) === entry ||
							o.directory?.id === entry ||
							(o.directory?.candidates ?? []).includes(entry);
						if (
							!(doc.orgs ?? []).some((o) => ofEntry(o) && overlaps(o, page, i, end)) &&
							!(doc.not_orgs ?? []).some((n) => n.entry === entry && overlaps(n, page, i, end))
						)
							fail(
								where,
								`page ${page} names ${JSON.stringify(chars.slice(i, end).join(''))} (${entry}) at ${i}, which no org or not_org label of ${entry} covers`
							);
					}
				}
			}
		}
		const text = chars.join('');
		const toPoint = (utf16) => [...text.slice(0, utf16)].length;
		for (const pattern of NUMBER_SHAPES) {
			for (const m of text.matchAll(pattern)) {
				const start = toPoint(m.index);
				const end = start + [...m[0]].length;
				if (
					![...(doc.identifiers ?? []), ...(doc.not_identifiers ?? [])].some(
						(s) => s.page === page && s.start <= start && end <= s.end
					)
				)
					fail(
						where,
						`page ${page} holds ${JSON.stringify(m[0])} at ${start}, which no identifier or not_identifier label holds whole`
					);
			}
		}
	});
}

if (process.argv.includes('--pdfkit')) {
	const pdfs = built.filter(({ doc }) => doc.format === 'pdf');
	const script = fileURLToPath(new URL('pdfkit-text.swift', import.meta.url));
	const read = JSON.parse(
		execFileSync(
			'swift',
			[script, ...pdfs.map(({ doc }) => fileURLToPath(new URL(doc.file, FILES)))],
			{ encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 1 << 26 }
		)
	);
	for (const { doc } of pdfs) {
		doc.pages.forEach((page, i) => {
			const want = page.image ? '' : pageText(page);
			if (read[doc.file]?.[i] !== want)
				fail(
					`files/${doc.file} page ${i + 1}`,
					`PDFKit reads ${JSON.stringify(read[doc.file]?.[i])}, the spec says ${JSON.stringify(want)}`
				);
		});
	}
}

if (errors.length) {
	console.error(errors.join('\n'));
	console.error(`\n${errors.length} failures`);
	process.exit(1);
}
console.log(
	`ok: ${built.length} documents, ${counts.spans} spans, ${counts.mentions} organisation mentions, ${counts.links} links, ${counts.not_links} not_links, ${counts.planted} planted tokens, ${organisations.size} organisations, ${directory.entries.length} directory entries${process.argv.includes('--pdfkit') ? ', PDFKit text matches' : ''}`
);
