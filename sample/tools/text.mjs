// Writes the page text an extractor reads, one JSON file per document (see
// sample/FORMAT.md):
//   node sample/tools/text.mjs          sample/text/ from spec.json
//   node sample/tools/text.mjs --ocr    also sample/text-ocr/, where each
//                                       scanned page holds what Apple Vision
//                                       reads from its picture (macOS only)
import { execFileSync } from 'node:child_process';
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { FILES, loadSpec, pageText, SAMPLE } from './generate.mjs';
import { readLabels } from './lib/labels.mjs';

export const TEXT_SETS = ['text', 'text-ocr'];

// readScan(doc, pageIndex) gives a scanned page's text; without it a scanned
// page holds the spec's text, as a perfect reading would.
export function textSet(spec, readScan) {
	const files = new Map();
	for (const doc of spec.documents) {
		const { id, file, format, lang, created } = doc;
		const pages = doc.pages.map((page, i) => ({
			text: page.image && readScan ? readScan(doc, i) : pageText(page),
			scanned: Boolean(page.image)
		}));
		files.set(`${id}.json`, { id, file, format, lang, created, pages });
	}
	return files;
}

export function index(name, spec, referenceDate, extra = {}) {
	return {
		format: 1,
		text_set: name,
		reference_date: referenceDate,
		documents: spec.documents.map((doc) => doc.id),
		...extra
	};
}

function write(name, files) {
	const dir = new URL(`${name}/`, SAMPLE);
	mkdirSync(dir, { recursive: true });
	for (const old of readdirSync(dir)) if (!files.has(old)) rmSync(new URL(old, dir));
	for (const [file, body] of files)
		writeFileSync(new URL(file, dir), JSON.stringify(body, null, '\t') + '\n');
	console.log(`wrote ${files.size} files to sample/${name}`);
}

function vision(spec) {
	const scanned = spec.documents.filter((doc) => doc.pages.some((page) => page.image));
	const script = fileURLToPath(new URL('vision-text.swift', import.meta.url));
	return JSON.parse(
		execFileSync(
			'swift',
			[script, ...scanned.map((doc) => fileURLToPath(new URL(doc.file, FILES)))],
			{ encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 1 << 26 }
		)
	);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
	const spec = loadSpec();
	const referenceDate = readLabels(new URL('labels.json', SAMPLE)).reference_date;
	const text = textSet(spec);
	text.set('index.json', index('text', spec, referenceDate));
	write('text', text);
	if (process.argv.includes('--ocr')) {
		const read = vision(spec);
		const ocr = textSet(spec, (doc, i) => read.files[doc.file][i].join('\n'));
		ocr.set('index.json', index('text-ocr', spec, referenceDate, { ocr: read.vision }));
		write('text-ocr', ocr);
	}
}

export const readTextSet = (name, sample = SAMPLE) => {
	const dir = new URL(`${name}/`, sample);
	const idx = JSON.parse(readFileSync(new URL('index.json', dir), 'utf8'));
	const docs = new Map(
		idx.documents.map((id) => [id, JSON.parse(readFileSync(new URL(`${id}.json`, dir), 'utf8'))])
	);
	return { index: idx, docs };
};
