// @ts-nocheck: run by node --test; the project has no Node types, and adding them is a new dependency.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { setImmediate as turn } from 'node:timers/promises';
import { createSearch, TYPING, CONTENTS } from '../src/lib/search.js';
import { createThings, SHOWN } from '../src/lib/things.js';

const golden = (name) => JSON.parse(readFileSync(new URL(`../engine/link/show/testdata/${name}`, import.meta.url), 'utf8'));
const FILES_FIND = golden('files-find.json');
const FILES_PAGES = golden('files-pages.json');
const SHOP = golden('shop.json');

/** Every page the goldens hold, by id, with the schema it is from. */
const PAGES = new Map();
for (const page of Object.values(FILES_PAGES)) PAGES.set(page.id, page);
for (const [name, page] of Object.entries(SHOP)) if (name.startsWith('page ')) PAGES.set(page.id, page);
/** The finds of both schemas, by the words the fake reader answers them for. */
const FINDS = new Map([
	...Object.entries(FILES_FIND),
	...Object.entries(SHOP).filter(([name]) => name.startsWith('find '))
]);

const NO_ANSWER = {
	no_tools: { reason: 'no_tools', why: 'the reader is not installed' },
	nothing_read_yet: { reason: 'nothing_read_yet' },
	took_too_long: { reason: 'took_too_long' },
	could_not_answer: { reason: 'could_not_answer', said: 'the store is busy' },
	quitting: { reason: 'quitting' }
};

/**
 * A fake clock and a fake backend shared by every module made on it.
 * `answer(call)` plans each call: undefined keeps it for the test to settle,
 * `{ after, value | error }` settles it on the clock.
 */
function clock({ answer = () => undefined } = {}) {
	let t = 0;
	let nextId = 0;
	const timers = new Map();
	const calls = [];
	function settle(call, plan) {
		if (call.done) return;
		call.done = true;
		if ('error' in plan) call.reject(plan.error);
		else call.resolve(plan.value);
	}
	const c = {
		calls,
		timers,
		get now() {
			return t;
		},
		io(show) {
			return {
				now: () => t,
				setTimer(run, ms) {
					const id = ++nextId;
					timers.set(id, { at: t + ms, run, order: id });
					return id;
				},
				clearTimer: (id) => timers.delete(id),
				invoke(command, args) {
					return new Promise((resolve, reject) => {
						const call = { command, args, at: t, done: false, resolve, reject };
						calls.push(call);
						const plan = answer(call);
						if (plan) timers.set(++nextId, { at: t + plan.after, order: nextId, run: () => settle(call, plan) });
					});
				},
				show
			};
		},
		of: (command) => calls.filter((call) => call.command === command),
		async advance(ms) {
			const end = t + ms;
			for (;;) {
				let next = null;
				for (const [id, timer] of timers) if (timer.at <= end && (!next || timer.at < next[1].at || (timer.at === next[1].at && timer.order < next[1].order))) next = [id, timer];
				if (!next) break;
				timers.delete(next[0]);
				t = next[1].at;
				next[1].run();
				await turn();
			}
			t = end;
			await turn();
		},
		async reply(call, value) {
			settle(call, { value });
			await turn();
		},
		async fail(call, error) {
			settle(call, { error });
			await turn();
		}
	};
	return c;
}

/** The things module on a clock; `view()` is what it showed last. */
function thingsOn(c) {
	const views = [];
	const things = createThings(c.io((view) => views.push({ at: c.now, ...view })));
	return { things, views, view: () => views.at(-1) };
}

/** A reader that answers every question from the goldens at once. */
const fromGoldens = (call) => {
	if (call.command === 'reader_find') return FINDS.has(call.args.words) ? { after: 0, value: FINDS.get(call.args.words) } : { after: 0, value: { groups: [] } };
	if (call.command === 'reader_page') return PAGES.has(call.args.id) ? { after: 0, value: PAGES.get(call.args.id) } : { after: 0, error: NO_ANSWER.could_not_answer };
	return undefined;
};

const ids = (view) => view.groups.flatMap((g) => g.rows.map((r) => r.id));
const shownIds = (golden) => golden.groups.flatMap((g) => g.things.slice(0, SHOWN).map((t) => t.id));
const section = (view) => ({ groups: view.groups.length, line: view.line });

/** Types and lets the pause pass, so the question is asked and answered. */
async function find(c, things, words) {
	things.type(words);
	await c.advance(TYPING);
	await turn();
}

