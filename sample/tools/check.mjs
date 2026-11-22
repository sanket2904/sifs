// Proves labels.json against spec.json, the generated files and the
// directory, with no extractor. Exits 1 and lists every failure.
//   node sample/tools/check.mjs            the check
//   node sample/tools/check.mjs --pdfkit   also reads every PDF through macOS PDFKit
import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { build, FILES, loadSpec, pageText, SAMPLE, SUMS, sumsFile } from './generate.mjs';
import { KINDS, parseMrz } from './lib/ids.mjs';
import { matchKey } from './lib/key.mjs';
import { LISTS, readLabels, spans } from './lib/labels.mjs';

const REASONS = new Set([
	'iban-bank-code',
	'bic-institution',
	'ifsc-prefix',
	'paired-code',
	'statement-header'
]);
const CODE_REASONS = new Set(['iban-bank-code', 'bic-institution', 'ifsc-prefix', 'paired-code']);
const NOT_ORG_WHY = new Set(['inside-other-name', 'word-not-name']);
const NOT_LINK_WHY = new Set([
	'no-tie',
	'code-names-other-bank',
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
const IBAN_BANK_CODE = {
	CH: [4, 9],
	DE: [4, 12],
	ES: [4, 8],
	FR: [4, 9],
	GB: [4, 8],
	IE: [4, 8],
	NL: [4, 8]
};
const FIELDS = {
	doc: ['id', 'note', 'nothing_to_find', ...LISTS, 'statement', 'passport'],
	org: ['id', 'page', 'start', 'end', 'text', 'org', 'by_name', 'candidates'],
	not_org: ['page', 'start', 'end', 'text', 'org', 'why'],
	identifier: ['id', 'page', 'start', 'end', 'text', 'kind', 'value', 'valid'],
	not_identifier: ['page', 'start', 'end', 'text', 'looks_like', 'why'],
	link: ['identifier', 'org', 'mention', 'reasons', 'via'],
	not_link: ['identifier', 'org', 'why'],
	statement: ['page', 'start', 'end', 'text', 'accounts', 'from', 'to'],
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

const ibanBankCode = (iban) => {
	const [from, to] = IBAN_BANK_CODE[iban.slice(0, 2)] ?? [];
	return { country: iban.slice(0, 2), code: from === undefined ? null : iban.slice(from, to) };
};
const bicMatches = (entry, bic) => entry.codes.bic.some((b) => b.slice(0, 6) === bic.slice(0, 6));

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
	/[A-Z0-9<]{30,}/g
];

// Labels.
const labels = readLabels(new URL('labels.json', SAMPLE));
if (labels.directory_version !== directory.version)
	fail(
		'labels.json',
		`directory_version ${labels.directory_version}, directory is version ${directory.version}`
	);
if (!isDate(labels.reference_date)) fail('labels.json', 'reference_date is not a date');
const specDocs = new Map(spec.documents.map((doc) => [doc.id, doc]));
const labelled = new Set(labels.documents.map((doc) => doc.id));
for (const id of specDocs.keys())
	if (!labelled.has(id)) fail(`labels ${id}`, 'document in spec.json has no labels');

const statements = [];
const passports = [];
let spanCount = 0;
let linkCount = 0;

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
		spanCount++;
	}
	if (doc.nothing_to_find === true) {
		lint(doc, pages, where);
		continue;
	}

	const orgs = new Map();
	for (const o of doc.orgs) {
		const at = `${where} org ${o.id}`;
		fields(at, o, FIELDS.org);
		if (orgs.has(o.id)) fail(at, 'id used twice');
		orgs.set(o.id, o);
		const held = [...(holders.get(matchKey(o.text)) ?? [])].sort();
		if (o.by_name === 'exact') {
			if (!entries.has(o.org)) fail(at, `org ${o.org} is not in the directory`);
			if (held.length !== 1 || held[0] !== o.org)
				fail(
					at,
					`by_name exact, but the directory gives ${held.join(', ') || 'nothing'} for ${JSON.stringify(o.text)}`
				);
		} else if (o.by_name === 'clash') {
			const candidates = [...(o.candidates ?? [])].sort();
			if (o.org !== null) fail(at, 'a clash has org null');
			if (held.length < 2 || candidates.join() !== held.join())
				fail(
					at,
					`candidates ${candidates.join(', ')}, the directory gives ${held.join(', ') || 'nothing'}`
				);
		} else if (o.by_name === 'none') {
			if (!entries.has(o.org)) fail(at, `org ${o.org} is not in the directory`);
			if (held.length) fail(at, `by_name none, but the directory gives ${held.join(', ')}`);
		} else fail(at, `by_name ${o.by_name} is not exact, clash or none`);
		if (o.by_name !== 'clash' && o.candidates) fail(at, 'only a clash has candidates');
	}
	const resolved = new Set(doc.orgs.map((o) => o.org).filter(Boolean));

	for (const n of doc.not_orgs) {
		const at = `${where} not_org ${JSON.stringify(n.text)}`;
		fields(at, n, FIELDS.not_org);
		if (!entries.has(n.org)) fail(at, `org ${n.org} is not in the directory`);
		const inside = doc.orgs.some(
			(o) =>
				o.page === n.page &&
				o.start <= n.start &&
				n.end <= o.end &&
				o.end - o.start > n.end - n.start &&
				o.org !== n.org
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
	for (const n of doc.not_identifiers) {
		const at = `${where} not_identifier ${JSON.stringify(n.text)}`;
		fields(at, n, FIELDS.not_identifier);
		if (!KINDS[n.looks_like]) fail(at, `looks_like ${n.looks_like} is not a kind`);
		if (!NOT_IDENTIFIER_WHY.has(n.why)) fail(at, `why ${n.why} is not in the list`);
	}

	const lineOf = (span) =>
		pages[span.page - 1].slice(0, span.start).filter((c) => c === '\n').length;
	const statementValues = new Set((doc.statement?.accounts ?? []).map((id) => ids.get(id)?.value));
	const codeLinked = (v, org) =>
		doc.links.some(
			(l) =>
				l.identifier === v.id &&
				l.org === org &&
				(l.reasons.includes('ifsc-prefix') || l.reasons.includes('bic-institution'))
		);
	// Why a reason does not hold for identifier id at org, or null when it holds.
	const reasonProblem = (reason, id, org, via) => {
		const entry = entries.get(org);
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
			if (via.page !== id.page || Math.abs(lineOf(via) - lineOf(id)) > 1)
				return `${via.value} is not on the same or the next line`;
			return null;
		}
		if (!statementValues.has(id.value)) return "the identifier is not the statement's account";
		if (resolved.size !== 1 || !resolved.has(org))
			return `the document names ${[...resolved].join(', ') || 'no organisation'}, not exactly ${org}`;
		return null;
	};
	const linked = new Map();
	for (const link of doc.links) {
		const at = `${where} link ${link.identifier}->${link.org}`;
		fields(at, link, FIELDS.link);
		linkCount++;
		const id = ids.get(link.identifier);
		const entry = entries.get(link.org);
		if (!id) fail(at, 'identifier is not in this document');
		if (!entry) fail(at, 'org is not in the directory');
		if (link.mention !== null && orgs.get(link.mention)?.org !== link.org)
			fail(at, `mention ${link.mention} is not a mention of ${link.org} in this document`);
		if (!id || !entry) continue;
		if (id.valid === false) fail(at, 'links an identifier that fails its check');
		const key = `${id.value} ${link.org}`;
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
		for (const reason of REASONS) {
			const listed = link.reasons.includes(reason);
			let problem;
			if (reason === 'paired-code' && !listed) {
				const pairs = doc.identifiers.filter((v) => !reasonProblem(reason, id, link.org, v));
				problem = pairs.length ? null : 'no code beside it';
			} else problem = reasonProblem(reason, id, link.org, ids.get(link.via));
			if (listed && problem) fail(at, `${reason}: ${problem}`);
			if (!listed && !problem) fail(at, `${reason} holds but is not listed`);
		}
	}
	for (const i of doc.identifiers) {
		if (!['iban', 'bic', 'ifsc'].includes(i.kind) || i.valid !== true) continue;
		for (const entry of directory.entries) {
			const named =
				(i.kind === 'iban' &&
					entry.codes.iban_bank_code.some(
						(c) =>
							c.country === ibanBankCode(i.value).country && c.code === ibanBankCode(i.value).code
					)) ||
				(i.kind === 'bic' && bicMatches(entry, i.value)) ||
				(i.kind === 'ifsc' && entry.codes.ifsc_prefix.includes(i.value.slice(0, 4)));
			if (named && !linked.has(`${i.value} ${entry.id}`))
				fail(`${where} identifier ${i.id}`, `its code names ${entry.id}, but there is no link`);
		}
	}
	const linkedTo = new Map();
	for (const key of linked.keys()) {
		const [value, org] = key.split(' ');
		if (linkedTo.has(value) && linkedTo.get(value) !== org)
			fail(where, `${value} is linked to both ${linkedTo.get(value)} and ${org}`);
		linkedTo.set(value, org);
	}

	for (const n of doc.not_links) {
		const at = `${where} not_link ${n.identifier}->${n.org}`;
		fields(at, n, FIELDS.not_link);
		const id = ids.get(n.identifier);
		if (!id) fail(at, 'identifier is not in this document');
		if (!entries.has(n.org)) fail(at, 'org is not in the directory');
		if (!id) continue;
		if (linked.has(`${id.value} ${n.org}`)) fail(at, 'the same pair is also a link');
		const other = linkedTo.get(id.value);
		if (!NOT_LINK_WHY.has(n.why)) fail(at, `why ${n.why} is not in the list`);
		else if (n.why === 'invalid-identifier' && id.valid !== false)
			fail(at, 'invalid-identifier, but the identifier passes its check');
		else if (
			n.why === 'code-names-other-bank' &&
			!(
				other &&
				other !== n.org &&
				[...linked.values()].some(
					(l) =>
						ids.get(l.identifier)?.value === id.value && l.reasons.some((r) => CODE_REASONS.has(r))
				)
			)
		)
			fail(at, 'code-names-other-bank, but no code ties the identifier to another bank');
		else if (n.why === 'no-tie' && other)
			fail(at, `no-tie, but the identifier is linked to ${other}`);
		else if (n.why === 'name-not-this-bank' && resolved.has(n.org))
			fail(at, 'name-not-this-bank, but the document names this bank');
	}

	if (doc.statement) {
		const s = doc.statement;
		const at = `${where} statement`;
		fields(at, s, FIELDS.statement);
		if (!s.accounts?.length || s.accounts.some((id) => !ids.has(id)))
			fail(at, 'accounts must name identifiers of this document');
		if (!isDate(s.from) || !isDate(s.to) || s.from > s.to)
			fail(at, `period ${s.from}..${s.to} is not a pair of dates in order`);
		const read = datesIn(s.text);
		if (read[0] !== s.from || read[1] !== s.to)
			fail(at, `the text gives ${read.join(' and ') || 'no dates'}, not ${s.from} and ${s.to}`);
		if (s.accounts?.length && ids.has(s.accounts[0]))
			statements.push({
				doc: doc.id,
				account: ids.get(s.accounts[0]).value,
				from: s.from,
				to: s.to,
				created: source.created
			});
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
const byAccount = Map.groupBy(statements, (s) => s.account);
const latestWant = answer(
	[...byAccount].map(([account, list]) => ({
		account,
		document: list.reduce((a, b) => (b.to > a.to ? b : a)).doc
	}))
);
if (!sameSet(latestWant, answer(labels.answers.latest_statement)))
	fail('labels answers', `latest_statement should be ${[...latestWant].join(' ')}`);
for (const [account, list] of byAccount) {
	if (list.length < 2) continue;
	const byPeriod = [...list]
		.sort((a, b) => a.to.localeCompare(b.to))
		.map((s) => s.doc)
		.join();
	const byFileDate = [...list]
		.sort((a, b) => a.created.localeCompare(b.created))
		.map((s) => s.doc)
		.join();
	if (byPeriod === byFileDate)
		fail(`statements of ${account}`, 'file dates agree with the periods; they must not');
}

// Completeness: every directory name and every identifier-shaped number on a
// page lies in a labelled span, so nothing findable is left unlabelled.
function lint(doc, pages, where) {
	const covered = (lists, page, start, end) =>
		lists.some((s) => s.page === page && s.start < end && start < s.end);
	pages.forEach((chars, index) => {
		const page = index + 1;
		const lower = chars.map((c) => c.toLowerCase());
		const word = (c) =>
			c !== undefined &&
			/[\p{L}\p{M}\p{N}]/u.test(c) &&
			!/[\p{scx=Han}\p{scx=Hiragana}\p{scx=Katakana}]/u.test(c);
		for (const [first, names] of nameIndex) {
			for (let i = 0; i < lower.length; i++) {
				if (lower[i] !== first) continue;
				for (const name of names) {
					if (!name.every((c, j) => lower[i + j] === c)) continue;
					const end = i + name.length;
					if ((word(name[0]) && word(chars[i - 1])) || (word(name.at(-1)) && word(chars[end])))
						continue;
					const ids = nameHolders.get(name.join(''));
					const ofThisName = (o) => ids.has(o.org) || (o.candidates ?? []).some((c) => ids.has(c));
					if (
						!covered((doc.orgs ?? []).filter(ofThisName), page, i, end) &&
						!covered(doc.not_orgs ?? [], page, i, end)
					) {
						fail(
							where,
							`page ${page} names ${JSON.stringify(chars.slice(i, end).join(''))} (${[...ids].join(', ')}) at ${i}, which no org or not_org label covers`
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
				if (
					!covered(
						[...(doc.identifiers ?? []), ...(doc.not_identifiers ?? [])],
						page,
						start,
						start + [...m[0]].length
					)
				)
					fail(
						where,
						`page ${page} holds ${JSON.stringify(m[0])} at ${start}, which no identifier or not_identifier label covers`
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
	`ok: ${built.length} documents, ${spanCount} spans, ${linkCount} links, ${directory.entries.length} directory entries${process.argv.includes('--pdfkit') ? ', PDFKit text matches' : ''}`
);
