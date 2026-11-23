// Scores one extractor output against labels.json, as sample/FORMAT.md says:
//   node sample/tools/score.mjs <output.json> [--json <path>]
// Prints a table and writes the score as JSON (default: <output>.score.json).
// Exits 1 when a certain row is wrong or a key merges, 2 when the output
// breaks the format. A likely row and a left-out row never fail.
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { SAMPLE } from './generate.mjs';
import { ibanProblem, KINDS } from './lib/ids.mjs';
import { locate, readsBack } from './lib/find.mjs';
import { readLabels } from './lib/labels.mjs';
import { isMain } from './lib/main.mjs';
import {
	CODE_REASONS,
	codeKey,
	documentOfKey,
	isCodeKey,
	mayBeCertainIdentifier,
	mayBeCertainJoin,
	mayBeCertainLink,
	mayBeCertainPassport,
	nameOfKey,
	TIERS
} from './lib/tiers.mjs';
import { readTextSet, TEXT_SETS } from './text.mjs';

export class FormatError extends Error {}

const REASONS = new Set([
	'iban-bank-code',
	'bic-institution',
	'ifsc-prefix',
	'paired-code',
	'inside-iban',
	'same-account',
	'statement-header'
]);
const SETTLED_BY = new Set(['name', 'mention', 'code', 'directory']);
const SAME_ORG_REASONS = new Set([
	'directory',
	'spells-code',
	'statement-header',
	'printed-together'
]);
const DATE = /^\d{4}-\d\d-\d\d$/;
const TIERS_HELP = 'FORMAT.md, "Tiers"';
const KEYS_HELP = 'FORMAT.md, "No merging"';
const MEASURES = [
	'mentions',
	'org_directory',
	'identifiers',
	'links',
	'same_org',
	'statement_period',
	'statement_account',
	'statement_issuer',
	'passport_holder',
	'passport_number',
	'passport_expiry',
	'passport_current',
	'answer_latest_statement',
	'answer_current_passport'
];

// A day of the calendar, YYYY-MM-DD.
const isDate = (v) => {
	if (typeof v !== 'string' || !DATE.test(v)) return false;
	const [y, m, d] = v.split('-').map(Number);
	const day = new Date(Date.UTC(y, m - 1, d));
	return day.getUTCFullYear() === y && day.getUTCMonth() === m - 1 && day.getUTCDate() === d;
};
const spanKey = (s) => JSON.stringify([s.page, s.start, s.end]);