// T1. The find loop.

test('T1: one question per pause in typing, with exactly the words in the box', async () => {
	const c = clock({ answer: fromGoldens });
	const { things } = thingsOn(c);
	for (const text of ['C', 'Co', 'Com', 'Commerz']) {
		things.type(text);
		await c.advance(TYPING - 1);
	}
	assert.equal(c.of('reader_find').length, 0, 'nothing asked while typing goes on');
	await c.advance(1);
	assert.deepEqual(
		c.of('reader_find').map((call) => call.args),
		[{ words: 'Commerz' }],
		'one question when typing pauses'
	);
	things.type('  Commerz ');
	await c.advance(TYPING * 10);
	assert.equal(c.of('reader_find').length, 1, 'the same words once trimmed ask nothing new');
	things.type('Commerzbank');
	await c.advance(TYPING);
	assert.deepEqual(c.of('reader_find').at(-1).args, { words: 'Commerzbank' }, 'the next pause asks again');
});

test('T1: the reader groups are shown as it groups them, in its order, under the kind and its count', async () => {
	const c = clock({ answer: fromGoldens });
	const { things, view } = thingsOn(c);
	for (const [words, answer] of FINDS) {
		await find(c, things, words);
		assert.equal(view().groups.length, answer.groups.length, `${words}: one group per reader group`);
		answer.groups.forEach((g, i) => {
			const shown = view().groups[i];
			assert.equal(shown.label, g.label, `${words}: the kind's label`);
			assert.equal(shown.count, g.things.length, `${words}: the group's count`);
			assert.equal(shown.collection[0], g.collection[0].toUpperCase(), `${words}: the collection capitalised`);
			assert.equal(shown.collection.slice(1), g.collection.slice(1), `${words}: and nothing more`);
			g.things.slice(0, SHOWN).forEach((t, j) => {
				const row = shown.rows[j];
				assert.equal(row.id, t.id);
				assert.equal(row.title, t.title);
				assert.equal(row.subtitle, t.subtitle && t.subtitle !== t.title ? t.subtitle : '', 'a subtitle equal to its title is not shown');
				assert.equal(row.matched, `${t.matched.label}: ${t.matched.value}`);
			});
		});
		assert.deepEqual(ids(view()), shownIds(answer), `${words}: no row dropped, merged or moved`);
	}
	await find(c, things, 'a word that is nowhere');
	assert.deepEqual(section(view()), { groups: 0, line: '' }, 'no things found: no section');
});

test('T1: a group of more than five shows five and then all of them when asked', async () => {
	const many = structuredClone(FILES_FIND["a German bank's printed name"]);
	const extra = many.groups[0].things.map((t) => ({ ...t, id: `${t.id}-again` }));
	many.groups[0].things.push(...extra);
	const c = clock({ answer: (call) => (call.command === 'reader_find' ? { after: 0, value: many } : undefined) });
	const { things, view } = thingsOn(c);
	await find(c, things, 'Commerzbank');
	const group = view().groups[0];
	assert.equal(group.rows.length, SHOWN);
	assert.equal(group.count, 10);
	assert.equal(group.more, true);
	group.showAll();
	assert.deepEqual(
		view().groups[0].rows.map((r) => r.id),
		many.groups[0].things.map((t) => t.id),
		'all of them, in the reader order'
	);
	assert.equal(view().groups[0].more, false);
});

test('T1: an older answer arriving after a newer question is dropped', async () => {
	const c = clock();
	const { things, view } = thingsOn(c);
	things.type('Commerzbank');
	await c.advance(TYPING);
	things.type('Eriksson');
	await c.advance(TYPING);
	const [older, newer] = c.of('reader_find');
	await c.reply(older, FILES_FIND["a German bank's printed name"]);
	assert.deepEqual(section(view()), { groups: 0, line: '' }, 'the older answer is not shown, even before the newer one comes');
	await c.reply(newer, FILES_FIND["a holder's surname"]);
	assert.deepEqual(ids(view()), shownIds(FILES_FIND["a holder's surname"]));
	await c.fail(older, NO_ANSWER.took_too_long);
	assert.deepEqual(ids(view()), shownIds(FILES_FIND["a holder's surname"]), 'nor is an older failure');
});

