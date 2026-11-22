// Writes every document in spec.json into files/, the same bytes on every
// run, sets each file's modified time to its "created" date, and writes
// files/SHA256SUMS:
//   node sample/tools/generate.mjs
import { createHash } from 'node:crypto';
import { mkdirSync, readdirSync, readFileSync, rmSync, utimesSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { pdf } from './lib/pdf.mjs';

export const SAMPLE = new URL('../', import.meta.url);
export const FILES = new URL('files/', SAMPLE);
export const SUMS = 'SHA256SUMS';
const FORMATS = new Set(['pdf', 'txt', 'html']);

export const pageText = (page) => page.lines.join('\n');
export const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');

export function loadSpec() {
	const spec = JSON.parse(readFileSync(new URL('spec.json', SAMPLE), 'utf8'));
	const seen = new Set();
	for (const doc of spec.documents) {
		const where = `spec.json document ${doc.id}`;
		if (!/^[a-z0-9-]+$/.test(doc.id) || seen.has(doc.id))
			throw new Error(`${where}: id must be unique, lower case, digits and hyphens`);
		if (!/^[a-z0-9-]+\.(pdf|txt|html)$/.test(doc.file) || seen.has(doc.file))
			throw new Error(`${where}: file name must be unique and end in .pdf, .txt or .html`);
		seen.add(doc.id).add(doc.file);
		if (!FORMATS.has(doc.format) || !doc.file.endsWith('.' + doc.format))
			throw new Error(`${where}: format must be pdf, txt or html and match the file name`);
		if (!/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/.test(doc.created))
			throw new Error(`${where}: created must be YYYY-MM-DDTHH:MM:SSZ`);
		if (doc.format !== 'pdf' && (doc.pages.length !== 1 || doc.pages[0].image))
			throw new Error(`${where}: a txt or html document is one text page`);
		for (const page of doc.pages) {
			if (page.lines.some((line) => /[\t\r\n]/.test(line)))
				throw new Error(`${where}: a line holds a tab or line break`);
			// PDFKit drops blank lines and runs of spaces, so a text page has none
			// and its text reads back exactly as written.
			if (
				doc.format === 'pdf' &&
				!page.image &&
				page.lines.some((line) => line === '' || /^ | $| {2}/.test(line))
			) {
				throw new Error(
					`${where}: a PDF text line is blank or has leading, trailing or repeated spaces`
				);
			}
		}
	}
	return spec;
}

function escapeHtml(text) {
	return text
		.replaceAll('&', '&amp;')
		.replaceAll('<', '&lt;')
		.replaceAll('>', '&gt;')
		.replaceAll('"', '&quot;');
}

export function render(doc) {
	if (doc.format === 'pdf') return pdf(doc);
	const lines = doc.pages[0].lines;
	if (doc.format === 'txt') return Buffer.from(lines.join('\n') + '\n', 'utf8');
	const body = lines.filter((line) => line !== '').map((line) => `<p>${escapeHtml(line)}</p>`);
	const html = [
		'<!doctype html>',
		`<html lang="${doc.lang}">`,
		'<head>',
		'<meta charset="utf-8">',
		`<title>${escapeHtml(doc.title)}</title>`,
		'</head>',
		'<body>',
		...body,
		'</body>',
		'</html>',
		''
	];
	return Buffer.from(html.join('\n'), 'utf8');
}

export const sumsFile = (built) =>
	built
		.map(({ doc, bytes }) => `${sha256(bytes)}  ${doc.file}`)
		.sort((a, b) => a.slice(66).localeCompare(b.slice(66), 'en'))
		.join('\n') + '\n';

export const build = (spec) => spec.documents.map((doc) => ({ doc, bytes: render(doc) }));

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
	const built = build(loadSpec());
	mkdirSync(FILES, { recursive: true });
	const keep = new Set([SUMS, ...built.map(({ doc }) => doc.file)]);
	for (const name of readdirSync(FILES)) if (!keep.has(name)) rmSync(new URL(name, FILES));
	for (const { doc, bytes } of built) {
		const path = new URL(doc.file, FILES);
		writeFileSync(path, bytes);
		const time = new Date(doc.created);
		utimesSync(path, time, time);
	}
	writeFileSync(new URL(SUMS, FILES), sumsFile(built));
	console.log(`wrote ${built.length} documents to sample/files`);
}
