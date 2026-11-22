// Scores one extractor output against labels.json, as sample/FORMAT.md says:
//   node sample/tools/score.mjs <output.json> [--json <path>]
// Prints a table and writes the score as JSON (default: <output>.score.json).
// Exits 1 when any link, merge, card or answer is wrong, 2 when the output
// breaks the format. Leaving something out never fails.
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { SAMPLE } from './generate.mjs';
import { KINDS } from './lib/ids.mjs';
import { locate, readsBack } from './lib/find.mjs';
import { readLabels } from './lib/labels.mjs';
import { isMain } from './lib/main.mjs';
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
const DATE = /^\d{4}-\d\d-\d\d$/;
// Measures whose wrong count fails the run, beside wrong merges.
const FAILING = new Set(['links', 'answer_latest_statement', 'answer_current_passport']);
const MEASURES = [
	'mentions',
	'org_directory',
	'identifiers',
	'links',
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
const spanKey = (s) => `${s.page}:${s.start}:${s.end}`;

// Checks everything FORMAT.md says a well-formed output is, and returns it
// with every reference resolved: keys to cards, identifier ids to entries.
// Scoring trusts this structure and reads nothing raw.
function parseOutput(output, textSet) {
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
	if (output.format !== 1) fail('format', 'must be 1');
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
		if (!key) fail(where, 'a key must be a non-empty string');
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
			identifiers.set(id.id, { id: id.id, ...at, kind: id.kind, value: id.value, valid: id.valid });
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
			return { identifier, org, reasons: link.reasons, via: optionalIdent(link.via, w) };
		});
		let statement = null;
		if (doc.statement !== undefined && doc.statement !== null) {
			const w = `${where}.statement`;
			const s = doc.statement;
			if (!isObject(s)) fail(w, 'must be an object');
			if (!isDate(s.from) || !isDate(s.to)) fail(w, 'from and to must be dates, YYYY-MM-DD');
			const accounts = list(s.accounts, `${w}.accounts`).map((id) => ident(id, w));
			if (s.issuer === undefined) fail(w, 'issuer must be a key or null');
			statement = { from: s.from, to: s.to, accounts, issuer: orgKey(s.issuer, w, true) };
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
			passport = { holder, number, mrz, issued: p.issued ?? null, expiry, expiryAt, current };
		}
		docs.set(doc.id, {
			orgs: mentions,
			identifiers: [...identifiers.values()],
			links,
			statement,
			passport
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
	const add = (measure, lang, outcome, document, what) => {
		if (!rows.has(measure)) rows.set(measure, { total: zero(), by_lang: {} });
		const row = rows.get(measure);
		row.total[outcome]++;
		if (lang) (row.by_lang[lang] ??= zero())[outcome]++;
		if (outcome !== 'right') items[outcome].push({ measure, document, what });
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
	const out = parseOutput(rawOutput, textSet);
	const mode = out.directory;
	const byValue = out.textSet !== 'text';
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
		names.get(t).add(normName(name));
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
		names.get(t)?.has(normName(name)) ||
		(entryNames.get(directoryIdOf(t)) ?? []).some((n) => normName(n) === normName(name));

	const pagesOf = (id) => textSet.docs.get(id).pages;
	const possible = (docId, s, value) =>
		locate(pagesOf(docId)[s.page - 1].text, s.text, value) !== null;
	const mentionKey = byValue ? (m) => `${m.page}:${m.text}` : spanKey;
	const identKey = byValue ? (i) => `${i.page}:${squash(i.value)}` : spanKey;

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
	const empty = { orgs: [], identifiers: [], links: [], statement: null, passport: null };
	const docIds = [...textSet.docs.keys()];
	const work = docIds.map((id) => {
		const label = labelled(id);
		const output = out.docs.get(id) ?? empty;
		const identById = new Map(label.identifiers.map((i) => [i.id, i]));
		const impossibleIdent = new Set(
			label.identifiers.filter((i) => !possible(id, i, i.value)).map((i) => i.id)
		);
		const impossibleMention = new Set(label.orgs.filter((m) => !possible(id, m)).map((m) => m.id));
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
	const merges = keys
		.filter((k) => targetsOf(k).size > 1)
		.map((k) => ({ key: k, organisations: [...targetsOf(k)].sort() }));
	const merged = new Set(merges.map((m) => m.key));
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
			const notOrg = label.not_orgs.find((n) => mentionKey(n) === mentionKey(o));
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
					diff.length ? `${what}: given ${diff.map((f) => `${f} ${o[f]}`).join(', ')}` : what
				);
			}
		}
		for (const o of w.idents.unmatched) {
			const decoy = label.not_identifiers.find(
				(n) => (byValue ? `${n.page}:${squash(n.text)}` : spanKey(n)) === identKey(o)
			);
			add(
				`identifier:${o.kind}`,
				lang,
				'wrong',
				id,
				`${o.kind} ${o.value}: ${decoy ? `a decoy (${decoy.why})` : 'not a labelled identifier'}`
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
			const k = [
				ident ? 'labelled' : 'unlabelled',
				shown.kind,
				shown.value,
				t ?? `key:${link.org}`
			].join('\0');
			if (!groups.has(k))
				groups.set(k, { ident, shown, org: link.org, t, reasons: new Set(), vias: [] });
			const g = groups.get(k);
			for (const r of link.reasons) g.reasons.add(r);
			const via = link.via && w.labelOf.get(link.via);
			if (via) g.vias.push(via);
		}
		const groupOrder = (a, b) =>
			byPosition(a.shown, b.shown) || (a.t ?? a.org).localeCompare(b.t ?? b.org);
		for (const g of [...groups.values()].sort(groupOrder)) {
			const what = `${g.shown.kind} ${g.shown.value} at ${g.org}`;
			const reasons = [...g.reasons].sort();
			if (g.t === null) {
				add('links', lang, 'wrong', id, `${what}: ${keyVerdict(g.org, null)}`);
				continue;
			}
			if (!g.ident) {
				add('links', lang, 'wrong', id, `${what} (${g.t}): no such identifier in the labels`);
				continue;
			}
			const right = labelLinks.find((l) => l.org === g.t && same(l.ident, g.ident));
			if (right) {
				const via = right.via && w.identById.get(right.via);
				const holds = reasons.filter(
					(r) =>
						right.reasons.includes(r) &&
						(r !== 'paired-code' || (via && g.vias.some((v) => same(v, via))))
				);
				right.found ||= holds.length > 0;
				add(
					'links',
					lang,
					holds.length ? 'right' : 'wrong',
					id,
					holds.length
						? `${what} (${g.t})`
						: `${what} (${g.t}): no reason given holds (${reasons.join(', ')})`
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
			add('links', lang, 'wrong', id, `${what} (${g.t}): ${why}`);
		}
		for (const l of labelLinks) {
			if (l.found) continue;
			const what = `${l.ident.kind} ${l.ident.value} at ${l.org}`;
			const blocked =
				w.impossibleIdent.has(l.identifier) ||
				l.reasons.every((r) => r === 'paired-code' && w.impossibleIdent.has(l.via));
			add('links', lang, blocked ? 'impossible' : 'left_out', id, what);
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
					got === want ? want : `${got}, labels ${want}`
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
					bad.length ? `${bad.join(', ')}, labels ${shown}` : shown
				);
			}
			const issuer = label.orgs.find((m) => m.id === ls.issuer);
			const t = issuer.resolved_by_document.org;
			if (w.impossibleMention.has(issuer.id)) add('statement_issuer', lang, 'impossible', id, t);
			else if (!os?.issuer) add('statement_issuer', lang, 'left_out', id, t);
			else {
				const why = keyVerdict(os.issuer, t);
				add('statement_issuer', lang, why ? 'wrong' : 'right', id, why ?? t);
			}
		} else if (os) {
			add('statement_period', lang, 'wrong', id, `${os.from}..${os.to}: not a statement`);
			if (os.accounts.length) add('statement_account', lang, 'wrong', id, 'not a statement');
			if (os.issuer) add('statement_issuer', lang, 'wrong', id, 'not a statement');
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
					`${got}, labels ${name}`
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
					`${op.number.value}, labels ${number.value}`
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
					`${op.expiry}, labels ${lp.expiry}`
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
					`${op.current}, labels ${lp.current}`
				);
		} else if (op) {
			if (op.holder) add('passport_holder', lang, 'wrong', id, 'not a passport');
			if (op.number) add('passport_number', lang, 'wrong', id, 'not a passport');
			if (op.expiry) add('passport_expiry', lang, 'wrong', id, 'not a passport');
			if (op.current !== null && op.current !== undefined)
				add('passport_current', lang, 'wrong', id, 'not a passport');
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
					bad.length ? `${show(want)}: given ${bad.map((g) => g.document).join(', ')}` : show(want)
				);
			}
		}
		for (const g of given.filter((g) => !used.has(g)))
			add(measure, langOf(g.document), 'wrong', g.document, `${show(g)}: not in the answers`);
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
	const failed =
		merges.length > 0 ||
		wrongCards.length > 0 ||
		[...rows].some(([m, r]) => FAILING.has(m) && r.total.wrong > 0);
	return {
		extractor: out.extractor,
		directory: mode,
		text_set: out.textSet,
		failed,
		measures,
		merges,
		wrong_cards: wrongCards,
		splits,
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
		...langs.map((l) => `${l} r/w/l`)
	];
	const body = Object.entries(result.measures).map(([m, r]) => [
		m,
		...['right', 'wrong', 'left_out', 'impossible'].map((k) => String(r.total[k])),
		...langs.map((l) => {
			const c = r.by_lang[l];
			return c ? `${c.right}/${c.wrong}/${c.left_out}` : '';
		})
	]);
	body.push(['wrong merges', '', String(result.merges.length), '', '', ...langs.map(() => '')]);
	body.push(['wrong cards', '', String(result.wrong_cards.length), '', '', ...langs.map(() => '')]);
	body.push(['splits', String(result.splits.length), '', '', '', ...langs.map(() => '')]);
	const widths = head.map((h, i) => Math.max(h.length, ...body.map((row) => row[i].length)));
	const line = (row) =>
		row.map((c, i) => (i ? c.padStart(widths[i]) : c.padEnd(widths[i]))).join('  ');
	const out = [
		`${result.extractor}, directory ${result.directory}, text set ${result.text_set}`,
		line(head),
		...body.map(line)
	];
	for (const m of result.merges)
		out.push(`WRONG MERGE: key ${m.key} covers ${m.organisations.join(', ')}`);
	for (const c of result.wrong_cards)
		out.push(`WRONG CARD: key ${c.key} is ${c.organisation} but claims ${c.claims.join(' and ')}`);
	for (const s of result.splits)
		out.push(`split: ${s.organisation} over keys ${s.keys.join(', ')}`);
	for (const kind of ['wrong', 'impossible', 'left_out'])
		for (const i of result[kind])
			out.push(`${kind.replace('_', ' ')}: ${i.measure} ${i.document ?? ''} ${i.what}`);
	out.push(
		result.failed
			? 'FAIL: a link, a merge, a card or an answer is wrong'
			: 'pass: no wrong link, merge, card or answer'
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
		const textSet = readTextSet(TEXT_SETS.includes(output.text_set) ? output.text_set : 'text');
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
