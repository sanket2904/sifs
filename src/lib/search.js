/**
 * The search box's searching: one loop that makes what is shown catch up
 * with what is wanted, whatever happens to any one reply.
 *
 * WANTED is the query in the box and `version`, which goes up on every
 * change that could alter its answer: a new query, an index change, files
 * read. SHOWN is the rows on screen and the rank they answer: their
 * version times two, plus one for names and contents together. After every
 * event `step` starts what is due, and the loop rests once SHOWN answers
 * WANTED with names and contents. `invoke` and the clock are passed in, so
 * the loop runs without a window.
 */

/** Names are searched once typing pauses this long. */
export const TYPING = 60;
/** Names with contents are searched this long after the names answer. */
export const CONTENTS = 120;
/** Files read refresh the search at most this often. */
export const READ_WAIT = 2000;
/** A search with no reply by then is given up: five times search_all's own bound (extract::find::CONTENT_TIME). */
export const ATTEMPT = 2000;
/**
 * The wait before searching again for the same version after a search is
 * given up, by how many were given up since the last reply.
 */
export const RETRY = [4000, 8000, 16000, 30000];
/** A newer version is searched at once only while fewer searches than this have no reply. */
export const OPEN_MAX = 3;
/** index::query::MAX_BYTES + 1: the query sent is bounded, and the backend still sees that it was cut. */
const SENT_CHARS = 4097;

/** @typedef {{ namesSlow: boolean, namesCapped: boolean, contentsSlow: boolean, contentsCapped: boolean, wordsLeftOut: boolean }} Flags */
/**
 * What the page renders. `pending`: names and contents have not answered
 * this query yet. `searching`: nothing has, and another query's rows were
 * taken down.
 * @typedef {{ rows: any[], flags: Flags, error: string, ms: number | null, pending: boolean, searching: boolean }} View
 */
/** @typedef {{ epoch: number, rank: number, rows: any[], flags: Flags, error: string, ms: number | null }} Shown */
/** @typedef {{ kind: 'names' | 'all', epoch: number, version: number, started: number, from: number }} Attempt */
/** @typedef {{ rows: any[], flags: Flags } | { error: string }} Answer */

/** @type {Flags} */
const NONE = { namesSlow: false, namesCapped: false, contentsSlow: false, contentsCapped: false, wordsLeftOut: false };

/** @type {View} */
export const BLANK = { rows: [], flags: NONE, error: '', ms: null, pending: false, searching: false };

/**
 * At most SENT_CHARS UTF-16 units of text, or one more rather than half
 * of a character: a lone surrogate can't cross IPC.
 * @param {string} text
 */
export function bounded(text) {
	let end = Math.min(text.length, SENT_CHARS);
	const last = text.charCodeAt(end - 1);
	if (end < text.length && last >= 0xd800 && last <= 0xdbff) end++;
	return text.slice(0, end);
}

/** @param {any} found a search_all reply */
function flagsOf(found) {
	return {
		namesSlow: !!found.names_timed_out,
		namesCapped: !!found.names_capped,
		contentsSlow: !!found.contents_timed_out,
		contentsCapped: !!found.contents_capped,
		wordsLeftOut: !!found.words_left_out
	};
}

/**
 * @param {{
 *   invoke: (command: string, args: { query: string, limit: number }) => Promise<any>,
 *   now: () => number,
 *   setTimer: (run: () => void, ms: number) => unknown,
 *   clearTimer: (id: unknown) => void,
 *   show: (view: View) => void
 * }} io
 */
