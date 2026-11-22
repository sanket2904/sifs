// @ts-nocheck: run by node --test; the project has no Node types, and adding them is a new dependency.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { setImmediate as turn } from 'node:timers/promises';
import { createSearch, TYPING, CONTENTS, ATTEMPT, READ_WAIT, OPEN_MAX } from '../src/lib/search.js';

/** A row named after the query and what the backend knew when asked. */
const row = (name) => ({ path: `/docs/${name}`, name });

/**
 * The module against a fake clock and a fake backend. `answer` decides each
 * call's reply: undefined keeps it for the test to settle by hand, and
 * `{ after, value | error }` settles it on the clock.
 */
function harness({ answer = () => undefined } = {}) {
	let t = 0;
	let nextId = 0;
	/** @type {Map<number, { at: number, run: () => void, order: number }>} */
	const timers = new Map();
	const calls = [];
	const views = [];
	const search = createSearch({
		now: () => t,
		setTimer(run, ms) {
			const id = ++nextId;
			timers.set(id, { at: t + ms, run, order: id });
			return id;
		},
		clearTimer: (id) => timers.delete(id),
		invoke(command, args) {
			return new Promise((resolve, reject) => {
				const call = { command, query: args.query, at: t, done: false, resolve, reject };
				calls.push(call);
				const plan = answer(call, calls.length - 1);
				if (plan) timers.set(++nextId, { at: t + plan.after, order: nextId, run: () => settle(call, plan) });
			});
		},
		show: (view) => views.push({ at: t, ...view })
	});
	function settle(call, plan) {
		if (call.done) return;
		call.done = true;
		if ('error' in plan) call.reject(plan.error);
		else call.resolve(plan.value);
	}
	const h = {
		search,
		calls,
		views,
		timers,
		get now() {
			return t;
		},
		view: () => views.at(-1),
		names: () => (views.at(-1)?.rows ?? []).map((r) => r.name),
		open: () => calls.filter((c) => !c.done).length,
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
		async reply(index, value) {
			settle(calls[index], { value });
			await turn();
		},
		async fail(index, error) {
			settle(calls[index], { error });
			await turn();
		},
		/** Types a query and answers its names and its names with contents. */
		async typed(q, rows) {
			search.type(q);
			await h.advance(TYPING);
			await h.reply(calls.length - 1, rows);
			await h.advance(CONTENTS);
			await h.reply(calls.length - 1, { hits: rows });
		}
	};
	return h;
}

const all = (rows, flags = {}) => ({ hits: rows, ...flags });

test('P1: the blocking sequence; another query\'s rows leave at the bound and never return', async () => {
	const h = harness();
	await h.typed('alpha', [row('alpha-a')]);
	const typedAt = h.now;
	h.search.type('beta');
	await h.advance(TYPING);
	h.search.changed();
	assert.deepEqual(h.calls.slice(2).map((c) => [c.command, c.query]), [['search_files', 'beta'], ['search_all', 'beta']]);
	await h.advance(typedAt + TYPING + ATTEMPT - 1 - h.now);
	assert.deepEqual(h.names(), ['alpha-a'], 'the old rows may stay while the first search is pending');
	await h.advance(1);
	assert.deepEqual(h.names(), [], 'gone at the bound');
	assert.equal(h.view().searching, true, 'and the window says it is still searching');
	await h.advance(600_000);
	assert(h.views.filter((v) => v.at >= typedAt + TYPING + ATTEMPT).every((v) => !v.rows.some((r) => r.name === 'alpha-a')), 'never back');
});

test('P1: against a backend that never answers, the old rows leave at the bound', async () => {
	const h = harness();
	await h.typed('alpha', [row('alpha-a')]);
	const typedAt = h.now;
	h.search.type('beta');
	await h.advance(TYPING + ATTEMPT - 1);
	assert.deepEqual(h.names(), ['alpha-a']);
	await h.advance(1);
	assert.deepEqual(h.names(), []);
	assert.equal(h.view().searching, true);
	await h.advance(600_000);
	assert.deepEqual(h.names(), []);
	assert.equal(h.views.at(-1).at, typedAt + TYPING + ATTEMPT, 'nothing renders after');
});

