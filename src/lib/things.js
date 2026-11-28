/**
 * The things the library knows, beside the file search: what the words in
 * the box find, and the page of one thing with the pages behind it.
 *
 * FIND. No words: nothing is asked and there is no section. Words: after
 * the file search's own pause one question goes out, and only the answer
 * to the words still in the box is shown; until it comes, the last answer
 * stays. An answer is groups, one quiet line, or nothing.
 * PAGE. A stack of pages, empty while the results show. Each is loading,
 * shown or failed. Opening a thing pushes its page, or goes back to it when
 * it is already on the stack, so the stack never holds a thing twice. Back
 * pops. An answer counts only for a page still on the stack.
 * SOURCES. On a page, each value's sources are closed or open, and each
 * source has no refusal or one line saying why its file did not open.
 *
 * What is shown is the reader's JSON as it gave it, in its order, with no
 * name of any type, attribute or link known here. `invoke` and the timers
 * are passed in, so this runs without a window.
 */
import { TYPING, bounded } from './search.js';

/** Rows a group shows before "show all". */
export const SHOWN = 5;

/** Every word the screen adds to what the reader says. */
export const WORDS = {
	tookTooLong: 'Looking this up took too long.',
	couldNotAnswer: (/** @type {string} */ said) => `Could not look this up: ${said}`,
	pageFailed: 'This page could not be read.',
	notFound: 'This file is no longer in the index.',
	couldNotOpen: (/** @type {string} */ said) => `Could not open it: ${said}`,
	loading: 'Reading…',
	back: 'Back',
	showAll: (/** @type {number} */ n) => `Show all ${n}`,
	sources: (/** @type {number} */ n) => `from ${n} ${n === 1 ? 'file' : 'files'}`,
	values: 'What the files say',
	links: 'Linked to'
};

/** The reader's status for a value nothing disputes; any other is shown. */
const ACCEPTED = 'accepted';
/** Reasons the reader is not there to ask, which show nothing at all. */
const SILENT = new Set(['no_tools', 'nothing_read_yet', 'quitting']);

/**
 * @typedef {{ id: string, title: string, subtitle: string, matched: string, open: () => void }} Row
 * @typedef {{ collection: string, label: string, count: number, more: boolean, rows: Row[], showAll: () => void }} Group
 * @typedef {{ record: string, refusal: string, open: () => void }} Source
 * @typedef {{ label: string, value: string, status: string, count: number, open: boolean, toggle: () => void, sources: Source[] }} Value
 * @typedef {{ label: string, id: string, title: string, subtitle: string, open: () => void }} End
 * @typedef {{ label: string, rows: { ends: End[], values: { label: string, value: string }[] }[] }} Links
 * @typedef {{ state: 'loading' | 'shown' | 'failed', line: string, kind: string, title: string, subtitle: string, values: Value[], links: Links[] }} Page
 * @typedef {{ groups: Group[], line: string, page: Page | null, depth: number }} View
 * @typedef {{ id: string, seen: { title: string, subtitle: string }, state: Page['state'], answer: any, line: string, open: Set<number>, refused: Map<string, string> }} Entry
 */

/** @type {View} */
export const NOTHING = { groups: [], line: '', page: null, depth: 0 };

/** @param {{ title: string, subtitle?: string }} thing */
const subtitleOf = (thing) => (thing.subtitle && thing.subtitle !== thing.title ? thing.subtitle : '');
/** @param {string} id */
const capitalised = (id) => id.charAt(0).toUpperCase() + id.slice(1);
/** @param {string[]} values */
const joined = (values) => values.join(', ');
/** @param {any} attribute */
const labelled = (attribute) => ({ label: attribute.label, value: joined(attribute.values) });

/** The screen's words for a question with no answer; '' when it shows nothing. @param {any} e */
function noAnswer(e) {
	if (SILENT.has(e?.reason)) return '';
	if (e?.reason === 'took_too_long') return WORDS.tookTooLong;
	return WORDS.couldNotAnswer(e?.said ?? String(e));
}

/**
 * @param {{
 *   invoke: (command: string, args: Record<string, string>) => Promise<any>,
 *   now: () => number,
 *   setTimer: (run: () => void, ms: number) => unknown,
 *   clearTimer: (id: unknown) => void,
 *   show: (view: View) => void
 * }} io `now` is taken for the shape `createSearch` has; the pause needs one timer only.
 */