// Checks everything FORMAT.md says a well-formed output is, and returns it
// with every reference resolved: keys to cards, identifier ids to entries.
// Scoring trusts this structure and reads nothing raw.
function parseOutput(output, textSet, entries) {
	const fail = (where, what) => {
		throw new FormatError(`${where}: ${what}`);
	};
	const isObject = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
	const list = (v, where) => {
		if (v === undefined) return [];
		if (!Array.isArray(v)) fail(where, 'must be a list');
		return v;
	};
	if (!isObject(output)) fail('output', 'must be an object');
	if (output.format === 1)
		fail(
			'format',
			`format 1 has no tiers. Write format 3: a tier on every link, tiers on every statement and passport, and same_org rows for keys that are one organisation (${TIERS_HELP})`
		);
	if (output.format === 2)
		fail(
			'format',
			`format 2 shares a printed name's key across documents and takes an expiry's century from the zone; write format 3, where a name's key is name:<document id>:<name> and a certain expiry cites a printed date with a four-digit year (FORMAT.md, "Changes in round 7")`
		);
	if (output.format !== 3) fail('format', 'must be 3');
	if (typeof output.extractor !== 'string' || !output.extractor)
		fail('extractor', 'must be a name');
	if (output.directory !== 'on' && output.directory !== 'off')
		fail('directory', 'must be "on" or "off"');
	if (!TEXT_SETS.includes(output.text_set))
		fail('text_set', `must be one of ${TEXT_SETS.join(', ')}`);
	if (output.text_set !== textSet.index.text_set)
		fail('text_set', `is ${output.text_set}, but the text set read is ${textSet.index.text_set}`);
	if (!isObject(output.organisations)) fail('organisations', 'must map keys to organisations');
	const cards = new Map();
	for (const [key, card] of Object.entries(output.organisations)) {
		const where = `organisations[${JSON.stringify(key)}]`;
		if (!isCodeKey(key) && !textSet.docs.has(documentOfKey(key)))
			fail(
				where,
				`a key is an institution code's key ("Code keys") or a document key, name:<document id>:<name> (${KEYS_HELP})`
			);
		if (!isObject(card)) fail(where, 'must be {name, directory}');
		if (card.name !== null && (typeof card.name !== 'string' || !card.name))
			fail(where, 'name must be a name or null');
		if (card.directory !== null && (typeof card.directory !== 'string' || !card.directory))
			fail(where, 'directory must be a directory id or null');
		cards.set(key, { name: card.name, directory: card.directory });
	}
	const orgKey = (key, where, nullable) => {
		if (key === null && nullable) return null;
		if (typeof key !== 'string' || !cards.has(key))
			fail(where, `${JSON.stringify(key)} is not a key in organisations`);
		return key;
	};
	const span = (s, pages, where) => {
		if (!isObject(s)) fail(where, 'must be a span');
		if (!Number.isInteger(s.page) || s.page < 1 || s.page > pages.length)
			fail(where, `page ${s.page} is not a page of the document`);
		const length = [...pages[s.page - 1].text].length;
		if (
			!Number.isInteger(s.start) ||
			!Number.isInteger(s.end) ||
			s.start < 0 ||
			s.start > s.end ||
			s.end > length
		)
			fail(where, `start and end must be whole numbers, 0 <= start <= end <= ${length}`);
		if (typeof s.text !== 'string' || !readsBack(pages[s.page - 1].text, s))
			fail(where, `${JSON.stringify(s.text)} is not the text at ${s.start}..${s.end}`);
		return { page: s.page, start: s.start, end: s.end, text: s.text };
	};
	// One physical occurrence is one row.
	const oneRowPerSpan = (rows, where) => {
		const at = new Map();
		rows.forEach((row, i) => {
			const k = spanKey(row);
			if (at.has(k)) fail(`${where}[${i}]`, `has the span of ${where}[${at.get(k)}]`);
			at.set(k, i);
		});
	};
	// Why an IBAN cannot be certain, for the message that refuses it.
	const ibanWhy = ({ kind, value }) => {
		const problem = kind === 'iban' ? ibanProblem(value) : null;
		return problem ? `: ${problem}` : '';
	};
	const tierOf = (v, where) =>
		TIERS.has(v) ? v : fail(where, `needs a tier, "certain" or "likely" (${TIERS_HELP})`);
	// The tiers of the facts an entry gives: one for each fact that is not null.
	const factTiers = (tiers, facts, where, mayBeCertain, hint = () => '') => {
		if (!isObject(tiers))
			fail(`${where}.tiers`, `needs a tier for ${facts.join(', ')} (${TIERS_HELP})`);
		const out = {};
		for (const fact of facts) {
			out[fact] = tierOf(tiers[fact], `${where}.tiers.${fact}`);
			if (out[fact] === 'certain' && !mayBeCertain(fact))
				fail(`${where}.tiers.${fact}`, `cannot be certain here${hint(fact)} (${TIERS_HELP})`);
		}
		return out;
	};
	const seen = new Set();
	const docs = new Map();
	list(output.documents, 'documents').forEach((doc, n) => {
		const where = `documents[${n}]`;
		if (!isObject(doc)) fail(where, 'must be an object');
		const input = textSet.docs.get(doc.id);
		if (!input) fail(where, `${JSON.stringify(doc.id)} is not a document of the text set`);
		if (seen.has(doc.id)) fail(where, `${doc.id} appears twice`);
		seen.add(doc.id);
		const pages = input.pages;
		const mentions = list(doc.orgs, `${where}.orgs`).map((m, i) => {
			const w = `${where}.orgs[${i}]`;
			const at = span(m, pages, w);
			if (m.settled_by !== undefined && m.settled_by !== null && !SETTLED_BY.has(m.settled_by))
				fail(w, `settled_by must be one of ${[...SETTLED_BY].join(', ')}`);
			return { ...at, org: orgKey(m.org, w, true), settled_by: m.settled_by ?? null };
		});
		oneRowPerSpan(mentions, `${where}.orgs`);
		const identifiers = new Map();
		list(doc.identifiers, `${where}.identifiers`).forEach((id, i) => {
			const w = `${where}.identifiers[${i}]`;
			const at = span(id, pages, w);
			if (typeof id.id !== 'string' || identifiers.has(id.id)) fail(w, 'id must be unique');
			if (!Object.hasOwn(KINDS, id.kind)) fail(w, `kind ${JSON.stringify(id.kind)} is not known`);
			if (typeof id.value !== 'string') fail(w, 'value must be text');
			if (![true, false, null].includes(id.valid)) fail(w, 'valid must be true, false or null');
			const tier = tierOf(id.tier, `${w}.tier`);
			const identifier = {
				id: id.id,
				...at,
				kind: id.kind,
				value: id.value,
				valid: id.valid,
				tier
			};
			if (tier === 'certain' && !mayBeCertainIdentifier(identifier, output.directory, entries))
				fail(
					`${w}.tier`,
					`kind ${id.kind} cannot be certain here${ibanWhy(identifier)} (${TIERS_HELP})`
				);
			identifiers.set(id.id, identifier);
		});
		oneRowPerSpan([...identifiers.values()], `${where}.identifiers`);
		const ident = (id, w) => {
			if (!identifiers.has(id))
				fail(w, `${JSON.stringify(id)} is not an identifier of the document`);
			return identifiers.get(id);
		};
		const optionalIdent = (id, w) => (id === null || id === undefined ? null : ident(id, w));
		const links = list(doc.links, `${where}.links`).map((link, i) => {
			const w = `${where}.links[${i}]`;
			if (!isObject(link)) fail(w, 'must be an object');
			const identifier = ident(link.identifier, w);
			const org = orgKey(link.org, w, false);
			if (!Array.isArray(link.reasons) || !link.reasons.length) fail(w, 'needs a reason');
			for (const r of link.reasons)
				if (!REASONS.has(r)) fail(w, `reason ${JSON.stringify(r)} is not known`);
			if (link.reasons.includes('paired-code') && (link.via === undefined || link.via === null))
				fail(w, 'paired-code needs a via');
			const canonical = codeKey(identifier);
			const own = link.reasons.find((r) => CODE_REASONS.get(r) === identifier.kind);
			if (own && canonical !== null && link.org !== canonical)
				fail(
					w,
					`a link by ${own} goes to its code's key, ${JSON.stringify(canonical)} (FORMAT.md, "Code keys")`
				);
			const tier = tierOf(link.tier, `${w}.tier`);
			const test = { identifier, reasons: link.reasons, card: cards.get(org) };
			if (tier === 'certain' && !mayBeCertainLink(test, output.directory, entries))
				fail(
					`${w}.tier`,
					`no reason passes its test for certain${ibanWhy(identifier)} (${TIERS_HELP})`
				);
			return { identifier, org, reasons: link.reasons, via: optionalIdent(link.via, w), tier };
		});
		let statement = null;
		if (doc.statement !== undefined && doc.statement !== null) {
			const w = `${where}.statement`;
			const s = doc.statement;
			if (!isObject(s)) fail(w, 'must be an object');
			if (!isDate(s.from) || !isDate(s.to)) fail(w, 'from and to must be dates, YYYY-MM-DD');
			const accounts = list(s.accounts, `${w}.accounts`).map((id) => ident(id, w));
			if (s.issuer === undefined) fail(w, 'issuer must be a key or null');
			const issuer = orgKey(s.issuer, w, true);
			const facts = [
				'period',
				...(accounts.length ? ['accounts'] : []),
				...(issuer ? ['issuer'] : [])
			];
			// A statement is read from layout and wording, so none of it is certain.
			const tiers = factTiers(s.tiers, facts, w, () => false);
			statement = { from: s.from, to: s.to, accounts, issuer, tiers };
		}
		let passport = null;
		if (doc.passport !== undefined && doc.passport !== null) {
			const w = `${where}.passport`;
			const p = doc.passport;
			if (!isObject(p)) fail(w, 'must be an object');
			const holder = p.holder ?? null;
			if (
				holder !== null &&
				(!isObject(holder) ||
					typeof holder.surname !== 'string' ||
					typeof holder.given !== 'string')
			)
				fail(w, 'holder must be {surname, given} or null');
			const number = optionalIdent(p.number, `${w}.number`);
			const mrz = optionalIdent(p.mrz, `${w}.mrz`);
			if (p.issued !== undefined && p.issued !== null && !isDate(p.issued))
				fail(w, 'issued must be a date, YYYY-MM-DD, or null');
			const expiry = p.expiry ?? null;
			if (expiry !== null && expiry !== 'cannot-tell' && !isDate(expiry))
				fail(w, 'expiry must be a date, YYYY-MM-DD, "cannot-tell" or null');
			const expiryAt =
				p.expiry_at === undefined || p.expiry_at === null
					? null
					: span(p.expiry_at, pages, `${w}.expiry_at`);
			const current = p.current ?? null;
			if (![true, false, null].includes(current)) fail(w, 'current must be true, false or null');
			const given = { holder, number, expiry, current };
			const facts = Object.keys(given).filter((f) => given[f] !== null);
			const tiers = factTiers(
				p.tiers,
				facts,
				w,
				(fact) => mayBeCertainPassport(fact, { mrz, number, holder, expiry, expiryAt }),
				(fact) =>
					fact === 'expiry'
						? `: the zone gives two digits of the year, so a certain expiry cites in expiry_at a printed date whose four-digit year ends in them and whose day is the zone's`
						: ''
			);
			passport = {
				holder,
				number,
				mrz,
				issued: p.issued ?? null,
				expiry,
				expiryAt,
				current,
				tiers
			};
		}
		// The keys the document uses, which a same_org row in it may join.
		const used = new Set(
			[...mentions.map((m) => m.org), ...links.map((l) => l.org), statement?.issuer].filter(Boolean)
		);
		for (const k of used) {
			const owner = documentOfKey(k);
			if (owner !== null && owner !== doc.id)
				fail(
					where,
					`key ${k} is a key of document ${owner}; a printed name's key is for one document (${KEYS_HELP})`
				);
		}
		const sameOrg = list(doc.same_org, `${where}.same_org`).map((row, i) => {
			const w = `${where}.same_org[${i}]`;
			if (!isObject(row)) fail(w, 'must be an object');
			if (!Array.isArray(row.keys) || row.keys.length !== 2 || row.keys[0] === row.keys[1])
				fail(w, 'keys must be two different keys');
			const keys = row.keys.map((k) => orgKey(k, w, false)).sort();
			for (const k of keys)
				if (!used.has(k))
					fail(w, `key ${k} is not used by a mention, link or issuer of the document`);
			if (!SAME_ORG_REASONS.has(row.reason))
				fail(w, `reason must be one of ${[...SAME_ORG_REASONS].join(', ')}`);
			const tier = tierOf(row.tier, `${w}.tier`);
			// The directory decides, from the printed names on one key and the
			// codes on the other, never from the ids the cards give.
			const names = (k) => mentions.filter((m) => m.org === k).map((m) => m.text);
			const codes = (k) =>
				links
					.filter(
						(l) => l.org === k && l.reasons.some((r) => CODE_REASONS.get(r) === l.identifier.kind)
					)
					.map((l) => l.identifier);
			const join = (n, c) =>
				mayBeCertainJoin(
					{ reason: row.reason, names: names(n), codes: codes(c) },
					output.directory,
					entries
				);
			if (tier === 'certain' && !join(keys[0], keys[1]) && !join(keys[1], keys[0]))
				fail(
					`${w}.tier`,
					`only a directory row whose printed name and code each look up to one entry, the same one, is certain (${TIERS_HELP})`
				);
			return { keys, reason: row.reason, tier };
		});
		docs.set(doc.id, {
			orgs: mentions,
			identifiers: [...identifiers.values()],
			links,
			statement,
			passport,
			sameOrg,
			used
		});
	});
	const answers = isObject(output.answers) ? output.answers : fail('answers', 'must be an object');
	const latest = list(answers.latest_statement, 'answers.latest_statement').map((a, i) => {
		if (typeof a?.account !== 'string' || !textSet.docs.has(a.document))
			fail(`answers.latest_statement[${i}]`, 'needs an account and a document of the text set');
		return a;
	});
	const passports = list(answers.current_passport, 'answers.current_passport').map((a, i) => {
		if (typeof a?.holder !== 'string' || !textSet.docs.has(a.document))
			fail(`answers.current_passport[${i}]`, 'needs a holder and a document of the text set');
		return a;
	});
	return {
		extractor: output.extractor,
		directory: output.directory,
		textSet: output.text_set,
		cards,
		docs,
		answers: { latest, passports }
	};
}

