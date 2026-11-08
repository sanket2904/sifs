<script>
	import { invoke } from '@tauri-apps/api/core';
	import { listen } from '@tauri-apps/api/event';
	import { open as pickFolder } from '@tauri-apps/plugin-dialog';
	import { onMount, tick } from 'svelte';

	let query = $state('');
	let results = $state([]);
	let selected = $state(0);
	/** @type {number | null} */
	let searchMs = $state(null);
	let settings = $state(null);
	let showSettings = $state(false);
	let progress = $state({});
	let ignoreText = $state('');
	/** @type {{ read: number, empty: number, waiting: number, failed: number, skipped: number } | null} */
	let contents = $state(null);
	/** @type {{ path: string, status: string, error: string }[] | null} */
	let failures = $state(null);
	let error = $state('');
	/** The current search's error, apart from other operations' errors. */
	let searchError = $state('');
	let namesSlow = $state(false);
	let namesCapped = $state(false);
	let contentsSlow = $state(false);
	let contentsCapped = $state(false);
	let wordsLeftOut = $state(false);
	/** Names are shown and contents not yet: an empty list is not "nothing found" yet. */
	let contentsPending = $state(false);
	let input;
	let timer;
	/** @type {ReturnType<typeof setTimeout> | undefined} */
	let contentTimer;
	/** @type {ReturnType<typeof setTimeout> | undefined} */
	let refreshTimer;
	let latest = 0;
	/** index::query::MAX_BYTES + 1: the query sent is bounded, and the backend still sees that it was cut. */
	const SENT_CHARS = 4097;
	/** What the last search was for. */
	let searched = '';
	/**
	 * The file the user moved the selection to, followed while the query
	 * stays the same; until then the first row is selected.
	 * @type {string | null}
	 */
	let chosen = null;

	/** Drops every pending search: its timers, and its replies still to come. */
	function cancel() {
		latest++;
		clearTimeout(timer);
		clearTimeout(contentTimer);
	}

	async function search() {
		cancel();
		const id = latest;
		const q = query.trim().slice(0, SENT_CHARS);
		searched = query.trim();
		if (!q) {
			clear();
			return;
		}
		const started = performance.now();
		contentsPending = true;
		try {
			const hits = await invoke('search_files', { query: q, limit: 60 });
			if (id !== latest) return;
			show(hits);
			setFlags({});
			searchError = '';
			searchMs = Math.round(performance.now() - started);
		} catch (e) {
			if (id !== latest) return;
			searchError = String(e);
		}
		// Names show at once; names and contents together replace them once
		// typing pauses.
		contentTimer = setTimeout(() => searchAll(id, q, started), 120);
	}

	/**
	 * @param {number} id
	 * @param {string} q
	 * @param {number} started
	 */
	async function searchAll(id, q, started) {
		if (id !== latest) return;
		try {
			const found = await invoke('search_all', { query: q.slice(0, SENT_CHARS), limit: 60 });
			if (id !== latest) return;
			show(found.hits);
			setFlags(found);
			searchError = '';
			searchMs = Math.round(performance.now() - started);
		} catch (e) {
			if (id !== latest) return;
			searchError = String(e);
		}
		contentsPending = false;
	}

	/**
	 * What a search_all reply says it left out; {} for none.
	 * @param {{ names_timed_out?: boolean, names_capped?: boolean, contents_timed_out?: boolean, contents_capped?: boolean, words_left_out?: boolean }} found
	 */
	function setFlags(found) {
		namesSlow = !!found.names_timed_out;
		namesCapped = !!found.names_capped;
		contentsSlow = !!found.contents_timed_out;
		contentsCapped = !!found.contents_capped;
		wordsLeftOut = !!found.words_left_out;
	}

	/** Searches names and contents again, for files read inside since: at most every 2 s. */
	function refreshSoon() {
		if (refreshTimer !== undefined) return;
		refreshTimer = setTimeout(() => {
			refreshTimer = undefined;
			// Not while the user is typing: that search will see them.
			if (!searched || searched !== query.trim()) return;
			clearTimeout(contentTimer);
			searchAll(++latest, searched, performance.now());
		}, 2000);
	}

	function clear() {
		results = [];
		searchMs = null;
		searchError = '';
		contentsPending = false;
		setFlags({});
	}

	/** @param {any[]} hits */
	function show(hits) {
		results = hits;
		const at = chosen === null ? -1 : hits.findIndex((hit) => hit.path === chosen);
		selected = Math.max(at, 0);
		reveal();
	}

	/** @param {number} to */
	function move(to) {
		selected = Math.max(0, Math.min(to, results.length - 1));
		chosen = results[selected]?.path ?? null;
	}

	async function reveal() {
		await tick();
		document.getElementById(`hit-${selected}`)?.scrollIntoView({ block: 'nearest' });
	}

	function onInput() {
		cancel();
		chosen = null;
		timer = setTimeout(search, 60);
	}

	async function openHit(hit, reveal = false) {
		try {
			await invoke(reveal ? 'reveal_path' : 'open_path', { path: hit.path });
		} catch (e) {
			error = String(e);
		}
	}

	async function onKey(e) {
		if (e.key === 'ArrowDown') {
			move(selected + 1);
			e.preventDefault();
		} else if (e.key === 'ArrowUp') {
			move(selected - 1);
			e.preventDefault();
		} else if (e.key === 'Enter' && results[selected]) {
			openHit(results[selected], e.metaKey);
		} else if (e.key === 'Escape') {
			cancel();
			chosen = null;
			query = '';
			clear();
		}
		await reveal();
	}

	async function loadSettings() {
		/** @type {any} */
		const loaded = await invoke('settings');
		settings = loaded;
		contents = loaded.contents;
		ignoreText = settings.ignore.join('\n');
	}

	async function toggleFailures() {
		failures = failures ? null : await invoke('content_failures', { limit: 200 });
	}

	async function addFolder(path) {
		error = '';
		const chosen = path ?? (await pickFolder({ directory: true, multiple: false }));
		if (!chosen) return;
		try {
			await invoke('add_folder', { path: chosen });
		} catch (e) {
			error = String(e);
		}
		await loadSettings();
	}

	async function removeFolder(path) {
		await invoke('remove_folder', { path });
		await loadSettings();
		search();
	}

	async function saveIgnore() {
		const patterns = ignoreText.split('\n').map((p) => p.trim()).filter(Boolean);
		try {
			await invoke('set_ignore', { patterns });
			error = '';
		} catch (e) {
			error = String(e);
		}
		await loadSettings();
	}

	const folderOf = (hit) => hit.path.slice(0, hit.path.length - hit.name.length - 1).replace(home(), '~');
	const home = () => settings?.suggested?.[0]?.replace(/\/[^/]+$/, '') ?? '';
	const when = (secs) => new Date(secs * 1000).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
	function size(bytes) {
		if (bytes < 1024) return `${bytes} B`;
		const units = ['KB', 'MB', 'GB', 'TB'];
		let v = bytes / 1024;
		let i = 0;
		while (v >= 1024 && i < units.length - 1) {
			v /= 1024;
			i++;
		}
		return `${v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
	}
	function kind(hit) {
		if (hit.is_dir) return '📁';
		const e = hit.ext;
		if (['pdf'].includes(e)) return '📕';
		if (['doc', 'docx', 'pages', 'rtf', 'txt', 'md'].includes(e)) return '📄';
		if (['xls', 'xlsx', 'numbers', 'csv'].includes(e)) return '📊';
		if (['ppt', 'pptx', 'key'].includes(e)) return '📽️';
		if (['jpg', 'jpeg', 'png', 'heic', 'gif', 'webp', 'tiff'].includes(e)) return '🖼️';
		if (['mp4', 'mov', 'mkv', 'avi'].includes(e)) return '🎬';
		if (['mp3', 'm4a', 'wav', 'flac'].includes(e)) return '🎵';
		if (['zip', 'rar', '7z', 'gz', 'tar'].includes(e)) return '🗜️';
		if (e === 'app') return '🚀';
		return '📎';
	}

	onMount(() => {
		input?.focus();
		loadSettings().then(() => {
			if (settings.folders.length === 0) showSettings = true;
		});
		const unlisten = [
			listen('scan-progress', (e) => (progress = { ...progress, [e.payload.root]: e.payload.indexed })),
			listen('scan-done', (e) => {
				const { [e.payload.root]: _, ...rest } = progress;
				progress = rest;
				loadSettings();
				search();
			}),
			listen('scan-error', (e) => (error = e.payload)),
			listen('extract-progress', (e) => {
				contents = e.payload;
				refreshSoon();
			}),
			listen('index-changed', () => query && search())
		];
		return () => {
			cancel();
			clearTimeout(refreshTimer);
			unlisten.forEach((p) => p.then((f) => f()));
		};
	});
</script>

<main>
	<header>
		<input
			bind:this={input}
			bind:value={query}
			oninput={onInput}
			onkeydown={onKey}
			placeholder="Search your files"
			spellcheck="false"
			autocomplete="off"
		/>
		<button class="icon" title="Folders and settings" onclick={() => (showSettings = !showSettings)}>⚙︎</button>
	</header>

	{#if Object.keys(progress).length}
		<div class="progress">
			{#each Object.entries(progress) as [root, n]}
				<span>Indexing {root.replace(home(), '~')}: {n.toLocaleString()} files</span>
			{/each}
		</div>
	{/if}
	{#if error}<div class="error">{error}</div>{/if}
	{#if searchError}<div class="error">{searchError}</div>{/if}

	{#if showSettings && settings}
		<section class="settings">
			<h2>Folders SIFS searches</h2>
			{#each settings.folders as f}
				<div class="folder">
					<span class="path">{f.path.replace(home(), '~')}</span>
					<span class="muted">{f.scanning ? 'indexing…' : `${f.files.toLocaleString()} items`}</span>
					<button onclick={() => removeFolder(f.path)}>Remove</button>
				</div>
			{:else}
				<p class="muted">No folders yet. Add the ones you want to search.</p>
			{/each}
			<div class="row">
				<button onclick={() => addFolder()}>Add a folder…</button>
				{#each settings.suggested.filter((s) => !settings.folders.some((f) => f.path === s)) as s}
					<button class="ghost" onclick={() => addFolder(s)}>+ {s.replace(home(), '~')}</button>
				{/each}
			</div>
			<h2>Skipped</h2>
			<p class="muted">One pattern per line. <code>~</code> is your home folder; <code>**/</code> matches any folder.</p>
			<textarea bind:value={ignoreText} rows="7" spellcheck="false"></textarea>
			<div class="row"><button onclick={saveIgnore}>Save and re-index</button></div>
			<p class="muted">{settings.total.toLocaleString()} items indexed in total.</p>
			{#if contents}
				<p class="muted">
					Reading inside files: {(contents.read + contents.empty).toLocaleString()} read ·
					{contents.waiting.toLocaleString()} waiting · {contents.failed.toLocaleString()} couldn’t be read ·
					{contents.skipped.toLocaleString()} skipped
					{#if contents.failed + contents.skipped > 0}
						<button class="link" onclick={toggleFailures}>{failures ? 'hide' : 'show'}</button>
					{/if}
				</p>
				{#if failures}
					<ul class="failures">
						{#each failures as f}
							<li>
								<span class="path">{f.path.replace(home(), '~')}</span>
								<span class="muted">{f.status === 'failed' ? 'couldn’t be read' : 'skipped'}: {f.error}</span>
							</li>
						{/each}
					</ul>
				{/if}
			{/if}
		</section>
	{/if}

	<ul class="results">
		{#each results as hit, i}
			<!-- svelte-ignore a11y_click_events_have_key_events -->
			<li
				id="hit-{i}"
				class:selected={i === selected}
				onclick={() => move(i)}
				ondblclick={() => openHit(hit)}
				role="option"
				aria-selected={i === selected}
			>
				<span class="kind">{kind(hit)}</span>
				<span class="name">{hit.name}{#if hit.fuzzy}<span class="muted"> · close match</span>{/if}</span>
				<span class="where">{folderOf(hit)}</span>
				<span class="meta">{hit.is_dir ? '' : size(hit.size)} · {when(hit.modified)}</span>
				{#if hit.found}
					<span class="found"
						>{#if hit.found.label}<span class="label">{hit.found.label}</span>{' · '}{/if}{#each hit.found.snippet as piece}{#if piece.hit}<b
								>{piece.text}</b
							>{:else}{piece.text}{/if}{/each}</span
					>
				{/if}
			</li>
		{/each}
	</ul>
	{#if query && results.length === 0 && searchMs !== null && !contentsPending}
		<p class="muted center">Nothing found for “{query}”.</p>
	{/if}
	<footer class="muted">
		{#if searchMs !== null}{results.length} results in {searchMs} ms{' · '}{/if}{#if namesSlow}names took too long{' · '}{/if}{#if namesCapped}many names match: the best 5,000 compared{' · '}{/if}{#if contentsSlow}contents took too long{' · '}{/if}{#if contentsCapped}very common words: only some files' contents searched{' · '}{/if}{#if searchMs !== null && contents?.waiting}still reading {contents.waiting.toLocaleString()} files{' · '}{/if}{#if wordsLeftOut}too many words, some not searched{' · '}{/if}↑↓ to move · Enter to open · ⌘Enter to show in Finder
	</footer>
</main>

<style>
	/* Follows the system's light or dark appearance. */
	:global(:root) {
		color-scheme: light dark;
		--bg: #fbfbfd;
		--panel: #ffffff;
		--text: #1d1d1f;
		--muted: #6e6e73;
		--border: #d2d2d7;
		--line: #e5e5ea;
		--accent: #0071e3;
		--selected: #e8f0fe;
		--error: #c00;
	}
	@media (prefers-color-scheme: dark) {
		:global(:root) {
			--bg: #1c1c1e;
			--panel: #2c2c2e;
			--text: #f5f5f7;
			--muted: #a1a1a6;
			--border: #48484a;
			--line: #38383a;
			--accent: #0a84ff;
			--selected: #0a84ff33;
			--error: #ff6961;
		}
	}
	:global(body) {
		margin: 0;
		font: 14px -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
		color: var(--text);
		background: var(--bg);
	}
	main {
		display: flex;
		flex-direction: column;
		height: 100vh;
	}
	header {
		display: flex;
		gap: 8px;
		padding: 14px 16px 10px;
	}
	header input {
		flex: 1;
		font-size: 18px;
		padding: 10px 14px;
		color: var(--text);
		background: var(--panel);
		border: 1px solid var(--border);
		border-radius: 10px;
		outline: none;
	}
	header input:focus {
		border-color: var(--accent);
		box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 25%, transparent);
	}
	button {
		font: inherit;
		color: var(--text);
		padding: 6px 12px;
		border: 1px solid var(--border);
		border-radius: 8px;
		background: var(--panel);
		cursor: pointer;
	}
	button.icon {
		font-size: 18px;
	}
	button.ghost {
		border-style: dashed;
	}
	button.link {
		padding: 0;
		border: none;
		background: none;
		color: var(--accent);
	}
	.failures {
		list-style: none;
		margin: 0;
		padding: 0;
		font-size: 12px;
		max-height: 30vh;
		overflow: auto;
	}
	.failures li {
		display: flex;
		gap: 10px;
		padding: 2px 0;
	}
	.failures .path {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.progress,
	.error {
		padding: 6px 16px;
		font-size: 12px;
	}
	.progress {
		color: var(--accent);
		display: flex;
		flex-direction: column;
	}
	.error {
		color: var(--error);
	}
	.settings {
		margin: 0 16px 10px;
		padding: 12px 16px;
		background: var(--panel);
		border: 1px solid var(--line);
		border-radius: 10px;
		overflow: auto;
		max-height: 60vh;
	}
	.settings h2 {
		font-size: 13px;
		margin: 8px 0;
	}
	.folder,
	.row {
		display: flex;
		align-items: center;
		gap: 10px;
		margin: 6px 0;
		flex-wrap: wrap;
	}
	.folder .path {
		flex: 1;
	}
	textarea {
		width: 100%;
		box-sizing: border-box;
		font: 12px ui-monospace, Menlo, monospace;
		color: var(--text);
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: 6px;
	}
	.found {
		grid-column: 2 / -1;
		font-size: 0.85em;
		opacity: 0.75;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.found b {
		font-weight: 600;
	}
	.found .label {
		font-style: italic;
	}
	.results {
		list-style: none;
		margin: 0;
		padding: 0 8px;
		flex: 1;
		overflow: auto;
	}
	.results li {
		display: grid;
		grid-template-columns: 28px 1fr auto;
		grid-template-rows: auto auto;
		column-gap: 8px;
		padding: 7px 10px;
		border-radius: 8px;
		cursor: default;
	}
	.results li.selected {
		background: var(--selected);
	}
	.kind {
		grid-row: span 2;
		font-size: 20px;
		align-self: center;
	}
	.name {
		font-weight: 500;
		color: var(--text);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.where {
		grid-column: 2;
		font-size: 12px;
		color: var(--muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.meta {
		grid-column: 3;
		grid-row: 1 / span 2;
		align-self: center;
		font-size: 12px;
		color: var(--muted);
		white-space: nowrap;
	}
	.muted {
		color: var(--muted);
	}
	.center {
		text-align: center;
	}
	footer {
		padding: 8px 16px;
		font-size: 12px;
		border-top: 1px solid var(--line);
	}
</style>