test('P1: against a backend that answers in 5 s, the old rows leave at the bound, then the answer shows', async () => {
	const h = harness({ answer: (c, i) => (i >= 2 ? { after: 5000, value: c.command === 'search_all' ? all([row('beta-b')]) : [row('beta-b')] } : undefined) });
	await h.typed('alpha', [row('alpha-a')]);
	h.search.type('beta');
	await h.advance(TYPING + ATTEMPT);
	assert.deepEqual(h.names(), []);
	await h.advance(5000 - ATTEMPT);
	assert.deepEqual(h.names(), ['beta-b']);
});

test('P1: an error for the new query takes the old rows down', async () => {
	const h = harness();
	await h.typed('alpha', [row('alpha-a')]);
	h.search.type('beta');
	await h.advance(TYPING);
	await h.fail(2, 'names failed');
	assert.deepEqual(h.names(), []);
	assert.equal(h.view().error, 'names failed');
});

test('P1: the bound runs from when the rows stopped matching the box, however long the typing goes on', async () => {
	const h = harness();
	await h.typed('alpha', [row('alpha-a')]);
	const wrongFrom = h.now;
	for (const q of ['b', 'be', 'bet', 'beta', 'beta1']) {
		h.search.type(q);
		await h.advance(1000);
	}
	assert(h.views.filter((v) => v.at >= wrongFrom + TYPING + ATTEMPT).every((v) => v.rows.length === 0), 'no alpha rows from the bound on');
	const cleared = h.views.find((v) => v.at === wrongFrom + TYPING + ATTEMPT);
	assert.deepEqual(cleared?.rows, [], 'gone at the bound, with no reply');
	assert.equal(cleared.searching, true, 'and the window says it is still searching');
});

test('P1: a reply for the current query before the bound shows, and the bound then runs from the next edit', async () => {
	const h = harness();
	await h.typed('alpha', [row('alpha-a')]);
	h.search.type('b');
	await h.advance(1000);
	h.search.type('be');
	await h.advance(TYPING);
	await h.reply(h.calls.length - 1, [row('be-names')]);
	assert.deepEqual(h.names(), ['be-names']);
	await h.advance(500);
	const editAt = h.now;
	h.search.type('bet');
	await h.advance(TYPING + ATTEMPT - 1);
	assert.deepEqual(h.names(), ['be-names'], 'rows of the last query shown stay until their own bound');
	await h.advance(1);
	assert.equal(h.now, editAt + TYPING + ATTEMPT);
	assert.deepEqual(h.names(), []);
});

test('P1: replies of the old query in any order with the new one never put old rows back', async () => {
	for (const order of permutations([0, 1, 2])) {
		const h = harness();
		h.search.type('alpha');
		await h.advance(TYPING);
		await h.reply(0, [row('alpha-a')]);
		await h.advance(CONTENTS);
		h.search.type('beta');
		await h.advance(TYPING);
		h.search.changed();
		// calls: 1 alpha all, 2 beta names, 3 beta all
		const replies = [() => h.reply(1, all([row('alpha-late')])), () => h.reply(2, [row('beta-names')]), () => h.reply(3, all([row('beta-all')]))];
		for (const i of order) await replies[i]();
		assert(h.views.slice(h.views.findIndex((v) => v.at >= h.calls[2].at)).every((v) => !v.rows.some((r) => r.name.startsWith('alpha-late'))), String(order));
		assert.deepEqual(h.names(), ['beta-all'], String(order));
	}
});