function tallies() {
	const rows = new Map();
	const items = { wrong: [], left_out: [], impossible: [] };
	// tier is the row's tier, or null for a measure that has none or a row the
	// output did not give.
	const add = (measure, lang, outcome, document, what, tier = null) => {
		if (!rows.has(measure)) rows.set(measure, { total: zero(), by_lang: {}, by_tier: {} });
		const row = rows.get(measure);
		row.total[outcome]++;
		if (lang) (row.by_lang[lang] ??= zero())[outcome]++;
		if (tier) (row.by_tier[tier] ??= { right: 0, wrong: 0 })[outcome]++;
		if (outcome !== 'right') items[outcome].push({ measure, document, what, tier });
	};
	return { rows, items, add };
}
const zero = () => ({ right: 0, wrong: 0, left_out: 0, impossible: 0 });

// Pairs labelled items with output items that have the same key. Equal keys
// pair by position on the page (page, then start), first with first, so the
// order of an output's rows never changes a pairing.
const byPosition = (a, b) => a.page - b.page || a.start - b.start;
function pair(labelled, output, keyOf) {
	const waiting = new Map();
	for (const o of [...output].sort(byPosition)) {
		const k = keyOf(o);
		if (!waiting.has(k)) waiting.set(k, []);
		waiting.get(k).push(o);
	}
	const partner = new Map();
	for (const l of [...labelled].sort(byPosition)) {
		const queue = waiting.get(keyOf(l));
		if (queue?.length) partner.set(l, queue.shift());
	}
	const used = new Set(partner.values());
	return { partner, unmatched: [...output].sort(byPosition).filter((o) => !used.has(o)) };
}

const squash = (v) => v.replace(/\s/gu, '');
// A card name ignores case and all whitespace: 三菱 UFJ 銀行 is 三菱UFJ銀行.
const cardName = (v) => v.normalize('NFKC').toUpperCase().replace(/\s/gu, '');
const normName = (v) =>
	v
		.normalize('NFKC')
		.toUpperCase()
		.replace(/\s+/gu, ' ')
		.replace(/\s*,\s*/u, ', ')
		.trim();