export function createSearch({ invoke, now, setTimer, clearTimer, show }) {
	let stopped = false;
	let version = 0;
	// WANTED: the query, and the version at which the box last changed (its epoch).
	let query = '';
	let sent = '';
	let epoch = 0;
	let since = 0;
	/** @type {Shown} */
	let shown = { epoch: 0, rank: -1, rows: [], flags: NONE, error: '', ms: null };
	// This query's searches. Names go once; names with contents until SHOWN answers WANTED.
	/** @type {'none' | 'due' | 'running' | 'done'} */
	let names = 'none';
	let namesStarted = 0;
	/** @type {Attempt | null} The newest names-and-contents search, until its reply or ATTEMPT. */
	let running = null;
	let newest = -1;
	let open = 0;
	let contentsAt = Infinity;
	let lost = 0;
	let retryAt = 0;
	/** @type {number | null} */
	let readAt = null;
	/** @type {Map<string, { at: number, id: unknown }>} */
	const timers = new Map();

	const answered = () => shown.epoch === epoch && shown.rank === version * 2 + 1;

	/** @param {Shown} next */
	function render(next) {
		shown = next;
		const mine = shown.epoch === epoch && !!query;
		const { rows, flags, error, ms } = shown;
		show({ rows, flags, error, ms, pending: !!query && !(mine && shown.rank % 2 === 1), searching: mine && shown.rank < 0 });
	}

	/** When the next names-and-contents search may start, or null if none is due. */
	function gateAt() {
		if (running || names === 'due' || names === 'none' || answered()) return null;
		const at = newest < version && open < OPEN_MAX ? contentsAt : Math.max(contentsAt, retryAt);
		return Number.isFinite(at) ? at : null;
	}

	/** @param {number} t */
	function bump(t) {
		version++;
		if (names === 'running' || names === 'done') contentsAt = Math.min(contentsAt, t);
	}

	/** @param {number} t */
	function giveUp(t) {
		lost++;
		retryAt = t + RETRY[Math.min(lost, RETRY.length) - 1];
	}

	/**
	 * @param {'search_files' | 'search_all'} command
	 * @param {Attempt} a
	 */
	function send(command, a) {
		newest = a.version;
		open++;
		invoke(command, { query: sent, limit: 60 }).then(
			(found) => reply(a, a.kind === 'all' ? { rows: found.hits, flags: flagsOf(found) } : { rows: found, flags: NONE }),
			(e) => reply(a, { error: String(e) })
		);
	}

	/**
	 * Any reply proves the backend answers. It changes the screen only if it
	 * is for this query, for the newest version searched, and ranks above
	 * what is shown.
	 * @param {Attempt} a
	 * @param {Answer} answer
	 */
	function reply(a, answer) {
		if (stopped || a.epoch !== epoch) return;
		const t = now();
		open--;
		lost = 0;
		if (running === a) running = null;
		if (a.kind === 'names' && names === 'running') {
			names = 'done';
			contentsAt = t + CONTENTS;
		}
		const rank = a.version * 2 + (a.kind === 'all' ? 1 : 0);
		const mine = shown.epoch === epoch;
		if (a.version >= newest && rank > (mine ? shown.rank : -1)) {
			if ('error' in answer) {
				render(mine ? { ...shown, rank, error: answer.error } : { epoch, rank, rows: [], flags: NONE, error: answer.error, ms: null });
			} else {
				render({ epoch, rank, rows: answer.rows, flags: answer.flags, error: '', ms: Math.round(t - a.from) });
			}
		}
		step();
	}

	function step() {
		if (stopped || !query) return sync(0);
		const t = now();
		if (readAt !== null && t >= readAt) {
			readAt = null;
			bump(t);
		}
		if (names === 'due' && t >= since + TYPING) {
			names = 'running';
			namesStarted = t;
			send('search_files', { kind: 'names', epoch, version, started: t, from: t });
		}
		if (running && t >= running.started + ATTEMPT) {
			running = null;
			giveUp(t);
		}
		const bound = since + TYPING + ATTEMPT;
		if (names === 'running' && t >= bound) {
			names = 'done';
			contentsAt = t;
			giveUp(t);
		}
		if (shown.epoch !== epoch && t >= bound) render({ epoch, rank: -1, rows: [], flags: NONE, error: '', ms: null });
		const gate = gateAt();
		if (gate !== null && t >= gate) {
			if (names === 'running') names = 'done';
			const from = shown.epoch === epoch && shown.rank % 2 === 1 ? t : namesStarted;
			running = { kind: 'all', epoch, version, started: t, from };
			send('search_all', running);
		}
		sync(t);
	}

	/** Arms one timer per deadline still ahead, and clears every other. @param {number} t */
	function sync(t) {
		/** @type {Map<string, number>} */
		const due = new Map();
		if (!stopped && query) {
			if (names === 'due') due.set('typing', since + TYPING);
			if (names === 'running' || shown.epoch !== epoch) due.set('bound', since + TYPING + ATTEMPT);
			if (running) due.set('attempt', running.started + ATTEMPT);
			if (readAt !== null) due.set('read', readAt);
			const gate = gateAt();
			if (gate !== null) due.set('gate', gate);
		}
		for (const [name, timer] of timers) {
			if (due.get(name) === timer.at) continue;
			clearTimer(timer.id);
			timers.delete(name);
		}
		for (const [name, at] of due) {
			if (timers.has(name) || at <= t) continue;
			const id = setTimer(() => {
				timers.delete(name);
				step();
			}, Math.max(0, Math.ceil(at - t)));
			timers.set(name, { at, id });
		}
	}

	return {
		/** The box changed. Same text once trimmed changes nothing. @param {string} text */
		type(text) {
			const q = text.trim();
			if (stopped || q === query) return;
			version++;
			query = q;
			sent = bounded(q);
			epoch = version;
			since = now();
			names = q ? 'due' : 'none';
			running = null;
			newest = -1;
			open = 0;
			contentsAt = Infinity;
			lost = 0;
			retryAt = 0;
			readAt = null;
			render(q ? shown : { epoch, rank: -1, rows: [], flags: NONE, error: '', ms: null });
			step();
		},
		/** The index changed: the query shown is searched again, its rows kept until the answer. */
		changed() {
			if (stopped) return;
			bump(now());
			step();
		},
		/** Files were read: like an index change, at most every READ_WAIT. */
		read() {
			if (stopped || !query || readAt !== null) return;
			readAt = now() + READ_WAIT;
			step();
		},
		/** The page is gone: every timer cleared, every reply ignored. */
		stop() {
			stopped = true;
			sync(0);
		}
	};
}
