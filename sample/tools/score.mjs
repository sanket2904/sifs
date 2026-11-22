// Scores one extractor output against labels.json, as sample/FORMAT.md says:
//   node sample/tools/score.mjs <output.json> [--json <path>]
// Prints a table and writes the score as JSON (default: <output>.score.json).
// Exits 1 when any link, merge or answer is wrong, 2 when the output breaks
// the format. Leaving something out never fails.
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { SAMPLE } from './generate.mjs';
import { KINDS } from './lib/ids.mjs';
import { locate, readsBack } from './lib/find.mjs';
import { readLabels } from './lib/labels.mjs';
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
	for (const [key, org] of Object.entries(output.organisations)) {
		const where = `organisations.${key}`;
		if (!isObject(org) || typeof org.name !== 'string') fail(where, 'needs a name');
		if (org.directory !== null && typeof org.directory !== 'string')
			fail(where, 'directory must be a directory id or null');
	}
	const orgKey = (key, where) => {
		if (key !== null && !Object.hasOwn(output.organisations, key))
			fail(where, `${JSON.stringify(key)} is not in organisations`);
	};
	const span = (s, pages, where) => {
		if (!isObject(s)) fail(where, 'must be a span');
		if (!Number.isInteger(s.page) || s.page < 1 || s.page > pages.length)
			fail(where, `page ${s.page} is not a page of the document`);
		if (!Number.isInteger(s.start) || !Number.isInteger(s.end) || s.start > s.end)
			fail(where, 'start and end must be whole numbers, start <= end');
		if (typeof s.text !== 'string' || !readsBack(pages[s.page - 1].text, s))
			fail(where, `${JSON.stringify(s.text)} is not the text at ${s.start}..${s.end}`);
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
		const orgs = list(doc.orgs, `${where}.orgs`);
		orgs.forEach((m, i) => {
			const w = `${where}.orgs[${i}]`;
			span(m, pages, w);
			if (m.org === undefined) fail(w, 'org must be a key or null');
			orgKey(m.org, w);
			if (m.settled_by !== undefined && m.settled_by !== null && !SETTLED_BY.has(m.settled_by))
				fail(w, `settled_by must be one of ${[...SETTLED_BY].join(', ')}`);
		});
		const identifiers = new Map();
		list(doc.identifiers, `${where}.identifiers`).forEach((id, i) => {
			const w = `${where}.identifiers[${i}]`;
			span(id, pages, w);
			if (typeof id.id !== 'string' || identifiers.has(id.id)) fail(w, 'id must be unique');
			if (!Object.hasOwn(KINDS, id.kind)) fail(w, `kind ${JSON.stringify(id.kind)} is not known`);
			if (typeof id.value !== 'string') fail(w, 'value must be text');
			if (![true, false, null].includes(id.valid)) fail(w, 'valid must be true, false or null');
			identifiers.set(id.id, id);
		});
		const ident = (id, w) => {
			if (!identifiers.has(id))
				fail(w, `${JSON.stringify(id)} is not an identifier of the document`);
			return identifiers.get(id);
		};
		const links = list(doc.links, `${where}.links`).map((link, i) => {
			const w = `${where}.links[${i}]`;
			if (!isObject(link)) fail(w, 'must be an object');
			const identifier = ident(link.identifier, w);
			if (link.org === null || link.org === undefined) fail(w, 'org must be a key');
			orgKey(link.org, w);
			if (!Array.isArray(link.reasons) || !link.reasons.length) fail(w, 'needs a reason');
			for (const r of link.reasons)
				if (!REASONS.has(r)) fail(w, `reason ${JSON.stringify(r)} is not known`);
			const via = link.via === undefined || link.via === null ? null : ident(link.via, w);
			return { identifier, org: link.org, reasons: link.reasons, via };
		});
		let statement = null;
		if (doc.statement !== undefined && doc.statement !== null) {
			const w = `${where}.statement`;
			const s = doc.statement;
			if (!isObject(s)) fail(w, 'must be an object');
			if (!DATE.test(s.from) || !DATE.test(s.to)) fail(w, 'from and to must be YYYY-MM-DD');
			const accounts = list(s.accounts, `${w}.accounts`).map((id) => ident(id, w));
			if (s.issuer === undefined) fail(w, 'issuer must be a key or null');
			orgKey(s.issuer, w);
			statement = { from: s.from, to: s.to, accounts, issuer: s.issuer };
		}
		let passport = null;
		if (doc.passport !== undefined && doc.passport !== null) {
			const w = `${where}.passport`;
			const p = doc.passport;
			if (!isObject(p)) fail(w, 'must be an object');
			const holder = p.holder;
			if (
				holder !== null &&
				(!isObject(holder) ||
					typeof holder.surname !== 'string' ||
					typeof holder.given !== 'string')
			)
				fail(w, 'holder must be {surname, given} or null');
			const number = p.number === null || p.number === undefined ? null : ident(p.number, w);
			if (p.expiry !== null && p.expiry !== 'cannot-tell' && !DATE.test(p.expiry))
				fail(w, 'expiry must be YYYY-MM-DD, "cannot-tell" or null');
			if (![true, false, null].includes(p.current)) fail(w, 'current must be true, false or null');
			passport = { holder, number, expiry: p.expiry, current: p.current };
		}
		docs.set(doc.id, { orgs, identifiers: [...identifiers.values()], links, statement, passport });
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
		organisations: output.organisations,
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

// Pairs labelled items with output items that have the same key, in order of
// occurrence, so a text that appears twice pairs first with first.
function pair(labelled, output, keyOf) {
	const waiting = new Map();
	for (const o of output) {
		const k = keyOf(o, 'output');
		if (!waiting.has(k)) waiting.set(k, []);
		waiting.get(k).push(o);
	}
	const partner = new Map();
	for (const l of labelled) {
		const queue = waiting.get(keyOf(l, 'label'));
		if (queue?.length) partner.set(l, queue.shift());
	}
	const used = new Set(partner.values());
	return { partner, unmatched: output.filter((o) => !used.has(o)) };
}

const spanKey = (s) => `${s.page}:${s.start}:${s.end}`;
const squash = (v) => v.replace(/\s/gu, '');
const normName = (v) =>
	v
		.normalize('NFKC')
		.toUpperCase()
		.replace(/\s+/gu, ' ')
		.replace(/\s*,\s*/u, ', ')
		.trim();

export function score(labels, textSet, rawOutput) {
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
		return {
			id,
			lang: langOf(id),
			label,
			output,
			identById,
			impossibleIdent,
			impossibleMention,
			mentions: pair(label.orgs, output.orgs, mentionKey),
			idents: pair(label.identifiers, output.identifiers, identKey)
		};
	});

	// Organisation keys. A key maps to the labelled organisations its
	// mentions land on; a key with no mention evidence, to the organisations
	// the labels link its linked identifiers to.
	const evidence = new Map();
	const note = (key, t) => {
		if (!evidence.has(key)) evidence.set(key, new Set());
		if (t) evidence.get(key).add(t);
	};
	for (const w of work)
		for (const [l, o] of w.mentions.partner) if (o.org && target(l)) note(o.org, target(l));
	for (const w of work)
		for (const link of w.output.links) {
			if (evidence.get(link.org)?.size) continue;
			for (const l of w.label.links) {
				const li = w.identById.get(l.identifier);
				if (li.kind === link.identifier.kind && li.value === link.identifier.value)
					note(`link:${link.org}`, l.org);
			}
		}
	const targetsOf = (key) =>
		evidence.get(key)?.size ? evidence.get(key) : (evidence.get(`link:${key}`) ?? new Set());
	const keys = Object.keys(out.organisations);
	const merges = keys
		.filter((k) => targetsOf(k).size > 1)
		.map((k) => ({ key: k, organisations: [...targetsOf(k)].sort() }));
	const merged = new Set(merges.map((m) => m.key));
	const mapped = (key) =>
		key && !merged.has(key) && targetsOf(key).size === 1 ? [...targetsOf(key)][0] : null;
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
		const got = mapped(key);
		if (got === null) return `key ${key} lands on no labelled organisation`;
		return got === want ? null : `key ${key} is ${got}, not ${want}`;
	};

	for (const k of keys) {
		const t = mapped(k);
		if (t === null) continue;
		const want =
			mode === 'off'
				? null
				: t.startsWith('directory:')
					? t.slice(10)
					: (labels.organisations[t]?.directory ?? null);
		const got = out.organisations[k].directory ?? null;
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
				const diff = ['kind', 'value', 'valid'].filter((f) => o[f] !== l[f]);
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

		// Links, by identifier kind and value and the organisation.
		const same = (li, oi) => li.kind === oi.kind && li.value === oi.value;
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
		const done = new Set();
		for (const link of output.links) {
			const oi = link.identifier;
			const what = `${oi.kind} ${oi.value} at ${link.org}`;
			const t = mapped(link.org);
			const seenAs = `${oi.kind}\0${oi.value}\0${t ?? `key:${link.org}`}`;
			if (done.has(seenAs)) continue;
			done.add(seenAs);
			if (t === null) {
				add('links', lang, 'wrong', id, `${what}: ${keyVerdict(link.org, null)}`);
				continue;
			}
			const right = labelLinks.find((l) => l.org === t && same(l.ident, oi));
			if (right) {
				const holds = link.reasons.filter(
					(r) =>
						right.reasons.includes(r) &&
						(r !== 'paired-code' || (link.via && same(w.identById.get(right.via), link.via)))
				);
				right.found ||= holds.length > 0;
				add(
					'links',
					lang,
					holds.length ? 'right' : 'wrong',
					id,
					holds.length
						? `${what} (${t})`
						: `${what} (${t}): no reason given holds (${link.reasons.join(', ')})`
				);
				continue;
			}
			const li = label.identifiers.find((i) => same(i, oi));
			const notLink =
				li && label.not_links.find((n) => n.org === t && same(w.identById.get(n.identifier), oi));
			const why = notLink
				? `a not_link (${notLink.why})`
				: !li
					? 'not a labelled identifier'
					: li.valid === false
						? 'the identifier is invalid'
						: !inDocument.has(t)
							? `the document does not settle ${t}`
							: 'nothing ties them';
			add('links', lang, 'wrong', id, `${what} (${t}): ${why}`);
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
		merges.length > 0 || [...rows].some(([m, r]) => FAILING.has(m) && r.total.wrong > 0);
	return {
		extractor: out.extractor,
		directory: mode,
		text_set: out.textSet,
		failed,
		measures,
		merges,
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
	for (const s of result.splits)
		out.push(`split: ${s.organisation} over keys ${s.keys.join(', ')}`);
	for (const kind of ['wrong', 'impossible', 'left_out'])
		for (const i of result[kind])
			out.push(`${kind.replace('_', ' ')}: ${i.measure} ${i.document ?? ''} ${i.what}`);
	out.push(
		result.failed
			? 'FAIL: a link, a merge or an answer is wrong'
			: 'pass: no wrong link, merge or answer'
	);
	return out.join('\n');
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
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
		const result = score(readLabels(new URL('labels.json', SAMPLE)), textSet, output);
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