const same = (a, b) => a.kind === b.kind && a.value === b.value;
// Identifier kinds that carry their institution inside them, so an extractor
// can know a bank from the code alone.
const CODE_KINDS = new Set(['ifsc', 'bic', 'iban']);

export function score(labels, textSet, rawOutput, directory) {
	const entries = new Map(directory.entries.map((e) => [e.id, e]));
	const out = parseOutput(rawOutput, textSet, entries);
	const mode = out.directory;
	const { rows, items, add } = tallies();
	const langOf = (id) => textSet.docs.get(id)?.lang;
	const labelDocs = new Map(labels.documents.map((d) => [d.id, d]));
	const dirToLocal = new Map(
		Object.entries(labels.organisations)
			.filter(([, o]) => o.directory)
			.map(([id, o]) => [o.directory, id])
	);

	// The organisation a labelled mention is, in this run's mode (README,
	// "Mentions, with the directory on and off"); null for no organisation.
	const target = (m) => {
		if (m.resolved_by_document) return m.resolved_by_document.org;
		if (mode === 'on' && m.directory.lookup === 'exact')
			return dirToLocal.get(m.directory.id) ?? `directory:${m.directory.id}`;
		return null;
	};
	const directoryIdOf = (t) =>
		t.startsWith('directory:') ? t.slice(10) : (labels.organisations[t]?.directory ?? null);

	// The names a card for an organisation may carry: its labelled spellings,
	// the text of every labelled mention of it, and with the directory on its
	// directory entry's names.
	const names = new Map();
	const addName = (t, name) => {
		if (!names.has(t)) names.set(t, new Set());
		names.get(t).add(cardName(name));
	};
	for (const [id, o] of Object.entries(labels.organisations))
		for (const name of [o.name, ...o.names]) addName(id, name);
	for (const d of labels.documents)
		for (const m of d.orgs ?? []) if (target(m)) addName(target(m), m.text);
	const entryNames = new Map(
		mode === 'on'
			? directory.entries.map((e) => [e.id, [e.name, ...e.names.map((n) => n.text)]])
			: []
	);
	const nameFits = (t, name) =>
		names.get(t)?.has(cardName(name)) ||
		(entryNames.get(directoryIdOf(t)) ?? []).some((n) => cardName(n) === cardName(name));

	const pagesOf = (id) => textSet.docs.get(id).pages;
	const possible = (docId, s, value) =>
		locate(pagesOf(docId)[s.page - 1].text, s.text, value) !== null;
	// Only a page the OCR read can move a span. Every other page holds the
	// same text in both text sets and matches by span.
	const ocrRead = (docId, page) => out.textSet !== 'text' && pagesOf(docId)[page - 1].scanned;

	const labelled = (id) => {
		const d = labelDocs.get(id);
		return d && !d.nothing_to_find
			? d
			: {
					id,
					orgs: [],
					not_orgs: [],
					identifiers: [],
					not_identifiers: [],
					links: [],
					not_links: []
				};
	};
	const empty = {
		orgs: [],
		identifiers: [],
		links: [],
		statement: null,
		passport: null,
		sameOrg: [],
		used: new Set()
	};
	const docIds = [...textSet.docs.keys()];
	const work = docIds.map((id) => {
		const label = labelled(id);
		const output = out.docs.get(id) ?? empty;
		const identById = new Map(label.identifiers.map((i) => [i.id, i]));
		const impossibleIdent = new Set(
			label.identifiers.filter((i) => !possible(id, i, i.value)).map((i) => i.id)
		);
		const impossibleMention = new Set(label.orgs.filter((m) => !possible(id, m)).map((m) => m.id));
		const onPage = (page, text) => JSON.stringify([page, text]);
		const mentionKey = (m) => (ocrRead(id, m.page) ? onPage(m.page, m.text) : spanKey(m));
		const identKey = (i) => (ocrRead(id, i.page) ? onPage(i.page, squash(i.value)) : spanKey(i));
		const decoyKey = (n) => (ocrRead(id, n.page) ? onPage(n.page, squash(n.text)) : spanKey(n));
		const idents = pair(label.identifiers, output.identifiers, identKey);
		// The labelled identifier an output identifier is: the one it pairs
		// with, when the kind and value agree.
		const labelOf = new Map();
		for (const [l, o] of idents.partner) if (same(l, o)) labelOf.set(o, l);
		return {
			id,
			lang: langOf(id),
			label,
			output,
			identById,
			impossibleIdent,
			impossibleMention,
			mentionKey,
			identKey,
			decoyKey,
			mentions: pair(label.orgs, output.orgs, mentionKey),
			idents,
			labelOf
		};
	});

	// Organisation keys. A key maps to the labelled organisations its
	// mentions land on. A key no mention lands on maps to the organisations
	// the labels link its linked codes to (an IFSC, a BIC or an IBAN names its
	// bank by itself); an account tied to it never says who it is.
	const byMention = new Map();
	const byCode = new Map();
	const note = (evidence, key, t) => {
		if (!evidence.has(key)) evidence.set(key, new Set());
		evidence.get(key).add(t);
	};
	for (const w of work)
		for (const [l, o] of w.mentions.partner)
			if (o.org && target(l)) note(byMention, o.org, target(l));
	for (const w of work)
		for (const link of w.output.links) {
			const code = w.labelOf.get(link.identifier);
			if (!code || !CODE_KINDS.has(code.kind)) continue;
			for (const l of w.label.links)
				if (same(w.identById.get(l.identifier), code)) note(byCode, link.org, l.org);
		}
	const targetsOf = (key) => byMention.get(key) ?? byCode.get(key) ?? new Set();
	const keys = [...out.cards.keys()];
	// The no-merge rule, read from the output alone. A code's key is spelt
	// from its code, which the format checks, so one key never holds two
	// codes. A key that holds a code is that code and no printed name.
	const codeKeys = new Set();
	const named = new Set();
	for (const d of out.docs.values()) {
		for (const m of d.orgs) if (m.org) named.add(m.org);
		for (const l of d.links)
			if (l.reasons.some((r) => CODE_REASONS.get(r) === l.identifier.kind) && codeKey(l.identifier))
				codeKeys.add(l.org);
	}
	const merges = keys.flatMap((k) => {
		const organisations = [...targetsOf(k)].sort();
		return [
			...(organisations.length > 1 ? [{ key: k, why: 'organisations', organisations }] : []),
			...(codeKeys.has(k) && named.has(k) ? [{ key: k, why: 'name-and-code' }] : [])
		];
	});
	// Only papers of two organisations make a key's rows wrong. The other
	// merges fail the run, and the labels still say who the key is.
	const merged = new Set(merges.filter((m) => m.why === 'organisations').map((m) => m.key));
	const single = (key) => (targetsOf(key).size === 1 ? [...targetsOf(key)][0] : null);

	// A card states who it is. A name or a directory id that belongs to
	// another organisation puts this organisation's papers on a wrong card.
	const wrongCards = [];
	for (const k of keys) {
		const t = single(k);
		if (t === null) continue;
		const { name, directory: id } = out.cards.get(k);
		const claims = [];
		if (name !== null && !nameFits(t, name)) claims.push(`name ${JSON.stringify(name)}`);
		if (id !== null && id !== directoryIdOf(t)) claims.push(`directory id ${id}`);
		if (claims.length) wrongCards.push({ key: k, organisation: t, claims });
	}
	const wrongCard = new Map(wrongCards.map((c) => [c.key, c]));
	const mapped = (key) => (!merged.has(key) && !wrongCard.has(key) ? single(key) : null);
	const reach = new Map();
	for (const k of keys)
		for (const t of targetsOf(k)) {
			if (!reach.has(t)) reach.set(t, []);
			reach.get(t).push(k);
		}
	const splits = [...reach]
		.filter(([, ks]) => ks.length > 1)
		.map(([t, ks]) => ({ organisation: t, keys: ks.sort() }))
		.sort((a, b) => a.organisation.localeCompare(b.organisation));
	// Document keys of two documents whose names are spelt the same. Nothing in
	// one document joins them; that is the engine's rule "may be the same
	// organisation", which a person confirms or rejects. This reports what the
	// rule will face and fails nothing (FORMAT.md, "No merging").
	const byName = new Map();
	for (const d of out.docs.values())
		for (const k of d.used) {
			const name = nameOfKey(k);
			if (name !== null) byName.set(name, [...(byName.get(name) ?? []), k]);
		}
	const sameName = [...byName.values()]
		.flatMap((ks) => ks.sort().flatMap((a, i) => ks.slice(i + 1).map((b) => [a, b])))
		.sort((x, y) => (x.join('\n') < y.join('\n') ? -1 : 1))
		.map((keys) => ({ keys, organisations: keys.map(single) }));
	const labelsSay = (one) =>
		sameName.filter(
			({ organisations: [x, y] }) => x !== null && y !== null && (x === y) === one
		).length;
	const keyVerdict = (key, want) => {
		if (merged.has(key)) return `key ${key} merges ${[...targetsOf(key)].join(' and ')}`;
		const card = wrongCard.get(key);
		if (card)
			return `key ${key} is ${card.organisation} but its card claims ${card.claims.join(' and ')}`;
		const got = mapped(key);
		if (got === null) return `key ${key} lands on no labelled organisation`;
		return got === want ? null : `key ${key} is ${got}, not ${want}`;
	};

	for (const k of keys) {
		const t = single(k);
		if (t === null) continue;
		const want = mode === 'off' ? null : directoryIdOf(t);
		const got = out.cards.get(k).directory;
		const what = `${k} (${t}) directory ${got ?? 'none'}`;
		if (got === want) add('org_directory', null, 'right', null, what);
		else if (got === null) add('org_directory', null, 'left_out', null, `${what}, labels ${want}`);
		else
			add(
				'org_directory',
				null,
				'wrong',
				null,
				mode === 'off'
					? `${what} in a run with the directory off`
					: `${what}, labels ${want ?? 'none'}`
			);
	}

	const wrongAttachments = [];
	for (const w of work) {
		const { id, lang, label, output } = w;
		const doc = (m, o) => `${m.text} at ${m.page}:${m.start}${o ? ` (${o.org})` : ''}`;

		for (const l of label.orgs) {
			const o = w.mentions.partner.get(l);
			const t = target(l);
			if (w.impossibleMention.has(l.id)) add('mentions', lang, 'impossible', id, doc(l));
			else if (o && o.org && mode === 'off' && o.settled_by === 'directory')
				add(
					'mentions',
					lang,
					'wrong',
					id,
					`${doc(l, o)} settled by the directory in a run with it off`
				);
			else if (t === null)
				add(
					'mentions',
					lang,
					o?.org ? 'wrong' : 'right',
					id,
					o?.org ? `${doc(l, o)}: the document settles no organisation` : doc(l)
				);
			else if (!o?.org) add('mentions', lang, 'left_out', id, `${doc(l)} is ${t}`);
			else {
				const why = keyVerdict(o.org, t);
				add(
					'mentions',
					lang,
					why ? 'wrong' : 'right',
					id,
					why ? `${doc(l, o)}: ${why}` : doc(l, o)
				);
			}
		}
		for (const o of w.mentions.unmatched) {
			if (!o.org) continue;
			const notOrgs = label.not_orgs.filter((n) => w.mentionKey(n) === w.mentionKey(o));
			const notOrg = notOrgs[0];
			// Text the labels say is no organisation puts its document on a wrong
			// card, unless the key is the card of the labelled name around it.
			const enclosing = (n) =>
				label.orgs.filter((l) => l.page === n.page && l.start <= n.start && n.end <= l.end);
			const t = single(o.org);
			const excluded = notOrgs.find(
				(n) => t === null || !enclosing(n).some((l) => target(l) === t)
			);
			if (excluded)
				wrongAttachments.push({
					document: id,
					key: o.org,
					organisation: t,
					mention: `${o.text} at ${o.page}:${o.start}`,
					why: excluded.why
				});
			add(
				'mentions',
				lang,
				'wrong',
				id,
				`${doc(o, o)}: ${notOrg ? `not an organisation (${notOrg.why})` : 'no organisation is mentioned there'}`
			);
		}

		for (const l of label.identifiers) {
			const o = w.idents.partner.get(l);
			const measure = `identifier:${l.kind}`;
			const what = `${l.kind} ${l.value}`;
			// An extractor that repairs the OCR text can still find it.
			if (!o) add(measure, lang, w.impossibleIdent.has(l.id) ? 'impossible' : 'left_out', id, what);
			else {
				// A label's valid of null is not scored: the kind has no check, or the checks disagree.
				const fields = l.valid === null ? ['kind', 'value'] : ['kind', 'value', 'valid'];
				const diff = fields.filter((f) => o[f] !== l[f]);
				add(
					measure,
					lang,
					diff.length ? 'wrong' : 'right',
					id,
					diff.length ? `${what}: given ${diff.map((f) => `${f} ${o[f]}`).join(', ')}` : what,
					o.tier
				);
			}
		}
		for (const o of w.idents.unmatched) {
			const decoy = label.not_identifiers.find((n) => w.decoyKey(n) === w.identKey(o));
			add(
				`identifier:${o.kind}`,
				lang,
				'wrong',
				id,
				`${o.kind} ${o.value}: ${decoy ? `a decoy (${decoy.why})` : 'not a labelled identifier'}`,
				o.tier
			);
		}

		// Links, by identifier kind and value and the organisation. Rows for
		// the same link are one link with the union of their reasons and vias.
		const labelLinks = label.links.map((l) => ({
			...l,
			ident: w.identById.get(l.identifier),
			found: false
		}));
		const inDocument = new Set(
			[
				...label.orgs.map((m) => m.resolved_by_document?.org),
				...label.links.map((l) => l.org)
			].filter(Boolean)
		);
		const groups = new Map();
		for (const link of output.links) {
			const ident = w.labelOf.get(link.identifier) ?? null;
			const t = mapped(link.org);
			const shown = ident ?? link.identifier;
			const k = JSON.stringify([
				ident !== null,
				shown.kind,
				shown.value,
				t,
				t === null ? link.org : null
			]);
			if (!groups.has(k))
				groups.set(k, {
					ident,
					shown,
					org: link.org,
					t,
					reasons: new Set(),
					vias: [],
					tier: 'likely'
				});
			const g = groups.get(k);
			for (const r of link.reasons) g.reasons.add(r);
			// One certain row makes the link a certain claim.
			if (link.tier === 'certain') g.tier = 'certain';
			const via = link.via && w.labelOf.get(link.via);
			if (via) g.vias.push(via);
		}
		const groupOrder = (a, b) =>
			byPosition(a.shown, b.shown) || (a.t ?? a.org).localeCompare(b.t ?? b.org);
		for (const g of [...groups.values()].sort(groupOrder)) {
			const what = `${g.shown.kind} ${g.shown.value} at ${g.org}`;
			const reasons = [...g.reasons].sort();
			if (g.t === null) {
				add('links', lang, 'wrong', id, `${what}: ${keyVerdict(g.org, null)}`, g.tier);
				continue;
			}
			if (!g.ident) {
				add(
					'links',
					lang,
					'wrong',
					id,
					`${what} (${g.t}): no such identifier in the labels`,
					g.tier
				);
				continue;
			}
			const right = labelLinks.find((l) => l.org === g.t && same(l.ident, g.ident));
			if (right) {
				const via = right.via && w.identById.get(right.via);
				// An IBAN whose check digits pass is held at the institution with
				// its bank code, whatever reasons the label lists for the link.
				const ownIban = (r) =>
					r === 'iban-bank-code' && g.ident.kind === 'iban' && KINDS.iban.valid(g.ident.value);
				const holds = reasons.filter(
					(r) =>
						ownIban(r) ||
						(right.reasons.includes(r) &&
							(r !== 'paired-code' || (via && g.vias.some((v) => same(v, via)))))
				);
				right.found ||= holds.length > 0;
				add(
					'links',
					lang,
					holds.length ? 'right' : 'wrong',
					id,
					holds.length
						? `${what} (${g.t})`
						: `${what} (${g.t}): no reason given holds (${reasons.join(', ')})`,
					g.tier
				);
				continue;
			}
			const notLink = label.not_links.find(
				(n) => n.org === g.t && same(w.identById.get(n.identifier), g.ident)
			);
			const why = notLink
				? `a not_link (${notLink.why})`
				: g.ident.valid === false
					? 'the identifier is invalid'
					: !inDocument.has(g.t)
						? `the document does not settle ${g.t}`
						: 'nothing ties them';
			add('links', lang, 'wrong', id, `${what} (${g.t}): ${why}`, g.tier);
		}
		for (const l of labelLinks) {
			if (l.found) continue;
			const what = `${l.ident.kind} ${l.ident.value} at ${l.org}`;
			const blocked =
				w.impossibleIdent.has(l.identifier) ||
				l.reasons.every((r) => r === 'paired-code' && w.impossibleIdent.has(l.via));
			add('links', lang, blocked ? 'impossible' : 'left_out', id, what);
		}

		// same_org: rows for the same two keys are one claim, right when both
		// keys are one labelled organisation. Two keys of one organisation in
		// the document that no right rows join are left out.
		const claims = new Map();
		for (const row of output.sameOrg) {
			const k = JSON.stringify(row.keys);
			if (!claims.has(k)) claims.set(k, { keys: row.keys, tier: 'likely' });
			if (row.tier === 'certain') claims.get(k).tier = 'certain';
		}
		const joinedTo = new Map();
		const root = (k) => (joinedTo.has(k) ? root(joinedTo.get(k)) : k);
		for (const k of [...claims.keys()].sort()) {
			const {
				keys: [a, b],
				tier
			} = claims.get(k);
			const [ta, tb] = [mapped(a), mapped(b)];
			const what = `${a} and ${b}`;
			if (ta !== null && ta === tb) {
				add('same_org', lang, 'right', id, `${what} (${ta})`, tier);
				if (root(a) !== root(b)) joinedTo.set(root(a), root(b));
				continue;
			}
			const why =
				ta === null
					? keyVerdict(a, null)
					: tb === null
						? keyVerdict(b, null)
						: `${a} is ${ta}, ${b} is ${tb}`;
			add('same_org', lang, 'wrong', id, `${what}: ${why}`, tier);
		}
		const keysOf = new Map();
		for (const k of output.used) {
			const t = mapped(k);
			if (t) keysOf.set(t, [...(keysOf.get(t) ?? []), k]);
		}
		for (const [t, ks] of [...keysOf].sort(([x], [y]) => x.localeCompare(y))) {
			const apart = new Set(ks.map(root)).size;
			for (let n = 1; n < apart; n++)
				add('same_org', lang, 'left_out', id, `${ks.sort().join(', ')} are all ${t}`);
		}

		const ls = label.statement;
		const os = output.statement;
		if (ls) {
			const periodGone = !possible(id, ls);
			const want = `${ls.from}..${ls.to}`;
			if (periodGone) add('statement_period', lang, 'impossible', id, want);
			else if (!os) add('statement_period', lang, 'left_out', id, want);
			else {
				const got = `${os.from}..${os.to}`;
				add(
					'statement_period',
					lang,
					got === want ? 'right' : 'wrong',
					id,
					got === want ? want : `${got}, labels ${want}`,
					os.tiers.period
				);
			}
			const wantAccounts = new Set(ls.accounts.map((a) => w.identById.get(a).value));
			const accountsGone = ls.accounts.every((a) => w.impossibleIdent.has(a));
			const got = os?.accounts.map((a) => a.value) ?? [];
			const shown = [...wantAccounts].join(' / ');
			if (!got.length)
				add('statement_account', lang, accountsGone ? 'impossible' : 'left_out', id, shown);
			else {
				const bad = got.filter((v) => !wantAccounts.has(v));
				add(
					'statement_account',
					lang,
					bad.length ? 'wrong' : 'right',
					id,
					bad.length ? `${bad.join(', ')}, labels ${shown}` : shown,
					os.tiers.accounts
				);
			}
			const issuer = label.orgs.find((m) => m.id === ls.issuer);
			const t = issuer.resolved_by_document.org;
			if (w.impossibleMention.has(issuer.id)) add('statement_issuer', lang, 'impossible', id, t);
			else if (!os?.issuer) add('statement_issuer', lang, 'left_out', id, t);
			else {
				const why = keyVerdict(os.issuer, t);
				add('statement_issuer', lang, why ? 'wrong' : 'right', id, why ?? t, os.tiers.issuer);
			}
		} else if (os) {
			add(
				'statement_period',
				lang,
				'wrong',
				id,
				`${os.from}..${os.to}: not a statement`,
				os.tiers.period
			);
			if (os.accounts.length)
				add('statement_account', lang, 'wrong', id, 'not a statement', os.tiers.accounts);
			if (os.issuer) add('statement_issuer', lang, 'wrong', id, 'not a statement', os.tiers.issuer);
		}

		const lp = label.passport;
		const op = output.passport;
		if (lp) {
			const text = pagesOf(id)
				.map((p) => p.text)
				.join('\n');
			const holderGone = !text.includes(lp.holder.surname) || !text.includes(lp.holder.given);
			const name = `${lp.holder.surname}, ${lp.holder.given}`;
			const got = op?.holder ? `${op.holder.surname}, ${op.holder.given}` : null;
			if (holderGone) add('passport_holder', lang, 'impossible', id, name);
			else if (!got) add('passport_holder', lang, 'left_out', id, name);
			else
				add(
					'passport_holder',
					lang,
					normName(got) === normName(name) ? 'right' : 'wrong',
					id,
					`${got}, labels ${name}`,
					op.tiers.holder
				);

			const number = w.identById.get(lp.number);
			if (!op?.number)
				add(
					'passport_number',
					lang,
					w.impossibleIdent.has(lp.number) ? 'impossible' : 'left_out',
					id,
					number.value
				);
			else
				add(
					'passport_number',
					lang,
					op.number.value === number.value ? 'right' : 'wrong',
					id,
					`${op.number.value}, labels ${number.value}`,
					op.tiers.number
				);

			const expiryGone = !possible(id, lp.expiry_at);
			if (expiryGone) add('passport_expiry', lang, 'impossible', id, lp.expiry);
			else if (!op?.expiry) add('passport_expiry', lang, 'left_out', id, lp.expiry);
			else
				add(
					'passport_expiry',
					lang,
					op.expiry === lp.expiry ? 'right' : 'wrong',
					id,
					`${op.expiry}, labels ${lp.expiry}`,
					op.tiers.expiry
				);

			if (expiryGone) add('passport_current', lang, 'impossible', id, String(lp.current));
			else if (op?.current === null || op?.current === undefined)
				add('passport_current', lang, 'left_out', id, String(lp.current));
			else
				add(
					'passport_current',
					lang,
					op.current === lp.current ? 'right' : 'wrong',
					id,
					`${op.current}, labels ${lp.current}`,
					op.tiers.current
				);
		} else if (op) {
			if (op.holder) add('passport_holder', lang, 'wrong', id, 'not a passport', op.tiers.holder);
			if (op.number) add('passport_number', lang, 'wrong', id, 'not a passport', op.tiers.number);
			if (op.expiry) add('passport_expiry', lang, 'wrong', id, 'not a passport', op.tiers.expiry);
			if (op.current !== null && op.current !== undefined)
				add('passport_current', lang, 'wrong', id, 'not a passport', op.tiers.current);
		}
	}

	// Answers. An account is found by any of its spellings.
	const workOf = new Map(work.map((w) => [w.id, w]));
	const latestGone = (docId) => {
		const w = workOf.get(docId);
		const s = w.label.statement;
		return !possible(docId, s) || s.accounts.every((a) => w.impossibleIdent.has(a));
	};
	const passportGone = (docId) => {
		const p = workOf.get(docId).label.passport;
		const text = pagesOf(docId)
			.map((pg) => pg.text)
			.join('\n');
		return (
			!possible(docId, p.expiry_at) ||
			!text.includes(p.holder.surname) ||
			!text.includes(p.holder.given)
		);
	};
	const answer = (measure, wanted, given, matches, gone, show) => {
		const used = new Set();
		for (const want of wanted) {
			const hits = given.filter((g) => matches(want, g));
			hits.forEach((g) => used.add(g));
			const lang = langOf(want.document);
			if (gone(want.document)) add(measure, lang, 'impossible', want.document, show(want));
			else if (!hits.length) add(measure, lang, 'left_out', want.document, show(want));
			else {
				const bad = hits.filter((g) => g.document !== want.document);
				add(
					measure,
					lang,
					bad.length ? 'wrong' : 'right',
					want.document,
					bad.length ? `${show(want)}: given ${bad.map((g) => g.document).join(', ')}` : show(want),
					'likely'
				);
			}
		}
		for (const g of given.filter((g) => !used.has(g)))
			add(
				measure,
				langOf(g.document),
				'wrong',
				g.document,
				`${show(g)}: not in the answers`,
				'likely'
			);
	};
	answer(
		'answer_latest_statement',
		labels.answers.latest_statement,
		out.answers.latest,
		(want, g) => want.spellings.includes(squash(g.account)) || want.account === squash(g.account),
		latestGone,
		(a) => a.account
	);
	answer(
		'answer_current_passport',
		labels.answers.current_passport,
		out.answers.passports,
		(want, g) => normName(want.holder) === normName(g.holder),
		passportGone,
		(a) => a.holder
	);

	const order = (m) => {
		const base = m.startsWith('identifier:') ? 'identifiers' : m;
		return MEASURES.indexOf(base);
	};
	const measures = Object.fromEntries(
		[...rows].sort(([a], [b]) => order(a) - order(b) || a.localeCompare(b))
	);
	// A card that no certain link or certain same_org row stands on is a
	// proposal: nothing on it is shown as confirmed (FORMAT.md, "Cards on likely rows only").
	const certainKeys = new Set();
	for (const d of out.docs.values()) {
		for (const l of d.links) if (l.tier === 'certain') certainKeys.add(l.org);
		for (const r of d.sameOrg) if (r.tier === 'certain') r.keys.forEach((k) => certainKeys.add(k));
	}
	const failed = merges.length > 0 || items.wrong.some((i) => i.tier === 'certain');
	return {
		extractor: out.extractor,
		directory: mode,
		text_set: out.textSet,
		failed,
		measures,
		merges,
		wrong_cards: wrongCards,
		wrong_attachments: wrongAttachments,
		splits,
		same_name: {
			one_organisation: labelsSay(true),
			two_organisations: labelsSay(false),
			pairs: sameName
		},
		likely_only_cards: keys.filter((k) => !certainKeys.has(k)).sort(),
		...items
	};
}