test('P2: the blocking sequence; a search again for the new query, and its answer shows', async () => {
	const h = harness({ answer: (c, i) => (i >= 4 ? { after: 50, value: all([row('beta-b')]) } : undefined) });
	await h.typed('alpha', [row('alpha-a')]);
	h.search.type('beta');
	await h.advance(TYPING);
	h.search.changed();
	await h.advance(10_000);
	assert.deepEqual(h.names(), ['beta-b']);
	assert.equal(h.view().pending, false);
	const quiet = h.calls.length;
	await h.advance(600_000);
	assert.equal(h.calls.length, quiet, 'and it rests once answered');
});

test('P2: a lost reply to the last change is searched again with no later event', async () => {
	const h = harness({ answer: (c, i) => (i >= 3 ? { after: 80, value: all([]) } : undefined) });
	await h.typed('alpha', [row('deleted')]);
	h.search.changed();
	await h.advance(60_000);
	assert.deepEqual(h.names(), []);
});

test('P2: after ten minutes of lost replies and changes, the backend answering again catches up', async () => {
	let alive = false;
	const h = harness({ answer: (c, i) => (i >= 2 && alive ? { after: 50, value: all([row('now')]) } : undefined) });
	await h.typed('alpha', [row('then')]);
	for (let i = 0; i < 600; i++) {
		h.search.changed();
		await h.advance(1000);
	}
	alive = true;
	await h.advance(32_100);
	assert.deepEqual(h.names(), ['now']);
	h.search.changed();
	await h.advance(100);
	assert.equal(h.calls.at(-1).at, h.now - 100, 'and the next change searches at once');
});

test('P2: a rejected refresh is searched again, and the deleted row goes', async () => {
	const h = harness({ answer: (c, i) => (i >= 3 ? { after: 10, value: all([]) } : undefined) });
	await h.typed('alpha', [row('deleted')]);
	h.search.changed();
	await h.advance(20);
	await h.fail(2, 'transient failure');
	assert.equal(h.view().error, 'transient failure');
	assert.deepEqual(h.names(), ['deleted'], 'rows kept with the error');
	await h.advance(600_000);
	assert.deepEqual(h.names(), []);
	assert.equal(h.view().error, '', 'the success clears the error');
	assert.equal(h.view().pending, false);
});

test('P2: a retry rejected, then the first search for the same version succeeding: the success shows', async () => {
	const h = harness();
	await h.typed('alpha', [row('old')]);
	h.search.changed();
	await h.advance(6000);
	assert.deepEqual(h.calls.slice(2).map((c) => c.at - h.calls[2].at), [0, ATTEMPT + 4000]);
	await h.advance(10);
	await h.fail(3, 'transient failure');
	await h.advance(10);
	await h.reply(2, all([row('fresh')]));
	assert.deepEqual(h.names(), ['fresh']);
	assert.equal(h.view().error, '');
});

test('P2: against a backend that answers in 5 s, the last change shows', async () => {
	let snapshot = 0;
	const h = harness({ answer: (c, i) => (i >= 2 ? { after: 5000, value: all([row(`v${snapshot}`)]) } : undefined) });
	await h.typed('alpha', [row('v0')]);
	for (let i = 0; i < 80; i++) {
		snapshot++;
		h.search.changed();
		await h.advance(250);
	}
	await h.advance(5000 + ATTEMPT);
	assert.deepEqual(h.names(), ['v80']);
	assert.equal(h.open(), 0);
});

test('P2: three overlapping searches, replies in every order, with one lost: the newest change shows', async () => {
	for (const order of permutations([0, 1, 2])) {
		for (const lostOne of [-1, 0, 1, 2]) {
			const h = harness();
			const start = await overlapping(h);
			for (const i of order) if (i !== lostOne) await h.reply(start + i, all([row(`v${i}`)]));
			await h.advance(60_000);
			const retries = h.calls.slice(start + 3).map((c, k) => start + 3 + k);
			for (const k of retries) if (!h.calls[k].done) await h.reply(k, all([row('v2')]));
			assert.deepEqual(h.names(), ['v2'], `${order} lost ${lostOne}`);
		}
	}
});

