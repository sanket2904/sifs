// Writes the page text an extractor reads, one JSON file per document (see
// sample/FORMAT.md):
//   node sample/tools/text.mjs          sample/text/ from spec.json, and
//                                       sample/text-ocr/ keeping its
//                                       committed Vision readings
//   node sample/tools/text.mjs --ocr    reads every scanned page again with
//                                       Apple Vision instead (macOS only)
//   node sample/tools/text.mjs --check  writes nothing; fails when a
//                                       committed file differs from what
//                                       this tool writes
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { FILES, loadSpec, pageText, SAMPLE } from './generate.mjs';
import { readLabels } from './lib/labels.mjs';
import { isMain } from './lib/main.mjs';

export const TEXT_SETS = ['text', 'text-ocr'];

// Hashed from the file's bytes, not read from SHA256SUMS, so --check fails
// when a file changes and its recorded hash does not.
const fileHash = (file) =>
	createHash('sha256')
		.update(readFileSync(new URL(file, FILES)))
		.digest('hex');

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
		files.set(`${id}.json`, { id, file, sha256: fileHash(file), format, lang, created, pages });
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

const serialize = (body) => JSON.stringify(body, null, '\t') + '\n';

// Both sets as this tool writes them without Vision: every page from
// spec.json, except that a scanned page in text-ocr keeps its committed
// reading, because that reading depends on the Mac that made it.
function expectedSets() {
	const spec = loadSpec();
	const referenceDate = readLabels(new URL('labels.json', SAMPLE)).reference_date;
	const text = textSet(spec);
	text.set('index.json', index('text', spec, referenceDate));
	const committed = readTextSet('text-ocr');
	const ocr = textSet(spec, (doc, i) => committed.docs.get(doc.id)?.pages[i]?.text ?? null);
	ocr.set('index.json', index('text-ocr', spec, referenceDate, { ocr: committed.index.ocr }));
	return { spec, referenceDate, text, 'text-ocr': ocr };
}

// Each problem names a committed file that differs from what this tool writes.
export function checkTextSets() {
	const expected = expectedSets();
	const problems = [];
	for (const name of TEXT_SETS) {
		const dir = new URL(`${name}/`, SAMPLE);
		const onDisk = new Set(existsSync(dir) ? readdirSync(dir) : []);
		for (const [file, body] of expected[name]) {
			if (!onDisk.has(file)) problems.push(`${name}/${file}: missing`);
			else if (readFileSync(new URL(file, dir), 'utf8') !== serialize(body))
				problems.push(`${name}/${file}: differs from what text.mjs writes`);
			onDisk.delete(file);
		}
		for (const file of onDisk) problems.push(`${name}/${file}: not written by text.mjs`);
	}
	return problems;
}

function write(name, files) {
	for (const [file, body] of files)
		if (body.pages?.some((page) => page.text === null))
			throw new Error(`${name}/${file}: a scanned page has no reading; run with --ocr`);
	const dir = new URL(`${name}/`, SAMPLE);
	mkdirSync(dir, { recursive: true });
	for (const old of readdirSync(dir)) if (!files.has(old)) rmSync(new URL(old, dir));
	for (const [file, body] of files) writeFileSync(new URL(file, dir), serialize(body));
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

if (isMain(import.meta.url)) {
	if (process.argv.includes('--check')) {
		const problems = checkTextSets();
		for (const problem of problems) console.error(problem);
		if (problems.length) {
			console.error(`${problems.length} text files are stale; run node sample/tools/text.mjs`);
			process.exit(1);
		}
		console.log('sample/text and sample/text-ocr match what text.mjs writes');
	} else {
		const { spec, referenceDate, text, 'text-ocr': kept } = expectedSets();
		write('text', text);
		if (process.argv.includes('--ocr')) {
			const read = vision(spec);
			const ocr = textSet(spec, (doc, i) => read.files[doc.file][i].join('\n'));
			ocr.set('index.json', index('text-ocr', spec, referenceDate, { ocr: read.vision }));
			write('text-ocr', ocr);
		} else write('text-ocr', kept);
	}
}

export function readTextSet(name, sample = SAMPLE) {
	const dir = new URL(`${name}/`, sample);
	const idx = JSON.parse(readFileSync(new URL('index.json', dir), 'utf8'));
	const docs = new Map(
		idx.documents.map((id) => [id, JSON.parse(readFileSync(new URL(`${id}.json`, dir), 'utf8'))])
	);
	return { index: idx, docs };
}