test('T1: the section keeps the last answer while the next question is asked', async () => {
	const c = clock();
	const { things, view } = thingsOn(c);
	things.type('Commerzbank');
	await c.advance(TYPING);
	await c.reply(c.of('reader_find')[0], FILES_FIND["a German bank's printed name"]);
	things.type('Commerzbank AG');
	await c.advance(TYPING);
	assert.deepEqual(ids(view()), shownIds(FILES_FIND["a German bank's printed name"]), 'nothing jumps while the reader thinks');
});

test('T1: clearing the box clears the section at once, and a late answer stays out', async () => {
	const c = clock();
	const { things, view } = thingsOn(c);
	things.type('Commerzbank');
	await c.advance(TYPING);
	await c.reply(c.of('reader_find')[0], FILES_FIND["a German bank's printed name"]);
	things.type('Eriksson');
	await c.advance(TYPING);
	const at = c.now;
	things.type('');
	assert.deepEqual(section(view()), { groups: 0, line: '' }, 'cleared with no wait');
	assert.equal(view().at, at);
	await c.reply(c.of('reader_find')[1], FILES_FIND["a holder's surname"]);
	await c.advance(60_000);
	assert.deepEqual(section(view()), { groups: 0, line: '' }, 'the answer to the cleared words never shows');
	assert.equal(c.of('reader_find').length, 2, 'an empty box asks nothing');
	assert.equal(c.timers.size, 0, 'and leaves no timer');
});

test('T1: every reason the reader gives no answer shows what it should', async () => {
	for (const [reason, error] of Object.entries(NO_ANSWER)) {
		const c = clock();
		const { things, view } = thingsOn(c);
		things.type('Commerzbank');
		await c.advance(TYPING);
		await c.reply(c.of('reader_find')[0], FILES_FIND["a German bank's printed name"]);
		things.type('Eriksson');
		await c.advance(TYPING);
		await c.fail(c.of('reader_find')[1], error);
		assert.equal(view().groups.length, 0, `${reason}: no groups`);
		if (reason === 'took_too_long') assert.notEqual(view().line, '', `${reason}: one quiet line`);
		else if (reason === 'could_not_answer') assert.match(view().line, /the store is busy/, `${reason}: the reader's words`);
		else assert.equal(view().line, '', `${reason}: nothing at all, the screen is as it was before`);
	}
});

test('T1: a reader that takes 3 s delays, reorders and blanks nothing the file search shows', async () => {
	const fileRow = (name) => ({ path: `/docs/${name}`, name });
	/** The file backend answers names in 5 ms and contents in 30 ms; the reader takes 3 s. */
	const answer = (call) => {
		if (call.command === 'search_files') return { after: 5, value: [fileRow(`${call.args.query}-name`)] };
		if (call.command === 'search_all') return { after: 30, value: { hits: [fileRow(`${call.args.query}-name`), fileRow(`${call.args.query}-text`)] } };
		if (call.command === 'reader_find') return { after: 3000, value: FILES_FIND["a German bank's printed name"] };
		return undefined;
	};
	async function run(withThings) {
		const c = clock({ answer });
		const files = [];
		const search = createSearch(c.io((view) => files.push({ at: c.now, rows: view.rows.map((r) => r.name), pending: view.pending })));
		const things = withThings ? thingsOn(c) : null;
		for (const text of ['Co', 'Commerz', 'Commerzbank']) {
			search.type(text);
			things?.things.type(text);
			await c.advance(TYPING + CONTENTS);
		}
		search.type('');
		things?.things.type('');
		search.type('Eriksson');
		things?.things.type('Eriksson');
		await c.advance(10_000);
		return { files, things, reader: c.of('reader_find').length };
	}
	const alone = await run(false);
	const beside = await run(true);
	assert.ok(beside.reader > 0, 'the reader was asked');
	assert.ok(beside.things.views.some((v) => v.groups.length > 0), 'and its things showed');
	assert.deepEqual(beside.files, alone.files, 'every file view, at the same moment, as without the reader');
});

test('T1: 200 keystrokes ask the reader at most as often as the file search asks its backend', async () => {
	let seed = 7;
	const random = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
	const c = clock({
		answer: (call) =>
			call.command === 'search_files' ? { after: 3, value: [] } : call.command === 'search_all' ? { after: 9, value: { hits: [] } } : call.command === 'reader_find' ? { after: 20, value: { groups: [] } } : undefined
	});
	const search = createSearch(c.io(() => {}));
	const { things } = thingsOn(c);
	let text = '';
	for (let i = 0; i < 200; i++) {
		text = random() < 0.15 && text ? text.slice(0, -1) : text + 'abcdefghij'[i % 10];
		search.type(text);
		things.type(text);
		await c.advance(Math.floor(random() * 2.5 * TYPING));
	}
	await c.advance(10_000);
	assert.ok(c.of('reader_find').length > 0);
	assert.ok(c.of('reader_find').length <= c.of('search_files').length, `${c.of('reader_find').length} questions against ${c.of('search_files').length} name searches`);
});

