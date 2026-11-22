import { readFileSync, writeFileSync } from 'node:fs';

export const LISTS = ['orgs', 'not_orgs', 'identifiers', 'not_identifiers', 'links', 'not_links'];

// Every object in labels.json that points into a page's text.
export function* spans(doc) {
	for (const list of ['orgs', 'not_orgs', 'identifiers', 'not_identifiers']) yield* doc[list] ?? [];
	if (doc.statement) yield doc.statement;
	if (doc.passport) yield doc.passport.expiry_at;
}

function format(value, pad, key) {
	if (Array.isArray(value) && LISTS.includes(key)) {
		if (value.length === 0) return '[]';
		return `[\n${value.map((item) => `${pad}  ${JSON.stringify(item)}`).join(',\n')}\n${pad}]`;
	}
	if (Array.isArray(value)) {
		if (value.every((item) => typeof item !== 'object' || item === null))
			return JSON.stringify(value);
		return `[\n${value.map((item) => `${pad}  ${format(item, pad + '  ')}`).join(',\n')}\n${pad}]`;
	}
	const flat = (v) => typeof v !== 'object' || v === null || (Array.isArray(v) && v.every(flat));
	if (value && typeof value === 'object' && Object.values(value).every(flat))
		return JSON.stringify(value);
	if (value && typeof value === 'object') {
		const entries = Object.entries(value).map(
			([k, v]) => `${pad}  ${JSON.stringify(k)}: ${format(v, pad + '  ', k)}`
		);
		return entries.length ? `{\n${entries.join(',\n')}\n${pad}}` : '{}';
	}
	return JSON.stringify(value);
}

export const readLabels = (url) => JSON.parse(readFileSync(url, 'utf8'));
export const writeLabels = (url, labels) => writeFileSync(url, format(labels, '') + '\n');
