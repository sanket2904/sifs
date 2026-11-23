// Turns labels.json into the output a perfect extractor would write, so the
// scorer can be shown to give full marks:
//   node sample/tools/reference.mjs <on|off> <text|text-ocr> <output.json>
// On the OCR text set, whatever the OCR text does not hold is left out, since
// no extractor could find it there.
import { readFileSync, writeFileSync } from 'node:fs';
import { SAMPLE } from './generate.mjs';
import { locate, readsBack } from './lib/find.mjs';
import { matchKey } from './lib/key.mjs';
import { readLabels } from './lib/labels.mjs';
import { isMain } from './lib/main.mjs';
import {
	CODE_REASONS,
	codeKey,
	documentKey,
	mayBeCertainIdentifier,
	mayBeCertainJoin,
	mayBeCertainLink,
	mayBeCertainPassport
} from './lib/tiers.mjs';
import { readTextSet, TEXT_SETS } from './text.mjs';

// Keys of its own, so a pass proves the scorer maps keys, not label ids. In
// each document, a printed name's organisation has a document key spelt from
// the first name the document prints for it, such as
// name:hdfc-statement-sep:hdfcbank; an institution code has its own canonical
// key, such as IFSC HDFC, and same_org rows join the two.
const squash = (v) => v.normalize('NFKC').toUpperCase().replace(/\s/gu, '');