// T2. The page and the stack.

/** Opens a page by id as a link or a row would, and lets it answer. */
async function opened(c, open) {
	open();
	await turn();
	await c.advance(0);
}

/** Asserts that a page's view holds everything the golden page holds, in its order. */
function holdsAll(page, golden) {
	assert.equal(page.state, 'shown', `${golden.id}: shown`);
	assert.equal(page.kind, golden.label, `${golden.id}: the kind's label`);
	assert.equal(page.title, golden.title);
	assert.equal(page.subtitle, golden.subtitle && golden.subtitle !== golden.title ? golden.subtitle : '');
	assert.equal(page.values.length, golden.values.length, `${golden.id}: every value`);
	golden.values.forEach((v, i) => {
		const shown = page.values[i];
		assert.equal(shown.label, v.label);
		assert.equal(shown.value, v.values.join(', '));
		assert.equal(shown.count, v.sources.length, `${golden.id} ${v.label}: its source count`);
		assert.deepEqual(
			shown.sources.map((s) => s.record),
			v.sources,
			`${golden.id} ${v.label}: one row per source`
		);
	});
	assert.equal(page.links.length, golden.links.length, `${golden.id}: every link group`);
	golden.links.forEach((l, i) => {
		const shown = page.links[i];
		assert.equal(shown.label, l.label);
		assert.equal(shown.rows.length, l.rows.length, `${golden.id} ${l.label}: every row`);
		l.rows.forEach((r, j) => {
			assert.deepEqual(
				shown.rows[j].ends.map((e) => [e.label, e.title, e.subtitle]),
				r.ends.map((e) => [e.label, e.thing.title, e.thing.subtitle && e.thing.subtitle !== e.thing.title ? e.thing.subtitle : ''])
			);
			assert.deepEqual(
				shown.rows[j].values.map((a) => [a.label, a.value]),
				r.attributes.map((a) => [a.label, a.values.join(', ')])
			);
		});
	});
}

test('T2: a thing opened from its group shows everything its page holds, in the reader order', async () => {
	const c = clock({ answer: fromGoldens });
	const { things, view } = thingsOn(c);
	await find(c, things, "a holder's surname");
	await opened(c, () => view().groups[0].rows[0].open());
	assert.deepEqual(c.of('reader_page').map((call) => call.args), [{ id: 'holder:passport-bad-mrz' }]);
	holdsAll(view().page, PAGES.get('holder:passport-bad-mrz'));
});

test('T2: every golden page of both schemas draws whole', async () => {
	for (const [id, golden] of PAGES) {
		const c = clock({ answer: fromGoldens });
		const { things, view } = thingsOn(c);
		await opened(c, () => things.open(id));
		assert.deepEqual(c.of('reader_page').at(-1).args, { id });
		holdsAll(view().page, golden);
	}
});

test('T2: while a page loads it shows the title it was opened by', async () => {
	const c = clock();
	const { things, view } = thingsOn(c);
	await find(c, things, 'Eriksson');
	await c.reply(c.of('reader_find')[0], FILES_FIND["a holder's surname"]);
	view().groups[0].rows[0].open();
	assert.equal(view().page.state, 'loading');
	assert.equal(view().page.title, FILES_FIND["a holder's surname"].groups[0].things[0].title);
});

/** The other end of the first link of a page whose thing has this id. */
const endTo = (page, id) => page.links.flatMap((l) => l.rows.flatMap((r) => r.ends)).find((e) => e.id === id);

