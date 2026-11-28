// @ts-nocheck: run by node; the project has no Node types, and adding them is a new dependency.
// Writes build/things-preview.html: the things section and pages drawn by the
// real components from the committed goldens, with the page's own styles. No
// app, no server, no window: the components are compiled and rendered to HTML
// here, and the file is only written.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { setImmediate as turn } from 'node:timers/promises';
import { compile } from 'svelte/compiler';
import { render } from 'svelte/server';
import { createThings } from '../src/lib/things.js';

const root = new URL('../', import.meta.url);
const read = (path) => readFileSync(new URL(path, root), 'utf8');
const golden = (name) => JSON.parse(read(`engine/link/show/testdata/${name}`));

/** Compiles a component for the server and loads it, its imports pointed at real files. */
async function component(path) {
	const out = compile(read(path), { filename: path, generate: 'server', warningFilter: () => false });
	const code = out.js.code.replace(/from '([^']+)'/g, (_, spec) => `from '${spec.startsWith('.') ? new URL(spec, new URL(path, root)).href : import.meta.resolve(spec)}'`);
	const module = await import(`data:text/javascript,${encodeURIComponent(code)}`);
	return { draw: module.default, css: out.css?.code ?? '' };
}

/** The page's own styles: its colours for light and dark, its type and spacing. */
const page = compile(read('src/routes/+page.svelte'), { filename: '+page.svelte', generate: 'server', warningFilter: () => false });
const Things = await component('src/lib/Things.svelte');
const ThingPage = await component('src/lib/ThingPage.svelte');

const FILES_FIND = golden('files-find.json');
const FILES_PAGES = golden('files-pages.json');
const SHOP = golden('shop.json');
const finds = new Map([...Object.entries(FILES_FIND), ...Object.entries(SHOP)]);
const pages = new Map([...Object.values(FILES_PAGES), ...Object.values(SHOP)].filter((p) => p.values).map((p) => [p.id, p]));

/** The module's view after one act, against a reader that answers from the goldens. */
async function viewAfter(act) {
	let view;
	const due = [];
	const things = createThings({
		invoke: async (command, args) => {
			if (command === 'reader_find') return finds.get(args.words);
			if (command === 'reader_page') return pages.get(args.id);
			throw { reason: 'not_found' };
		},
		now: () => 0,
		setTimer: (run) => due.push(run),
		clearTimer: () => {},
		show: (next) => (view = next)
	});
	await act(things, () => view);
	while (due.length) due.shift()();
	await turn();
	return view;
}

const escape = (s) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
const panels = [];

for (const words of ["a German bank's printed name", 'a word of several kinds', 'find a word of two kinds']) {
	const view = await viewAfter((things) => things.type(words));
	panels.push(`<h2>The things found: ${escape(words)}</h2><div class="frame list">${render(Things.draw, { props: { groups: view.groups, line: view.line } }).body}</div>`);
}

const opened = [
	['a bank', 'org:9e577a3773d0d204', false],
	['an account', 'identifier:2288ef2e20318bbd', false],
	["a statement, with its first value's files open", 'statement:abn-amro-statement', true],
	['a passport holder', 'holder:passport-bad-mrz', false],
	['an invoice of the bakery', 'invoice:0041', false]
];
for (const [what, id, sources] of opened) {
	const view = await viewAfter(async (things, now) => {
		things.open(id);
		await turn();
		if (sources) now().page.values[0].toggle();
	});
	panels.push(`<h2>The page of ${escape(what)}</h2><div class="frame">${render(ThingPage.draw, { props: { page: view.page, depth: 1, back: () => {} } }).body}</div>`);
}

const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>SIFS things preview</title>
<style>${page.css?.code ?? ''}</style>
<style>${Things.css}</style>
<style>${ThingPage.css}</style>
<style>
	.preview { max-width: 760px; margin: 0 auto; padding: 16px; }
	.preview > h2 { font-size: 13px; font-weight: 600; color: var(--muted); margin: 22px 0 6px; }
	.frame { position: relative; height: 520px; background: var(--bg); border: 1px solid var(--line); border-radius: 10px; overflow: hidden; }
	.frame.list { height: auto; padding: 6px 0; }
</style>
</head>
<body>
<main class="preview">
${panels.join('\n')}
</main>
</body>
</html>
`;

mkdirSync(new URL('build/', root), { recursive: true });
const out = new URL('build/things-preview.html', root);
writeFileSync(out, html);
console.log(`wrote ${out.pathname}`);