test('P3: against a backend that never answers, searches per minute follow the schedule, however many changes', async () => {
	const totals = [];
	for (const gap of [1000, 100]) {
		const h = harness();
		await h.typed('alpha', [row('a')]);
		const from = h.calls.length;
		const t0 = h.now;
		for (let i = 0; i < 600_000 / gap; i++) {
			h.search.changed();
			await h.advance(gap);
		}
		const perMinute = countPerMinute(h.calls.slice(from), t0, 10);
		totals.push(perMinute);
		assert(perMinute[0] <= 5 && perMinute.slice(1).every((n) => n <= 2), `changes every ${gap} ms: ${perMinute}`);
	}
	assert.deepEqual(totals[0], totals[1], 'ten times the changes, the same searches');
	const h = harness();
	h.search.type('alpha');
	await h.advance(600_000);
	const alone = countPerMinute(h.calls, 0, 10);
	assert(alone[0] <= 5 && alone.slice(1).every((n) => n <= 2), `a typed query alone: ${alone}`);
});

test('P3: against a backend that always rejects, searches per minute follow the same schedule, however many changes', async () => {
	const reject = (c, i) => (i >= 2 ? { after: 10, error: 'down' } : undefined);
	for (const gap of [1000, 100]) {
		const h = harness({ answer: reject });
		await h.typed('alpha', [row('a')]);
		const from = h.calls.length;
		const t0 = h.now;
		for (let i = 0; i < 600_000 / gap; i++) {
			h.search.changed();
			await h.advance(gap);
		}
		const perMinute = countPerMinute(h.calls.slice(from), t0, 10);
		assert(perMinute[0] <= 5 && perMinute.slice(1).every((n) => n <= 2), `changes every ${gap} ms: ${perMinute}`);
		assert.equal(h.view().error, 'down');
	}
	const h = harness({ answer: () => ({ after: 10, error: 'down' }) });
	h.search.type('alpha');
	await h.advance(600_000);
	const alone = countPerMinute(h.calls, 0, 10);
	assert(alone[0] <= 5 && alone.slice(1).every((n) => n <= 2), `a typed query alone: ${alone}`);
});

test('P3: against a backend that answers one search every 5 s, searches stay bounded and do not pile up', async () => {
	let free = 0;
	const h = harness({
		answer: (c, i) => {
			if (i < 2) return undefined;
			free = Math.max(free, h.now) + 5000;
			return { after: free - h.now, value: all([row('x')]) };
		}
	});
	await h.typed('alpha', [row('a')]);
	const from = h.calls.length;
	const t0 = h.now;
	let most = 0;
	for (let i = 0; i < 600; i++) {
		h.search.changed();
		await h.advance(1000);
		most = Math.max(most, h.open());
	}
	assert(most <= OPEN_MAX, `at most ${OPEN_MAX} without a reply, saw ${most}`);
	const perMinute = countPerMinute(h.calls.slice(from), t0, 10);
	assert(perMinute[0] <= OPEN_MAX + 12 && perMinute.slice(1).every((n) => n <= 12), `one per reply after the first ${OPEN_MAX}: ${perMinute}`);
	assert(perMinute.at(-1) <= perMinute[1] + 1, `steady, not growing: ${perMinute}`);
});

test('P3: against a backend whose every reply takes 5 s, at most one search per bound', async () => {
	const h = harness({ answer: (c, i) => (i >= 2 ? { after: 5000, value: all([row('x')]) } : undefined) });
	await h.typed('alpha', [row('a')]);
	const from = h.calls.length;
	const t0 = h.now;
	for (let i = 0; i < 1200; i++) {
		h.search.changed();
		await h.advance(500);
	}
	const perMinute = countPerMinute(h.calls.slice(from), t0, 10);
	assert(perMinute.every((n) => n <= 60_000 / ATTEMPT), `per minute ${perMinute}`);
});