test('T2: follow a link and another; Back twice is the first page, then the results as they were', async () => {
	const c = clock({ answer: fromGoldens });
	const { things, view } = thingsOn(c);
	await find(c, things, 'find a name');
	const results = view().groups;
	await opened(c, () => results[0].rows[0].open());
	assert.equal(view().page.title, PAGES.get('customer:ana').title);
	await opened(c, () => endTo(view().page, 'invoice:0041').open());
	holdsAll(view().page, PAGES.get('invoice:0041'));
	await opened(c, () => endTo(view().page, 'product:loaf').open());
	holdsAll(view().page, PAGES.get('product:loaf'));
	assert.deepEqual(
		c.of('reader_page').map((call) => call.args),
		[{ id: 'customer:ana' }, { id: 'invoice:0041' }, { id: 'product:loaf' }],
		'each page asked for by the id of the thing at the other end'
	);
	assert.equal(view().depth, 3);
	things.back();
	holdsAll(view().page, PAGES.get('invoice:0041'));
	things.back();
	holdsAll(view().page, PAGES.get('customer:ana'));
	assert.equal(c.of('reader_page').length, 3, 'going back asks nothing again');
	things.back();
	assert.equal(view().page, null, 'the results again');
	assert.equal(view().groups, results, 'the very rows left, not a new search');
	assert.equal(c.of('reader_find').length, 1);
	things.back();
	assert.equal(view().page, null, 'Back with no page does nothing');
});

test('T2: opening a thing already on the stack goes back to it rather than growing the stack', async () => {
	const c = clock({ answer: fromGoldens });
	const { things, view } = thingsOn(c);
	await opened(c, () => things.open('passport:passport-bad-mrz'));
	await opened(c, () => endTo(view().page, 'holder:passport-bad-mrz').open());
	assert.equal(view().depth, 2);
	await opened(c, () => endTo(view().page, 'passport:passport-bad-mrz').open());
	assert.equal(view().depth, 1, 'back to the passport, the holder taken off');
	holdsAll(view().page, PAGES.get('passport:passport-bad-mrz'));
	assert.equal(c.of('reader_page').length, 2, 'the page on the stack is not asked for again');
	things.back();
	assert.equal(view().page, null);
});

test('T2: a page that fails to load says so, and Back still works', async () => {
	for (const [reason, error] of Object.entries(NO_ANSWER)) {
		const c = clock();
		const { things, view } = thingsOn(c);
		things.open('org:9e577a3773d0d204');
		await c.fail(c.of('reader_page')[0], error);
		assert.equal(view().page.state, 'failed', reason);
		assert.notEqual(view().page.line, '', `${reason}: it says so`);
		if (reason === 'could_not_answer') assert.match(view().page.line, /the store is busy/);
		things.back();
		assert.equal(view().page, null, `${reason}: Back works`);
	}
});

test('T2: an answer for a page the person has already left is dropped', async () => {
	const c = clock();
	const { things, view } = thingsOn(c);
	things.open('customer:ana');
	things.back();
	await c.reply(c.of('reader_page')[0], PAGES.get('customer:ana'));
	assert.equal(view().page, null, 'left for the results');
	things.open('invoice:0041');
	things.back();
	things.open('product:loaf');
	await c.reply(c.of('reader_page')[1], PAGES.get('invoice:0041'));
	assert.equal(view().page.state, 'loading', 'left for another page');
	await c.reply(c.of('reader_page')[2], PAGES.get('product:loaf'));
	holdsAll(view().page, PAGES.get('product:loaf'));
});

test('T2: a source opens its file by exactly its record id, and a refusal shows in place', async () => {
	const c = clock({ answer: fromGoldens });
	const { things, view } = thingsOn(c);
	await opened(c, () => things.open('statement:abn-amro-statement'));
	const golden = PAGES.get('statement:abn-amro-statement');
	assert.equal(view().page.values[0].open, false, 'sources start closed');
	view().page.values[0].toggle();
	assert.equal(view().page.values[0].open, true);
	assert.equal(view().page.values[1].open, false, 'only that value opens');
	const record = golden.values[0].sources[0];
	view().page.values[0].sources[0].open();
	const asked = c.of('open_source');
	assert.deepEqual(asked.map((call) => call.args), [{ record }]);
	await c.fail(asked[0], { reason: 'not_found' });
	assert.notEqual(view().page.values[0].sources[0].refusal, '', 'not found: said in place');
	view().page.values[0].sources[0].open();
	await c.fail(c.of('open_source')[1], { reason: 'could_not_open', said: 'no app opens this kind of file' });
	assert.match(view().page.values[0].sources[0].refusal, /no app opens this kind of file/);
	view().page.values[0].sources[0].open();
	await c.reply(c.of('open_source')[2], null);
	assert.equal(view().page.values[0].sources[0].refusal, '', 'opened: nothing to say');
	view().page.values[0].toggle();
	assert.equal(view().page.values[0].open, false);
});