export function reference(labels, textSet, directory, banks) {
	const entries = new Map(banks.entries.map((e) => [e.id, e]));
	const organisations = {};
	// The document's key for each organisation it names, by local id.
	let nameKeys;
	const declare = (document, target, name) => {
		if (!nameKeys.has(target))
			nameKeys.set(
				target,
				documentKey(document, matchKey(name ?? labels.organisations[target].name))
			);
		const key = nameKeys.get(target);
		if (target.startsWith('directory:'))
			organisations[key] ??= { name, directory: directory === 'on' ? target.slice(10) : null };
		else {
			const org = labels.organisations[target];
			organisations[key] ??= {
				name: org.name,
				directory: directory === 'on' ? org.directory : null
			};
		}
		return key;
	};
	// The key of an institution code. With the directory on, its card is the
	// directory entry that holds the code; a code is never a printed name.
	const codeKeyOf = new Map();
	const declareCode = (key, target) => {
		organisations[key] ??= {
			name: null,
			directory: directory === 'on' ? labels.organisations[target].directory : null
		};
		codeKeyOf.set(key, target);
		return key;
	};
	const dirToLocal = new Map(
		Object.entries(labels.organisations)
			.filter(([, o]) => o.directory)
			.map(([id, o]) => [o.directory, id])
	);
	const documents = [];
	for (const label of labels.documents) {
		if (label.nothing_to_find) continue;
		nameKeys = new Map();
		const pages = textSet.docs.get(label.id).pages;
		// The span of a labelled text in this text set, or null when it is not there.
		const find = (s, value) => {
			const text = pages[s.page - 1].text;
			if (readsBack(text, s)) return { page: s.page, start: s.start, end: s.end, text: s.text };
			const at = locate(text, s.text, value);
			return at && { page: s.page, ...at };
		};
		const orgs = [];
		const mentionTarget = new Map();
		for (const m of label.orgs) {
			const at = find(m);
			if (!at) continue;
			let target = m.resolved_by_document?.org ?? null;
			let settled = m.resolved_by_document?.evidence[0].by ?? null;
			if (!target && directory === 'on' && m.directory.lookup === 'exact') {
				target = dirToLocal.get(m.directory.id) ?? `directory:${m.directory.id}`;
				settled = 'directory';
			}
			const org = target && declare(label.id, target, m.text);
			mentionTarget.set(m.id, org);
			orgs.push({ ...at, org, settled_by: settled });
		}
		const identifiers = [];
		const kept = new Set();
		for (const i of label.identifiers) {
			const at = find(i, i.value);
			if (!at) continue;
			kept.add(i.id);
			const tier = mayBeCertainIdentifier(i, directory, entries) ? 'certain' : 'likely';
			identifiers.push({ id: i.id, ...at, kind: i.kind, value: i.value, valid: i.valid, tier });
		}
		const byId = new Map(identifiers.map((i) => [i.id, i]));
		// A code's own reason puts the link on the code's key, and paired-code
		// on its via's key. Every other link is held at the printed name.
		const keyFor = (l, reasons) => {
			const ident = byId.get(l.identifier);
			if (reasons.some((r) => CODE_REASONS.get(r) === ident.kind))
				return declareCode(codeKey(ident), l.org);
			if (reasons.includes('paired-code')) return declareCode(codeKey(byId.get(l.via)), l.org);
			return declare(label.id, l.org);
		};
		const links = label.links
			.filter((l) => kept.has(l.identifier))
			.map((l) => ({ l, reasons: l.reasons.filter((r) => r !== 'paired-code' || kept.has(l.via)) }))
			.filter(({ reasons }) => reasons.length)
			.map(({ l, reasons }) => {
				const org = keyFor(l, reasons);
				const test = { identifier: byId.get(l.identifier), reasons, card: organisations[org] };
				return {
					identifier: l.identifier,
					org,
					reasons,
					...(reasons.includes('paired-code') ? { via: l.via } : {}),
					tier: mayBeCertainLink(test, directory, entries) ? 'certain' : 'likely'
				};
			});
		const doc = { id: label.id, orgs, identifiers, links };
		if (label.statement) {
			const s = label.statement;
			const accounts = s.accounts.filter((a) => kept.has(a));
			const issuer = mentionTarget.get(s.issuer) ?? null;
			doc.statement = {
				from: s.from,
				to: s.to,
				accounts,
				issuer,
				tiers: {
					period: 'likely',
					...(accounts.length ? { accounts: 'likely' } : {}),
					...(issuer ? { issuer: 'likely' } : {})
				}
			};
		}
		if (label.passport) {
			const p = label.passport;
			const seen = find(p.expiry_at);
			const mrz = kept.has(p.mrz) ? p.mrz : null;
			doc.passport = {
				holder: p.holder,
				number: kept.has(p.number) ? p.number : null,
				mrz,
				issued: p.issued,
				expiry: seen ? p.expiry : null,
				...(p.expiry_candidates ? { expiry_candidates: p.expiry_candidates } : {}),
				expiry_at: seen,
				current: seen ? p.current : null
			};
			const zone = {
				mrz: byId.get(mrz),
				number: byId.get(doc.passport.number),
				holder: doc.passport.holder,
				expiry: doc.passport.expiry,
				expiryAt: seen
			};
			doc.passport.tiers = Object.fromEntries(
				['holder', 'number', 'expiry', 'current']
					.filter((f) => doc.passport[f] !== null)
					.map((f) => [f, mayBeCertainPassport(f, zone) ? 'certain' : 'likely'])
			);
		}
		doc.same_org = sameOrg(label, doc);
		documents.push(doc);
	}
	// Joins each organisation's code keys in a document to its printed name's
	// key there, or to each other when the document prints no name of it.
	function sameOrg(label, doc) {
		const used = new Set([
			...doc.orgs.map((m) => m.org),
			...doc.links.map((l) => l.org),
			doc.statement?.issuer
		]);
		const rows = [];
		for (const target of new Set(doc.links.map((l) => codeKeyOf.get(l.org)).filter(Boolean))) {
			const codes = [...used].filter((k) => codeKeyOf.get(k) === target).sort();
			const name = used.has(nameKeys.get(target)) ? nameKeys.get(target) : null;
			const [first, ...rest] = name ? [name, ...codes] : codes;
			for (const other of rest) {
				const row = joinRow(label, doc, target, first, other);
				if (row) rows.push(row);
			}
		}
		return rows;
	}
	// The row joining keys a and b of one organisation, or null when nothing
	// in the document or the directory joins them.
	function joinRow(label, doc, target, a, b) {
		const mentions = label.orgs.filter((m) => m.resolved_by_document?.org === target);
		const join = {
			reason: 'directory',
			names: doc.orgs.filter((m) => m.org === a).map((m) => m.text),
			codes: doc.links
				.map((l) => ({ l, ident: byIdOf(doc, l.identifier) }))
				.filter(
					({ l, ident }) => l.org === b && l.reasons.some((r) => CODE_REASONS.get(r) === ident.kind)
				)
				.map(({ ident }) => ident)
		};
		if (mayBeCertainJoin(join, directory, entries))
			return { keys: [a, b], reason: 'directory', tier: 'certain' };
		const codes = doc.links
			.filter((l) => [a, b].includes(l.org))
			.map((l) => byIdOf(doc, l.identifier));
		const letters = codes
			.filter((i) => ['ifsc', 'bic'].includes(i.kind))
			.map((i) => i.value.slice(0, 4));
		const line = (s) => lineOf(doc.id, s);
		const reason = mentions.some((m) => letters.some((c) => squash(m.text).startsWith(c)))
			? 'spells-code'
			: label.statement && mentions.some((m) => m.id === label.statement.issuer)
				? 'statement-header'
				: mentions.some((m) =>
							codes.some((i) => i.page === m.page && Math.abs(line(i) - line(m)) <= 1)
					  )
					? 'printed-together'
					: null;
		return reason && { keys: [a, b], reason, tier: 'likely' };
	}
	function byIdOf(doc, id) {
		return doc.identifiers.find((i) => i.id === id);
	}
	function lineOf(docId, s) {
		const text = textSet.docs.get(docId).pages[s.page - 1].text;
		return [...text].slice(0, s.start).filter((c) => c === '\n').length;
	}
	return {
		format: 3,
		extractor: 'reference (labels.json)',
		directory,
		text_set: textSet.index.text_set,
		organisations,
		documents,
		answers: {
			latest_statement: labels.answers.latest_statement.map(({ account, document }) => ({
				account,
				document
			})),
			current_passport: labels.answers.current_passport.map(({ holder, document }) => ({
				holder,
				document
			}))
		}
	};
}

if (isMain(import.meta.url)) {
	const [directory, set, path] = process.argv.slice(2);
	if (!['on', 'off'].includes(directory) || !TEXT_SETS.includes(set) || !path) {
		console.error('usage: node sample/tools/reference.mjs <on|off> <text|text-ocr> <output.json>');
		process.exit(2);
	}
	const output = reference(
		readLabels(new URL('labels.json', SAMPLE)),
		readTextSet(set),
		directory,
		JSON.parse(readFileSync(new URL('directory/banks.json', SAMPLE), 'utf8'))
	);
	writeFileSync(path, JSON.stringify(output, null, '\t') + '\n');
}
