// Every organisation and identifier a document's author put in its text is
// listed in spec.json under "planted". This proves the labels against that
// list: each planted token is labelled with its exact span (an identifier as
// the kind it was planted as), each label is a planted token, and every
// organisation-shaped word or identifier-shaped number on a page lies inside a
// planted token or an "ignore" entry.

const ORG_WORDS =
	/(?<![\p{L}\p{M}\p{N}])(bank|bundesbank|commerzbank|ltd|limited|gmbh|ag|plc|corporation|company|office|school|insurance|lombard|traders|supplies|grocers|books|cafe|paribas|union|sahakari|distribution)(?![\p{L}\p{M}\p{N}])|बैंक|लिमिटेड|銀行/giu;
const ID_SHAPES = [
	/\d(?: ?\d){7,}/g,
	/\b[A-Z]{4}0[A-Z0-9]{6}\b/g,
	/\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]{4}){3,7}(?: ?[A-Z0-9]{1,3})?\b/g,
	/\b[A-Z]{5}\d{4}[A-Z]\b/g,
	/[A-Z0-9<]{30,}/g,
	/\b\d\d-\d\d-\d\d\b/g,
	/\b\d{5,7}\b/g
];

const word = (c) =>
	c !== undefined &&
	/[\p{L}\p{M}\p{N}]/u.test(c) &&
	!/[\p{scx=Han}\p{scx=Hiragana}\p{scx=Katakana}]/u.test(c);

// Occurrences of text on each page, in Unicode scalar values, that do not run
// into a neighbouring letter or digit.
function occurrences(pages, text) {
	const want = [...text];
	const found = [];
	pages.forEach((chars, index) => {
		for (let i = 0; i + want.length <= chars.length; i++) {
			if (!want.every((c, j) => chars[i + j] === c)) continue;
			const end = i + want.length;
			if ((word(want[0]) && word(chars[i - 1])) || (word(want.at(-1)) && word(chars[end])))
				continue;
			found.push({ page: index + 1, start: i, end, text });
		}
	});
	return found;
}

const inside = (a, b) => a.page === b.page && b.start <= a.start && a.end <= b.end;
const same = (a, b) => a.page === b.page && a.start === b.start && a.end === b.end;
const longer = (a, b) => b.end - b.start > a.end - a.start;

function hits(pages, patterns) {
	const found = [];
	pages.forEach((chars, index) => {
		const text = chars.join('');
		const toPoint = (utf16) => [...text.slice(0, utf16)].length;
		for (const pattern of patterns) {
			for (const m of text.matchAll(pattern)) {
				const start = toPoint(m.index);
				found.push({ page: index + 1, start, end: start + [...m[0]].length, text: m[0] });
			}
		}
	});
	return found;
}

// Why a planted identifier's label is not what spec.json planted, or null.
// spec.json gives each one either the kind it was planted as, or the kind it
// only looks like and why it is not one.
function plantedAs(entry, [list, label]) {
	const want =
		entry.kind !== undefined && entry.not === undefined
			? { list: 'identifier', kind: entry.kind }
			: entry.not !== undefined && entry.kind === undefined
				? { list: 'not_identifier', kind: entry.looks_like, why: entry.not }
				: null;
	if (!want) return 'in spec.json needs either kind, or looks_like and not';
	const got =
		list === 'identifier'
			? { list, kind: label.kind }
			: { list, kind: label.looks_like, why: label.why };
	return JSON.stringify(got) === JSON.stringify(want)
		? null
		: `is planted as ${Object.values(want).join(' ')}, its label is ${Object.values(got).join(' ')}`;
}

// Returns { rows, problems } for one document. rows lists each planted token
// occurrence and the label that holds it.
export function planted(source, doc, pages) {
	const rows = [];
	const problems = [];
	const plan = source.planted;
	if (!plan) return { rows, problems: ['spec.json has no planted list'] };
	const labels = {
		org: [
			...(doc.orgs ?? []).map((l) => ['org', l]),
			...(doc.not_orgs ?? []).map((l) => ['not_org', l])
		],
		identifier: [
			...(doc.identifiers ?? []).map((l) => ['identifier', l]),
			...(doc.not_identifiers ?? []).map((l) => ['not_identifier', l])
		]
	};
	const tokens = { org: [], identifier: [] };
	for (const [kind, list] of [
		['org', plan.orgs],
		['identifier', plan.identifiers]
	]) {
		for (const item of list) {
			const entry = typeof item === 'string' ? { text: item } : item;
			const at = occurrences(pages, entry.text);
			if (!at.length)
				problems.push(`planted ${kind} ${JSON.stringify(entry.text)} is not in the text`);
			tokens[kind].push(...at.map((t) => ({ ...t, entry })));
		}
		for (const token of tokens[kind]) {
			if (tokens[kind].some((t) => t !== token && longer(token, t) && inside(token, t))) continue;
			const label = labels[kind].find(([, l]) => same(l, token));
			const { entry, ...at } = token;
			rows.push({
				kind,
				...at,
				label: label ? `${label[0]} ${label[1].id ?? ''}`.trim() : null
			});
			if (!label)
				problems.push(
					`planted ${kind} ${JSON.stringify(token.text)} at page ${token.page} ${token.start}..${token.end} has no label with exactly that span`
				);
			else if (kind === 'identifier') {
				const problem = plantedAs(entry, label);
				if (problem) problems.push(`planted identifier ${JSON.stringify(token.text)} ${problem}`);
			}
		}
	}
	for (const [list, kind] of [
		[doc.orgs ?? [], 'org'],
		[doc.identifiers ?? [], 'identifier'],
		[doc.not_identifiers ?? [], 'identifier']
	]) {
		for (const l of list) {
			if (!tokens[kind].some((t) => same(l, t)))
				problems.push(`label ${JSON.stringify(l.text)} is not a planted ${kind} in spec.json`);
		}
	}
	const ignored = (plan.ignore ?? []).flatMap((entry) => {
		const at = occurrences(pages, entry.text);
		if (!at.length) problems.push(`ignore ${JSON.stringify(entry.text)} is not in the text`);
		if (!entry.why) problems.push(`ignore ${JSON.stringify(entry.text)} has no why`);
		return at;
	});
	for (const [kind, patterns] of [
		['org', [ORG_WORDS]],
		['identifier', ID_SHAPES]
	]) {
		for (const hit of hits(pages, patterns)) {
			if ([...tokens[kind], ...ignored].some((t) => inside(hit, t))) continue;
			problems.push(
				`page ${hit.page} has ${kind}-shaped ${JSON.stringify(hit.text)} at ${hit.start}, which no planted ${kind} or ignore entry holds`
			);
		}
	}
	return { rows, problems };
}