test('P4: replies for an older query never change the screen', async () => {
	const h = harness();
	h.search.type('alpha');
	await h.advance(TYPING);
	h.search.type('beta');
	await h.advance(TYPING);
	h.search.type('alpha');
	await h.advance(TYPING);
	h.search.changed();
	await h.reply(3, all([row('current-alpha')]));
	const before = h.views.length;
	await h.reply(0, [row('first-alpha')]);
	await h.fail(1, 'beta failed');
	await h.reply(2, [row('third-alpha-names')]);
	assert.equal(h.views.length, before, 'nothing rendered');
	assert.deepEqual(h.names(), ['current-alpha']);
	assert.equal(h.view().error, '');
});

test('P4: a reply for the old query during the typing pause never changes the screen', async () => {
	const h = harness();
	h.search.type('alpha');
	await h.advance(TYPING);
	await h.reply(0, [row('alpha-names')]);
	await h.advance(CONTENTS);
	h.search.type('beta');
	const before = h.views.length;
	await h.reply(1, all([row('alpha-all')]));
	await h.advance(TYPING - 1);
	assert.equal(h.views.length, before, 'nothing rendered');
	assert.deepEqual(h.names(), ['alpha-names']);
});

test('P4: three overlapping searches of one query, replies in every order: an older version never changes the screen', async () => {
	for (const order of permutations([0, 1, 2])) {
		const h = harness();
		const start = await overlapping(h);
		const seen = [];
		for (const i of order) {
			const before = h.views.length;
			await h.reply(start + i, all([row(`v${i}`)]));
			seen.push(...h.views.slice(before).map((v) => v.rows.map((r) => r.name).join()));
		}
		assert.deepEqual(seen, ['v2'], `${order}`);
	}
	for (const order of permutations([0, 1, 2])) {
		const h = harness();
		const start = await overlapping(h);
		for (const i of order) await h.fail(start + i, `error v${i}`);
		assert.equal(h.view().error, 'error v2', `${order}`);
	}
});

test('P4: against a backend that answers in 5 s, late replies of older versions never show', async () => {
	let snapshot = 0;
	const h = harness({ answer: (c, i) => (i >= 2 ? { after: 5000, value: all([row(`v${snapshot}`)]) } : undefined) });
	await h.typed('alpha', [row('v0')]);
	for (let i = 0; i < 40; i++) {
		snapshot++;
		h.search.changed();
		await h.advance(250);
	}
	await h.advance(20_000);
	const shown = h.views.filter((v) => v.rows.length).map((v) => Number(v.rows[0].name.slice(1)));
	assert(shown.every((v, i) => i === 0 || v >= shown[i - 1]), `never back: ${shown}`);
	assert.deepEqual(h.names(), ['v40']);
});

test('P5: names after 60 ms, names with contents 120 ms after their answer, the query bounded', async () => {
	const h = harness();
	const q = 'cat '.repeat(1024) + '😀';
	h.search.type(q);
	await h.advance(TYPING - 1);
	assert.equal(h.calls.length, 0);
	await h.advance(1);
	assert.deepEqual(h.calls.map((c) => c.command), ['search_files']);
	await h.reply(0, [row('name')]);
	assert.deepEqual(h.names(), ['name']);
	assert.equal(h.view().pending, true);
	await h.advance(CONTENTS - 1);
	assert.equal(h.calls.length, 1);
	await h.advance(1);
	assert.equal(h.calls[1].command, 'search_all');
	for (const c of h.calls) assert.equal(c.query.length, 4098);
	assert(h.calls.every((c) => c.query.isWellFormed()));
	await h.advance(30);
	await h.reply(1, all([row('content'), row('name')], { contents_timed_out: true, words_left_out: true }));
	assert.deepEqual(h.names(), ['content', 'name']);
	assert.deepEqual(h.view().flags, { namesSlow: false, namesCapped: false, contentsSlow: true, contentsCapped: false, wordsLeftOut: true });
	assert.equal(h.view().ms, TYPING + CONTENTS + 30 - TYPING, 'elapsed from the names search');
	assert.equal(h.view().pending, false);
});

