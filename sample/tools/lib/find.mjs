// Finds a labelled text in a page text that may not be the one its span
// counts in (the OCR text set). Returns a span in Unicode scalar values, or
// null when the page holds neither the text nor its value with any spacing.
const escape = (c) => c.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

export function locate(pageText, text, value = text) {
	const toPoint = (utf16) => [...pageText.slice(0, utf16)].length;
	let at = pageText.indexOf(text);
	if (at >= 0) {
		const start = toPoint(at);
		return { start, end: start + [...text].length, text };
	}
	const chars = [...value.replace(/\s/gu, '')];
	if (chars.length === 0) return null;
	const m = new RegExp(chars.map(escape).join('\\s*'), 'u').exec(pageText);
	if (!m) return null;
	const start = toPoint(m.index);
	return { start, end: start + [...m[0]].length, text: m[0] };
}

export const readsBack = (pageText, span) =>
	[...pageText].slice(span.start, span.end).join('') === span.text;
