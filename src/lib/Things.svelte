<script>
	import { WORDS } from './things.js';

	/** @type {{ groups: import('./things.js').Group[], line: string }} */
	let { groups, line } = $props();
</script>

{#if line}
	<p class="line">{line}</p>
{:else if groups.length}
	<section class="things">
		{#each groups as group, i}
			{#if i === 0 || groups[i - 1].collection !== group.collection}
				<h3>{group.collection}</h3>
			{/if}
			<div class="group">
				<div class="kind">{group.label}<span class="count">{group.count}</span></div>
				{#each group.rows as row}
					<button class="thing" onclick={row.open}>
						<span class="title"
							>{row.title}{#if row.subtitle}<span class="subtitle">{` · ${row.subtitle}`}</span>{/if}</span
						>
						<span class="matched">{row.matched}</span>
					</button>
				{/each}
				{#if group.more}
					<button class="all" onclick={group.showAll}>{WORDS.showAll(group.count)}</button>
				{/if}
			</div>
		{/each}
	</section>
{/if}

<style>
	.line {
		margin: 4px 18px 8px;
		font-size: 12px;
		color: var(--muted);
	}
	.things {
		margin: 0 8px 6px;
		padding-bottom: 6px;
		border-bottom: 1px solid var(--line);
	}
	h3 {
		margin: 8px 10px 2px;
		font-size: 11px;
		font-weight: 600;
		letter-spacing: 0.02em;
		color: var(--muted);
	}
	.kind {
		margin: 4px 10px 0;
		font-size: 12px;
		color: var(--muted);
	}
	.count {
		margin-left: 6px;
		opacity: 0.8;
	}
	button {
		display: grid;
		width: 100%;
		font: inherit;
		color: var(--text);
		text-align: left;
		padding: 6px 10px;
		border: none;
		border-radius: 8px;
		background: none;
		cursor: default;
	}
	button:hover,
	button:focus-visible {
		background: var(--selected);
		outline: none;
	}
	.title {
		font-weight: 500;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.subtitle,
	.matched {
		font-weight: 400;
		color: var(--muted);
	}
	.matched {
		font-size: 12px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.all {
		display: inline-block;
		width: auto;
		padding: 2px 10px;
		font-size: 12px;
		color: var(--accent);
	}
</style>