export function table(result) {
	const langs = [
		...new Set(Object.values(result.measures).flatMap((r) => Object.keys(r.by_lang)))
	].sort();
	const head = [
		'measure',
		'right',
		'wrong',
		'left out',
		'impossible',
		'certain r/w',
		'likely r/w',
		'likely precision',
		...langs.map((l) => `${l} r/w/l`)
	];
	const tier = (c) => (c ? `${c.right}/${c.wrong}` : '');
	const precision = (c) => (c ? `${Math.round((100 * c.right) / (c.right + c.wrong))}%` : '');
	const body = Object.entries(result.measures).map(([m, r]) => [
		m,
		...['right', 'wrong', 'left_out', 'impossible'].map((k) => String(r.total[k])),
		tier(r.by_tier.certain),
		tier(r.by_tier.likely),
		precision(r.by_tier.likely),
		...langs.map((l) => {
			const c = r.by_lang[l];
			return c ? `${c.right}/${c.wrong}/${c.left_out}` : '';
		})
	]);
	const count = (name, n) => [name, '', String(n), ...head.slice(3).map(() => '')];
	body.push(count('merges', result.merges.length));
	body.push(count('wrong cards', result.wrong_cards.length));
	body.push(count('wrong attachments', result.wrong_attachments.length));
	body.push(['splits', String(result.splits.length), ...head.slice(2).map(() => '')]);
	body.push([
		'cards on likely rows only',
		String(result.likely_only_cards.length),
		...head.slice(2).map(() => '')
	]);
	const widths = head.map((h, i) => Math.max(h.length, ...body.map((row) => row[i].length)));
	const line = (row) =>
		row.map((c, i) => (i ? c.padStart(widths[i]) : c.padEnd(widths[i]))).join('  ');
	const out = [
		`${result.extractor}, directory ${result.directory}, text set ${result.text_set}`,
		line(head),
		...body.map(line)
	];
	const merge = {
		organisations: (m) => `covers ${m.organisations.join(', ')}`,
		'name-and-code': () => 'is an institution code and holds a printed name'
	};
	for (const m of result.merges) out.push(`MERGE: key ${m.key} ${merge[m.why](m)}`);
	for (const c of result.wrong_cards)
		out.push(`wrong card: key ${c.key} is ${c.organisation} but claims ${c.claims.join(' and ')}`);
	for (const a of result.wrong_attachments)
		out.push(
			`wrong attachment: ${a.document} on key ${a.key} (${a.organisation}) by ${a.mention}, a name the labels exclude for it (${a.why})`
		);
	for (const s of result.splits)
		out.push(`split: ${s.organisation} over keys ${s.keys.join(', ')}`);
	const { one_organisation: one, two_organisations: two, pairs } = result.same_name;
	out.push(
		`same name in two documents: ${pairs.length} pairs of keys, the labels say ${one} are one organisation and ${two} are two`
	);
	for (const { keys: k, organisations: o } of pairs)
		if (o[0] !== null && o[1] !== null && o[0] !== o[1])
			out.push(`same name, two organisations: ${k[0]} (${o[0]}) and ${k[1]} (${o[1]})`);
	const heading = {
		wrong: (i) => (i.tier === 'certain' ? 'WRONG CERTAIN' : i.tier ? `wrong ${i.tier}` : 'wrong'),
		impossible: () => 'impossible',
		left_out: () => 'left out'
	};
	for (const kind of ['wrong', 'impossible', 'left_out'])
		for (const i of result[kind])
			out.push(`${heading[kind](i)}: ${i.measure} ${i.document ?? ''} ${i.what}`);
	out.push(
		result.failed
			? 'FAIL: a certain row is wrong or a key merges'
			: 'pass: no certain row is wrong and no key merges'
	);
	return out.join('\n');
}

if (isMain(import.meta.url)) {
	const args = process.argv.slice(2);
	const at = args.indexOf('--json');
	const jsonPath = at >= 0 ? args.splice(at, 2)[1] : null;
	if (args.length !== 1) {
		console.error('usage: node sample/tools/score.mjs <output.json> [--json <path>]');
		process.exit(2);
	}
	const path = resolve(args[0]);
	try {
		const output = JSON.parse(readFileSync(path, 'utf8'));
		const textSet = readTextSet(TEXT_SETS.includes(output?.text_set) ? output.text_set : 'text');
		const result = score(
			readLabels(new URL('labels.json', SAMPLE)),
			textSet,
			output,
			JSON.parse(readFileSync(new URL('directory/banks.json', SAMPLE), 'utf8'))
		);
		console.log(table(result));
		writeFileSync(
			jsonPath ?? path.replace(/\.json$/, '') + '.score.json',
			JSON.stringify(result, null, '\t') + '\n'
		);
		process.exit(result.failed ? 1 : 0);
	} catch (e) {
		if (!(e instanceof FormatError) && !(e instanceof SyntaxError)) throw e;
		console.error(`${args[0]}: ${e.message}`);
		process.exit(2);
	}
}