export function createThings({ invoke, setTimer, clearTimer, show }) {
	let stopped = false;
	let words = '';
	/** Goes up on every change of the box, so only the answer to the words in it is shown. */
	let asked = 0;
	/** @type {unknown} */
	let pause = null;
	/** @type {any[]} the reader's groups, as it gave them */
	let found = [];
	/** @type {Set<number>} groups shown whole */
	let whole = new Set();
	/** @type {Group[]} */
	let groups = [];
	let line = '';
	/** @type {Entry[]} */
	const stack = [];

	function render() {
		const top = stack.at(-1);
		show({ groups, line, page: top ? pageOf(top) : null, depth: stack.length });
	}

	/** @param {any[]} next @param {string} why */
	function answered(next, why) {
		found = next;
		whole = new Set();
		line = why;
		groups = groupsOf();
		render();
	}

	/** @returns {Group[]} */
	function groupsOf() {
		return found.map((group, i) => {
			const all = whole.has(i) || group.things.length <= SHOWN;
			return {
				collection: capitalised(group.collection),
				label: group.label,
				count: group.things.length,
				more: !all,
				rows: (all ? group.things : group.things.slice(0, SHOWN)).map((/** @type {any} */ thing) => ({
					id: thing.id,
					title: thing.title,
					subtitle: subtitleOf(thing),
					matched: `${thing.matched.label}: ${thing.matched.value}`,
					open: () => open(thing.id, thing)
				})),
				showAll() {
					whole.add(i);
					groups = groupsOf();
					render();
				}
			};
		});
	}

	function ask() {
		pause = null;
		const question = asked;
		invoke('reader_find', { words: bounded(words) }).then(
			(answer) => !stopped && question === asked && answered(answer.groups, ''),
			(e) => !stopped && question === asked && answered([], noAnswer(e))
		);
	}

	/**
	 * @param {string} id
	 * @param {{ title: string, subtitle?: string }} [seen] how the thing was shown where it was opened
	 */
	function open(id, seen = { title: '' }) {
		if (stopped) return;
		const at = stack.findIndex((entry) => entry.id === id);
		if (at >= 0) {
			stack.length = at + 1;
			return render();
		}
		/** @type {Entry} */
		const entry = { id, seen: { title: seen.title, subtitle: subtitleOf(seen) }, state: 'loading', answer: null, line: '', open: new Set(), refused: new Map() };
		stack.push(entry);
		invoke('reader_page', { id }).then(
			(answer) => settle(entry, { state: 'shown', answer }),
			(e) => settle(entry, { state: 'failed', line: noAnswer(e) || WORDS.pageFailed })
		);
		render();
	}

	/** @param {Entry} entry @param {Partial<Entry>} next */
	function settle(entry, next) {
		if (stopped || !stack.includes(entry)) return;
		Object.assign(entry, next);
		render();
	}

	/** @param {Entry} entry @param {string} record */
	function openSource(entry, record) {
		invoke('open_source', { record }).then(
			() => refuse(entry, record, ''),
			(e) => refuse(entry, record, e?.reason === 'not_found' ? WORDS.notFound : WORDS.couldNotOpen(e?.said ?? String(e)))
		);
	}

	/** @param {Entry} entry @param {string} record @param {string} why */
	function refuse(entry, record, why) {
		if (why) entry.refused.set(record, why);
		else entry.refused.delete(record);
		settle(entry, {});
	}

	/** @param {Entry} entry @returns {Page} */
	function pageOf(entry) {
		const { answer: page } = entry;
		if (entry.state !== 'shown') {
			return { state: entry.state, line: entry.line, kind: '', ...entry.seen, values: [], links: [] };
		}
		return {
			state: 'shown',
			line: '',
			kind: page.label,
			title: page.title,
			subtitle: subtitleOf(page),
			values: page.values.map((/** @type {any} */ value, /** @type {number} */ i) => ({
				...labelled(value),
				status: value.status === ACCEPTED ? '' : value.status,
				count: value.sources.length,
				open: entry.open.has(i),
				toggle() {
					if (!entry.open.delete(i)) entry.open.add(i);
					render();
				},
				sources: value.sources.map((/** @type {string} */ record) => ({
					record,
					refusal: entry.refused.get(record) ?? '',
					open: () => openSource(entry, record)
				}))
			})),
			links: page.links.map((/** @type {any} */ link) => ({
				label: link.label,
				rows: link.rows.map((/** @type {any} */ row) => ({
					ends: row.ends.map((/** @type {any} */ end) => ({
						label: end.label,
						id: end.thing.id,
						title: end.thing.title,
						subtitle: subtitleOf(end.thing),
						open: () => open(end.thing.id, end.thing)
					})),
					values: row.attributes.map(labelled)
				}))
			}))
		};
	}

	return {
		/** The box changed. Same text once trimmed changes nothing. @param {string} text */
		type(text) {
			const next = text.trim();
			if (stopped || next === words) return;
			words = next;
			asked++;
			if (pause !== null) clearTimer(pause);
			pause = null;
			if (!words) return answered([], '');
			pause = setTimer(ask, TYPING);
		},
		/** Opens the page of the thing with this id. @param {string} id */
		open: (/** @type {string} */ id) => open(id),
		/** The page before this one, or the results as they were left. */
		back() {
			if (stopped || !stack.length) return;
			stack.pop();
			render();
		},
		/** The page is gone: the timer cleared, every answer ignored. */
		stop() {
			stopped = true;
			if (pause !== null) clearTimer(pause);
			pause = null;
		}
	};
}