test('P5: files read refresh at most every 2 s; changes during a refresh cause one more search; rows stay', async () => {
	const h = harness();
	await h.typed('alpha', [row('a')]);
	for (let i = 0; i < 20; i++) h.search.read();
	await h.advance(READ_WAIT - 1);
	assert.equal(h.calls.length, 2);
	await h.advance(1);
	assert.equal(h.calls.length, 3);
	for (let i = 0; i < 3; i++) h.search.changed();
	assert.equal(h.calls.length, 3, 'one at a time');
	assert.deepEqual(h.names(), ['a'], 'rows kept during the refresh');
	assert.equal(h.view().pending, false);
	await h.fail(2, 'outdated');
	assert.equal(h.view().error, 'outdated');
	assert.deepEqual(h.names(), ['a']);
	assert.equal(h.calls.length, 4, 'one more for the three changes');
	await h.reply(3, all([row('b')]));
	assert.equal(h.view().error, '');
	await h.advance(60_000);
	assert.equal(h.calls.length, 4, 'then quiet');
	h.search.changed();
	assert.equal(h.calls.length, 5, 'and the next change at once');
});

test('P5: typing, Escape and the page going drop every timer and ignore every reply', async () => {
	for (const how of ['typing', 'Escape', 'stop']) {
		const h = harness();
		await h.typed('alpha', [row('a')]);
		h.search.read();
		h.search.changed();
		h.search.changed();
		await h.advance(250);
		const from = h.calls.length;
		if (how === 'typing') h.search.type('beta');
		else if (how === 'Escape') h.search.type('');
		else h.search.stop();
		if (how === 'typing') assert.deepEqual([...h.timers.values()].map((x) => x.at - h.now).sort((a, b) => a - b), [TYPING, TYPING + ATTEMPT]);
		else assert.equal(h.timers.size, 0, how);
		const before = h.views.length;
		await h.reply(2, all([row('stale')]));
		assert.equal(h.views.length, before, `${how}: reply ignored`);
		await h.advance(TYPING);
		if (how === 'typing') assert.deepEqual(h.calls.slice(from).map((c) => c.query), ['beta']);
		else assert.equal(h.calls.length, from, how);
	}
});

test('P5: an empty box searches nothing and clears', async () => {
	const h = harness();
	await h.typed('alpha', [row('a')]);
	h.search.type('   ');
	assert.deepEqual(h.view(), { at: h.now, rows: [], flags: h.view().flags, error: '', ms: null, pending: false, searching: false });
	h.search.changed();
	h.search.read();
	await h.advance(60_000);
	assert.equal(h.calls.length, 2);
});

test('P5: the same text once trimmed searches nothing new', async () => {
	const h = harness();
	await h.typed('alpha', [row('a')]);
	h.search.type(' alpha ');
	await h.advance(60_000);
	assert.equal(h.calls.length, 2);
});

/** Three searches of one query outstanding at once, for versions in order: returns the first's index. */
async function overlapping(h) {
	await h.typed('alpha', [row('v-')]);
	const start = h.calls.length;
	h.search.changed();
	await h.advance(100);
	h.search.changed();
	await h.advance(ATTEMPT - 100);
	h.search.changed();
	await h.advance(ATTEMPT);
	assert.equal(h.calls.length, start + 3, 'three outstanding');
	return start;
}

function countPerMinute(calls, t0, minutes) {
	const counts = Array(minutes).fill(0);
	for (const c of calls) {
		const m = Math.floor((c.at - t0) / 60_000);
		if (m < minutes) counts[m]++;
	}
	return counts;
}

function permutations(xs) {
	if (xs.length <= 1) return [xs];
	return xs.flatMap((x, i) => permutations([...xs.slice(0, i), ...xs.slice(i + 1)]).map((p) => [x, ...p]));
}
