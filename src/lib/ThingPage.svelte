<script>
	import { onMount, tick } from 'svelte';
	import { WORDS } from './things.js';

	/** @type {{ page: import('./things.js').Page, depth: number, back: () => void }} */
	let { page, depth, back } = $props();

	/** @type {HTMLElement | undefined} */
	let article = $state();
	/** @type {HTMLButtonElement | undefined} */
	let backButton = $state();

	/** What had focus when the first page opened: it has it again when the last one closes. */
	let before = /** @type {Element | null | undefined} */ (undefined);

	// A new page starts at its top, with Back focused so Tab and Esc work from there.
	$effect(() => {
		void depth;
		before ??= document.activeElement;
		if (article) article.scrollTop = 0;
		backButton?.focus();
	});
	onMount(() => () => tick().then(() => before instanceof HTMLElement && before.focus()));
</script>

<article bind:this={article}>
	<button class="back" bind:this={backButton} onclick={back}>‹ {WORDS.back}</button>
	{#if page.kind}<p class="kind">{page.kind}</p>{/if}
	<h2>{page.title}</h2>
	{#if page.subtitle}<p class="subtitle">{page.subtitle}</p>{/if}

	{#if page.state === 'loading'}
		<p class="quiet">{WORDS.loading}</p>
	{:else if page.state === 'failed'}
		<p class="quiet">{page.line}</p>
	{:else}
		{#if page.values.length}
			<h3>{WORDS.values}</h3>
			<dl>
				{#each page.values as value}
					<dt>{value.label}</dt>
					<dd>
						<span class="value">{value.value}</span>
						{#if value.status}<span class="status">{value.status}</span>{/if}
						{#if value.count}
							<button class="link" aria-expanded={value.open} onclick={value.toggle}
								>{WORDS.sources(value.count)}</button
							>
						{/if}
						{#if value.open}
							<ul>
								{#each value.sources as source}
									<li>
										<button class="link" onclick={source.open}>{source.record}</button>
										{#if source.refusal}<span class="quiet">{source.refusal}</span>{/if}
									</li>
								{/each}
							</ul>
						{/if}
					</dd>
				{/each}
			</dl>
		{/if}
		{#if page.links.length}
			<h3>{WORDS.links}</h3>
			{#each page.links as group}
				<div class="links">
					<div class="label">{group.label}</div>
					{#each group.rows as row}
						<div class="row">
							{#each row.ends as end}
								<button class="end" onclick={end.open}>
									<span class="role">{end.label}</span>
									<span class="title"
										>{end.title}{#if end.subtitle}<span class="quiet">{` · ${end.subtitle}`}</span>{/if}</span
									>
								</button>
							{/each}
							{#if row.values.length}
								<div class="attributes">
									{#each row.values as attribute}
										<span>{attribute.label}: {attribute.value}</span>
									{/each}
								</div>
							{/if}
						</div>
					{/each}
				</div>
			{/each}
		{/if}
	{/if}
</article>

<style>
	article {
		position: absolute;
		inset: 0;
		overflow: auto;
		padding: 4px 26px 20px;
		background: var(--bg);
	}
	button {
		font: inherit;
		color: var(--text);
		border: none;
		background: none;
		cursor: default;
	}
	button:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 1px;
		border-radius: 6px;
	}
	.back {
		margin: 0 0 6px -8px;
		padding: 4px 8px;
		color: var(--accent);
	}
	.kind {
		margin: 0;
		font-size: 12px;
		color: var(--muted);
	}
	h2 {
		margin: 2px 0 0;
		font-size: 20px;
		font-weight: 600;
		overflow-wrap: anywhere;
	}
	.subtitle {
		margin: 2px 0 0;
		color: var(--muted);
	}
	h3 {
		margin: 18px 0 6px;
		font-size: 11px;
		font-weight: 600;
		letter-spacing: 0.02em;
		color: var(--muted);
	}
	dl {
		display: grid;
		grid-template-columns: minmax(110px, max-content) 1fr;
		gap: 6px 16px;
		margin: 0;
	}
	dt {
		color: var(--muted);
	}
	dd {
		margin: 0;
		overflow-wrap: anywhere;
	}
	.status {
		margin-left: 6px;
		padding: 0 6px;
		font-size: 11px;
		color: var(--muted);
		border: 1px solid var(--line);
		border-radius: 6px;
	}
	.link {
		padding: 0;
		margin-left: 8px;
		font-size: 12px;
		color: var(--accent);
	}
	ul {
		list-style: none;
		margin: 4px 0 0;
		padding: 0;
		font-size: 12px;
	}
	ul .link {
		margin: 0 8px 0 0;
	}
	.quiet {
		color: var(--muted);
	}
	.links {
		margin-bottom: 10px;
	}
	.label {
		font-size: 12px;
		color: var(--muted);
		margin-bottom: 2px;
	}
	.row {
		padding: 2px 0 6px;
		border-bottom: 1px solid var(--line);
	}
	.row:last-child {
		border-bottom: none;
	}
	.end {
		display: flex;
		gap: 8px;
		width: 100%;
		padding: 5px 8px;
		margin-left: -8px;
		text-align: left;
		border-radius: 8px;
	}
	.end:hover,
	.end:focus-visible {
		background: var(--selected);
		outline: none;
	}
	.role {
		min-width: 110px;
		color: var(--muted);
	}
	.title {
		font-weight: 500;
		overflow-wrap: anywhere;
	}
	.attributes {
		display: flex;
		flex-wrap: wrap;
		gap: 2px 14px;
		font-size: 12px;
		color: var(--muted);
	}
</style>
