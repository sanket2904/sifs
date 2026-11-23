// Turns labels.json into the output a perfect extractor would write, so the
// scorer can be shown to give full marks:
//   node sample/tools/reference.mjs <on|off> <text|text-ocr> <output.json>
// On the OCR text set, whatever the OCR text does not hold is left out, since
// no extractor could find it there.
import { writeFileSync } from 'node:fs';
import { SAMPLE } from './generate.mjs';
import { locate, readsBack } from './lib/find.mjs';
import { readLabels } from './lib/labels.mjs';
import { isMain } from './lib/main.mjs';
import { readTextSet, TEXT_SETS } from './text.mjs';

// Keys of its own, so a pass proves the scorer maps keys, not label ids.
const keyOf = (target) => `ref:${target}`;

export function reference(labels, textSet, directory) {
	const organisations = {};
	const declare = (target, name) => {
		const key = keyOf(target);
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
	const dirToLocal = new Map(
		Object.entries(labels.organisations)
			.filter(([, o]) => o.directory)
			.map(([id, o]) => [o.directory, id])
	);
	const documents = [];
	for (const label of labels.documents) {
		if (label.nothing_to_find) continue;
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
			const org = target && declare(target, m.text);
			mentionTarget.set(m.id, org);
			orgs.push({ ...at, org, settled_by: settled });
		}
		const identifiers = [];
		const kept = new Set();
		for (const i of label.identifiers) {
			const at = find(i, i.value);
			if (!at) continue;
			kept.add(i.id);
			identifiers.push({ id: i.id, ...at, kind: i.kind, value: i.value, valid: i.valid });
		}
		const links = label.links
			.filter((l) => kept.has(l.identifier))
			.map((l) => {
				const reasons = l.reasons.filter((r) => r !== 'paired-code' || kept.has(l.via));
				return {
					identifier: l.identifier,
					org: declare(l.org),
					reasons,
					...(reasons.includes('paired-code') ? { via: l.via } : {})
				};
			})
			.filter((l) => l.reasons.length);
		const doc = { id: label.id, orgs, identifiers, links };
		if (label.statement) {
			const s = label.statement;
			doc.statement = {
				from: s.from,
				to: s.to,
				accounts: s.accounts.filter((a) => kept.has(a)),
				issuer: mentionTarget.get(s.issuer) ?? null
			};
		}
		if (label.passport) {
			const p = label.passport;
			const seen = find(p.expiry_at);
			doc.passport = {
				holder: p.holder,
				number: kept.has(p.number) ? p.number : null,
				mrz: kept.has(p.mrz) ? p.mrz : null,
				issued: p.issued,
				expiry: seen ? p.expiry : null,
				...(p.expiry_candidates ? { expiry_candidates: p.expiry_candidates } : {}),
				expiry_at: seen,
				current: seen ? p.current : null
			};
		}
		documents.push(doc);
	}
	return {
		format: 1,
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
	const output = reference(readLabels(new URL('labels.json', SAMPLE)), readTextSet(set), directory);
	writeFileSync(path, JSON.stringify(output, null, '\t') + '\n');
}
