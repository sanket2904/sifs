// Authoring aid: fills start and end for every span in labels.json that has
// page and text but no start yet, at the nth occurrence (default 1) of text
// on that page. check.mjs proves the result; this only saves counting.
//   node sample/tools/locate.mjs
import { loadSpec, pageText, SAMPLE } from './generate.mjs';
import { readLabels, spans, writeLabels } from './lib/labels.mjs';

const url = new URL('labels.json', SAMPLE);
const labels = readLabels(url);
const docs = new Map(loadSpec().documents.map((doc) => [doc.id, doc]));
let filled = 0;
for (const doc of labels.documents) {
	for (const span of spans(doc)) {
		if (span.start !== undefined) continue;
		const page = [...pageText(docs.get(doc.id).pages[span.page - 1])];
		const want = [...span.text];
		let seen = 0;
		let at = -1;
		for (let i = 0; i + want.length <= page.length && at < 0; i++) {
			if (want.every((c, j) => page[i + j] === c) && ++seen === (span.nth ?? 1)) at = i;
		}
		if (at < 0)
			throw new Error(`${doc.id}: ${JSON.stringify(span.text)} not found on page ${span.page}`);
		const { id, page: pageNumber, text, ...rest } = span;
		delete rest.nth;
		for (const key of Object.keys(span)) delete span[key];
		Object.assign(
			span,
			id ? { id } : {},
			{ page: pageNumber, start: at, end: at + want.length, text },
			rest
		);
		filled++;
	}
}
writeLabels(url, labels);
console.log(`filled ${filled} spans`);
