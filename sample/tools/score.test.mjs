// node --test sample/tools/score.test.mjs
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import {
	cpSync,
	existsSync,
	mkdtempSync,
	readFileSync,
	rmSync,
	symlinkSync,
	writeFileSync
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { SAMPLE } from './generate.mjs';
import { readLabels } from './lib/labels.mjs';
import { reference } from './reference.mjs';
import { FormatError, score, table } from './score.mjs';
import { checkTextSets, readTextSet } from './text.mjs';

const labels = readLabels(new URL('labels.json', SAMPLE));
const sets = { text: readTextSet('text'), 'text-ocr': readTextSet('text-ocr') };

const banks = JSON.parse(readFileSync(new URL('directory/banks.json', SAMPLE), 'utf8'));
const perfect = (directory = 'on', set = 'text') =>
	structuredClone(reference(labels, sets[set], directory, banks));
const run = (output) => score(labels, sets[output.text_set], output, banks);
const doc = (output, id) => output.documents.find((d) => d.id === id);
const totals = (result, measure) => result.measures[measure].total;
const sum = (result, outcome, skip = () => false) =>
	Object.entries(result.measures)
		.filter(([m]) => !skip(m))
		.reduce((n, [, r]) => n + r.total[outcome], 0);
const wrongs = (result, measure) => result.wrong.filter((w) => w.measure === measure);
const card = (output, local) => output.organisations[`ref:${local}`];
const byTier = (result, measure, tier) => result.measures[measure].by_tier[tier];
const linkOf = (output, id, value) => {
	const d = doc(output, id);
	const ident = d.identifiers.find((i) => i.value === value);
	return d.links.find((l) => l.identifier === ident.id);
};
// Drops the same_org rows of a document whose keys it no longer uses.
const prune = (d) => {
	const used = new Set([...d.orgs.map((m) => m.org), ...d.links.map((l) => l.org), d.statement?.issuer]);
	d.same_org = d.same_org.filter((r) => r.keys.every((k) => used.has(k)));
};
// A span of the committed text of a document, found by its text.
const spanOf = (output, id, text) => {
	const pages = sets[output.text_set].docs.get(id).pages;
	const page = pages.findIndex((p) => p.text.includes(text));
	const start = [...pages[page].text.slice(0, pages[page].text.indexOf(text))].length;
	return { page: page + 1, start, end: start + [...text].length, text };
};

// With the directory off, nothing in these documents joins the bank's name to
// its code, so the perfect output leaves the join out.
const offJoins = {
	text: ['transfer-untied', 'scan-bob-letter', 'scan-pnb-deposit'],
	'text-ocr': ['transfer-untied']
};
const leftOut = (result) => result.left_out.map((i) => `${i.measure} ${i.document}`);
const joinsLeftOut = (directory, set) =>
	directory === 'on' ? [] : offJoins[set].map((id) => `same_org ${id}`);

for (const directory of ['on', 'off']) {
	test(`the perfect output scores everything right, directory ${directory}`, () => {
		const result = run(perfect(directory));
		for (const outcome of ['wrong', 'impossible'])
			assert.equal(sum(result, outcome), 0, `${outcome}: ${JSON.stringify(result[outcome])}`);
		assert.deepEqual(leftOut(result), joinsLeftOut(directory, 'text'));
		assert.deepEqual(result.merges, []);
		assert.equal(result.failed, false);
		assert.equal(totals(result, 'links').right, 47);
		assert.deepEqual(byTier(result, 'links', 'certain'), { right: directory === 'on' ? 21 : 4, wrong: 0 });
		assert.equal(totals(result, 'same_org').right, directory === 'on' ? 17 : 14);
		assert.equal(totals(result, 'answer_latest_statement').right, 12);
		assert.equal(totals(result, 'answer_current_passport').right, 3);
	});

	test(`on the OCR text set the perfect output loses only what OCR made impossible, directory ${directory}`, () => {
		const result = run(perfect(directory, 'text-ocr'));
		assert.equal(sum(result, 'wrong'), 0, JSON.stringify(result.wrong));
		assert.deepEqual(leftOut(result), joinsLeftOut(directory, 'text-ocr'));
		assert.deepEqual(
			result.impossible.map((i) => `${i.document} ${i.measure} ${i.what.split('\n')[0]}`),
			[
				'scan-bob-letter identifier:ifsc ifsc BARB0SAMPL6',
				'scan-bob-letter links account 12340100005678 at bank-of-baroda',
				'scan-bob-letter links ifsc BARB0SAMPL6 at bank-of-baroda',
				'scan-pnb-deposit identifier:ifsc ifsc PUNB0SAMPL7',
				'scan-pnb-deposit links account 0123000100456789 at pnb',
				'scan-pnb-deposit links ifsc PUNB0SAMPL7 at pnb',
				'passport-asha-expired identifier:mrz mrz P<UTOKALPIT<<ASHA<<<<<<<<<<<<<<<<<<<<<<<<<<<'
			]
		);
		assert.equal(result.failed, false);
	});
}

test('on the OCR text set, a value the extractor repairs from the OCR text is found', () => {
	const output = perfect('on', 'text-ocr');
	const d = doc(output, 'scan-bob-letter');
	d.identifiers.push({
		id: 'i2',
		page: 1,
		start: 199,
		end: 210,
		text: 'BARBØSAMPL6',
		kind: 'ifsc',
		value: 'BARB0SAMPL6',
		valid: true,
		tier: 'certain'
	});
	output.organisations['IFSC BARB'] = { name: null, directory: 'Q2003797' };
	d.links.push({ identifier: 'i2', org: 'IFSC BARB', reasons: ['ifsc-prefix'], tier: 'certain' });
	const result = run(output);
	assert.equal(totals(result, 'identifier:ifsc').right, 12);
	assert.equal(totals(result, 'identifier:ifsc').impossible, 1);
	assert.equal(totals(result, 'links').impossible, 3);
	assert.equal(result.failed, false);
});

test('a likely link to an organisation the document does not settle is wrong, counted, and fails nothing', () => {
	const output = perfect();
	linkOf(output, 'hdfc-statement-sep', '50100417263958').org = 'ref:kotak';
	const result = run(output);
	assert.equal(totals(result, 'links').wrong, 1);
	assert.deepEqual(byTier(result, 'links', 'likely'), { right: 25, wrong: 1 });
	assert.match(wrongs(result, 'links')[0].what, /does not settle kotak/);
	assert.equal(wrongs(result, 'links')[0].tier, 'likely');
	assert.equal(totals(result, 'links').left_out, 1, 'the account is no longer linked to HDFC Bank');
	assert.equal(result.failed, false);
});

test('a wrong certain link fails the run, and one certain row makes the link certain in either order', () => {
	for (const likelyFirst of [true, false]) {
		const output = perfect();
		// The IBAN's key now claims to be ABN AMRO, so the IBAN is held at no
		// labelled organisation.
		output.organisations['IBAN GB NWBK'].directory = card(output, 'abn-amro').directory;
		const d = doc(output, 'natwest-statement');
		const iban = linkOf(output, 'natwest-statement', 'GB29NWBK60161331926819');
		d.links.splice(likelyFirst ? 0 : d.links.length, 0, { ...iban, tier: 'likely' });
		for (const r of d.same_org) r.tier = 'likely';
		const result = run(output);
		assert.deepEqual(result.merges, []);
		assert.deepEqual(byTier(result, 'links', 'certain'), { right: 20, wrong: 1 });
		assert.equal(result.failed, true, String(likelyFirst));
	}
});

test('a link the labels list under not_links is wrong', () => {
	const output = perfect();
	doc(output, 'neft-receipt-tied').links.push({
		identifier: 'i1',
		org: 'ref:icici-bank',
		reasons: ['paired-code'],
		via: 'i4',
		tier: 'likely'
	});
	const result = run(output);
	assert.equal(totals(result, 'links').wrong, 1);
	assert.match(wrongs(result, 'links')[0].what, /not_link \(code-names-other-bank\)/);
});

test('a link on an identifier the labels mark invalid is wrong', () => {
	const output = perfect();
	const d = doc(output, 'invoice-lookalike-numbers');
	const row = { identifier: 'i2', reasons: ['statement-header'], tier: 'likely' };
	d.links.push({ ...row, org: 'ref:sample-supplies' }, { ...row, org: 'ref:natwest' });
	const result = run(output);
	assert.deepEqual(
		wrongs(result, 'links')
			.map((w) => w.what)
			.sort(),
		[
			'card 4111111111111112 at ref:natwest (natwest): the identifier is invalid',
			'card 4111111111111112 at ref:sample-supplies (sample-supplies): a not_link (invalid-identifier)'
		]
	);
});

test('a labelled link given only a reason that does not hold is wrong', () => {
	const output = perfect();
	doc(output, 'transfer-rows').links[1].reasons = ['statement-header'];
	const result = run(output);
	assert.equal(totals(result, 'links').wrong, 1);
	assert.match(wrongs(result, 'links')[0].what, /no reason given holds/);
});

test('paired-code through the wrong code does not hold', () => {
	const output = perfect();
	doc(output, 'transfer-rows').links[1].via = 'i3';
	const result = run(output);
	assert.equal(totals(result, 'links').wrong, 1);
	assert.equal(totals(result, 'links').left_out, 1);
});

test('one key over two labelled organisations is a merge and fails the run', () => {
	const output = perfect();
	for (const m of doc(output, 'hdfc-ltd-loan-2019').orgs) m.org = 'ref:hdfc-bank';
	doc(output, 'hdfc-ltd-loan-2019').statement.issuer = 'ref:hdfc-bank';
	const result = run(output);
	assert.deepEqual(result.merges, [
		{ key: 'ref:hdfc-bank', why: 'organisations', organisations: ['hdfc-bank', 'hdfc-ltd'] }
	]);
	assert.ok(totals(result, 'same_org').wrong > 0, 'rows on the merged key are wrong');
	assert.equal(result.failed, true);
});

test("a link by a code's own reason to any key but the code's canonical key breaks the format, so no key holds two codes", () => {
	const moves = [
		['natwest-statement', 'NWBKGB2L', 'IBAN GB NWBK', /goes to its code's key, "BIC NWBKGB2L"/],
		['natwest-statement', 'GB29NWBK60161331926819', 'iban GB NWBK', /"IBAN GB NWBK"/],
		['hdfc-statement-sep', 'HDFC0SAMPL1', 'ifsc HDFC', /"IFSC HDFC"/],
		['hdfc-statement-sep', 'HDFC0SAMPL1', 'ref:hdfc-bank', /"IFSC HDFC"/],
		['transfer-rows', 'ICIC0SAMPL2', 'ref:hdfc-bank', /"IFSC ICIC"/]
	];
	for (const [id, value, key, message] of moves) {
		const output = perfect();
		output.organisations[key] ??= { name: null, directory: null };
		linkOf(output, id, value).org = key;
		prune(doc(output, id));
		assert.throws(() => run(output), (e) => e instanceof FormatError && message.test(e.message), key);
	}
});

// Keys a statement's printed name to its code's key, as an extractor that
// merges would.
const nameOnCode = (output, id, key) => {
	const d = doc(output, id);
	d.orgs[0].org = key;
	d.statement.issuer = key;
	prune(d);
};

test('a key holding a printed name and an institution code is a merge', () => {
	const output = perfect();
	nameOnCode(output, 'hdfc-statement-sep', 'IFSC HDFC');
	const result = run(output);
	assert.deepEqual(result.merges, [{ key: 'IFSC HDFC', why: 'name-and-code' }]);
	assert.equal(sum(result, 'wrong'), 0, JSON.stringify(result.wrong));
	assert.equal(result.failed, true);
});

test('one labelled organisation over two keys is a split, counted and not wrong', () => {
	const output = perfect();
	output.organisations['hdfc-2'] = { name: 'HDFC Bank Limited', directory: 'Q631047' };
	for (const m of doc(output, 'hdfc-branch-letter').orgs) m.org = 'hdfc-2';
	const result = run(output);
	assert.deepEqual(
		result.splits.find((s) => s.organisation === 'hdfc-bank'),
		{ organisation: 'hdfc-bank', keys: ['IFSC HDFC', 'hdfc-2', 'ref:hdfc-bank'] }
	);
	assert.equal(sum(result, 'wrong'), 0, JSON.stringify(result.wrong));
	assert.equal(result.failed, false);
});

test('a missed link is left out and does not fail the run', () => {
	const output = perfect();
	doc(output, 'neft-receipt-tied').links.splice(2, 1);
	const result = run(output);
	assert.equal(totals(result, 'links').left_out, 1);
	assert.equal(totals(result, 'links').wrong, 0);
	assert.equal(result.failed, false);
});

test('a wrong passport expiry is wrong, and a date where the labels cannot tell is wrong', () => {
	const output = perfect();
	doc(output, 'passport-rohan').passport.expiry = '2028-01-05';
	doc(output, 'passport-bad-mrz').passport.expiry = '2031-04-15';
	const result = run(output);
	assert.equal(totals(result, 'passport_expiry').wrong, 2);
	assert.equal(totals(result, 'passport_expiry').right, 2);
});

test('a passport where the labels have none is wrong', () => {
	const output = perfect();
	doc(output, 'visa-asha').passport = {
		holder: { surname: 'KALPIT', given: 'ASHA' },
		number: 'i1',
		expiry: '2027-01-31',
		current: true,
		tiers: { holder: 'likely', number: 'likely', expiry: 'likely', current: 'likely' }
	};
	const result = run(output);
	for (const m of ['passport_holder', 'passport_number', 'passport_expiry', 'passport_current'])
		assert.equal(totals(result, m).wrong, 1, m);
});

test('a wrong latest statement is a wrong likely answer and fails nothing', () => {
	const output = perfect();
	output.answers.latest_statement.find((a) => a.account === '50100417263958').document =
		'hdfc-statement-jul';
	const result = run(output);
	assert.deepEqual(byTier(result, 'answer_latest_statement', 'likely'), { right: 11, wrong: 1 });
	assert.equal(result.failed, false);
});

test('an account given by another spelling is found', () => {
	const output = perfect();
	output.answers.latest_statement.find((a) => a.document === 'natwest-statement').account =
		'31926819';
	assert.equal(totals(run(output), 'answer_latest_statement').right, 12);
});

test('a decoy reported as an identifier is wrong', () => {
	const output = perfect();
	doc(output, 'rent-receipt').identifiers.push({
		id: 'x1',
		page: 1,
		start: 283,
		end: 298,
		text: '+44 7700 900417',
		kind: 'account',
		value: '447700900417',
		valid: null,
		tier: 'likely'
	});
	const result = run(output);
	assert.equal(totals(result, 'identifier:account').wrong, 1);
	assert.match(wrongs(result, 'identifier:account')[0].what, /decoy \(phone-number\)/);
	assert.equal(result.failed, false, 'an identifier alone is not a link');
});

test('an invalid IBAN reported as valid is wrong', () => {
	const output = perfect();
	doc(output, 'rechnung-falsche-iban').identifiers[0].valid = true;
	assert.equal(totals(run(output), 'identifier:iban').wrong, 1);
});

test('a short name nothing settles is wrong as an organisation, in both modes', () => {
	for (const directory of ['on', 'off']) {
		const output = perfect(directory);
		doc(output, 'household-notes').orgs.find((m) => m.text === 'HDFC').org = 'ref:hdfc-bank';
		assert.equal(totals(run(output), 'mentions').wrong, 1, directory);
	}
});

test('a name inside a longer name reported as a mention is wrong', () => {
	const output = perfect();
	doc(output, 'hdfc-life-receipt').orgs.push({
		page: 1,
		start: 58,
		end: 62,
		text: 'HDFC',
		org: 'ref:hdfc-bank',
		settled_by: 'code'
	});
	const result = run(output);
	assert.equal(totals(result, 'mentions').wrong, 1);
	assert.match(wrongs(result, 'mentions')[0].what, /inside-other-name/);
});

for (const directory of ['on', 'off'])
	for (const set of ['text', 'text-ocr'])
		test(`a keyed mention on a name the labels exclude for its organisation is a wrong attachment, listed and not failing, directory ${directory}, ${set}`, () => {
			const output = perfect(directory, set);
			doc(output, 'hdfc-life-receipt').orgs.push({
				...spanOf(output, 'hdfc-life-receipt', 'HDFC'),
				org: 'ref:hdfc-bank',
				settled_by: 'name'
			});
			const result = run(output);
			assert.deepEqual(result.wrong_attachments, [
				{
					document: 'hdfc-life-receipt',
					key: 'ref:hdfc-bank',
					organisation: 'hdfc-bank',
					mention: 'HDFC at 1:58',
					why: 'inside-other-name'
				}
			]);
			assert.equal(totals(result, 'mentions').wrong, 1);
			assert.equal(result.failed, false);
		});

for (const directory of ['on', 'off'])
	test(`a card made from a name inside a longer name is a wrong attachment whatever its key maps to, directory ${directory}`, () => {
		const output = perfect(directory);
		const school = doc(output, 'school-fee-banks');
		Object.assign(
			school.orgs.find((m) => m.text === 'Bank of India'),
			spanOf(output, 'school-fee-banks', 'Bank of India')
		);
		const result = run(output);
		assert.deepEqual(result.wrong_attachments, [
			{
				document: 'school-fee-banks',
				key: 'ref:bank-of-india',
				organisation: null,
				mention: 'Bank of India at 1:144',
				why: 'inside-other-name'
			}
		]);
	});

test('a name inside a longer name keyed to the longer name’s own card is no wrong attachment', () => {
	const output = perfect();
	doc(output, 'school-fee-banks').orgs.push({
		...spanOf(output, 'school-fee-banks', 'Bank of India'),
		org: 'ref:sbi',
		settled_by: 'name'
	});
	const result = run(output);
	assert.deepEqual(result.wrong_attachments, []);
	assert.equal(totals(result, 'mentions').wrong, 1);
	assert.equal(result.failed, false);
});

// An output whose only card is `card`, reporting the first four characters of
// each labelled name in `hits` as that card's only mention in its document.
const shortHits = (directory, set, card, hits) => {
	const output = perfect(directory, set);
	return {
		...output,
		organisations: { k: card },
		documents: hits.map(([id, name]) => {
			const span = spanOf(output, id, name);
			const text = name.slice(0, 4);
			return {
				id,
				orgs: [{ ...span, end: span.start + 4, text, org: 'k', settled_by: 'name' }],
				identifiers: [],
				links: []
			};
		}),
		answers: { latest_statement: [], current_passport: [] }
	};
};

for (const directory of ['on', 'off'])
	for (const set of ['text', 'text-ocr'])
		test(`one card named by a name two organisations share, with short hits in a document of each, is a wrong attachment, directory ${directory}, ${set}`, () => {
			const output = shortHits(directory, set, { name: 'HDFC', directory: null }, [
				['hdfc-branch-letter', 'HDFC Bank Limited'],
				['hdfc-ltd-loan-2019', 'HDFC Ltd']
			]);
			const result = run(output);
			assert.notDeepEqual(result.wrong_attachments, []);
		});

for (const directory of ['on', 'off'])
	for (const set of ['text', 'text-ocr'])
		test(`a right card reported only through the short form inside a longer labelled name is a wrong attachment, directory ${directory}, ${set}`, () => {
			const card = { name: 'HDFC Bank Limited', directory: directory === 'on' ? 'Q631047' : null };
			const result = run(
				shortHits(directory, set, card, [['hdfc-branch-letter', 'HDFC Bank Limited']])
			);
			assert.deepEqual(
				result.wrong_attachments.map((a) => [a.document, a.key]),
				[['hdfc-branch-letter', 'k']]
			);
		});

test('a keyed mention on a name the labels exclude, keyed to some other card, is a wrong attachment', () => {
	const output = perfect();
	doc(output, 'hdfc-life-receipt').orgs.push({
		...spanOf(output, 'hdfc-life-receipt', 'HDFC'),
		org: 'ref:sample-traders',
		settled_by: 'name'
	});
	const result = run(output);
	assert.deepEqual(
		result.wrong_attachments.map((a) => a.organisation),
		['sample-traders']
	);
});

test('a keyed mention the labels exclude, keyed to the enclosing name’s card, or that the labels cannot tell, fails nothing', () => {
	{
		const output = perfect();
		doc(output, 'hdfc-life-receipt').orgs.push({
			...spanOf(output, 'hdfc-life-receipt', 'HDFC'),
			org: 'ref:hdfc-life',
			settled_by: 'name'
		});
		const result = run(output);
		assert.deepEqual(result.wrong_attachments, []);
		assert.equal(totals(result, 'mentions').wrong, 1);
		assert.equal(result.failed, false);
	}
	const output = perfect();
	doc(output, 'household-notes').orgs.find((m) => m.text === 'BOI').org = 'ref:bank-of-india';
	const result = run(output);
	assert.deepEqual(result.wrong_attachments, []);
	assert.equal(totals(result, 'mentions').wrong, 1);
	assert.equal(result.failed, false);
});

test('on the OCR text set, a page the OCR did not change matches by span, so swapped citations are wrong', () => {
	const wrongLinks = ['text', 'text-ocr'].map((set) => {
		const output = perfect('on', set);
		const d = doc(output, 'neft-receipt-tied');
		const hdfc = d.identifiers.find((i) => i.value === 'HDFC0SAMPL1');
		const icici = d.identifiers.find((i) => i.value === 'ICIC0SAMPL2');
		const spanOfHdfc = { page: hdfc.page, start: hdfc.start, end: hdfc.end, text: hdfc.text };
		Object.assign(hdfc, { page: icici.page, start: icici.start, end: icici.end, text: icici.text });
		Object.assign(icici, spanOfHdfc);
		const result = run(output);
		assert.equal(result.failed, true, set);
		return wrongs(result, 'links').map((w) => w.what);
	});
	assert.equal(wrongLinks[0].length, 4);
	assert.deepEqual(wrongLinks[1], wrongLinks[0]);
});

for (const directory of ['on', 'off'])
	test(`a card name ignores case and all whitespace and nothing else, directory ${directory}`, () => {
		const spaced = perfect(directory);
		card(spaced, 'mufg-bank').name = '三菱 UFJ 銀行';
		card(spaced, 'kotak').name = 'kotakmahindra BANK';
		const result = run(spaced);
		assert.deepEqual(result.wrong_cards, []);
		assert.equal(result.failed, false);
		for (const name of ['HDFC Ltd', 'HDFC-Bank']) {
			const output = perfect(directory);
			card(output, 'hdfc-bank').name = name;
			assert.deepEqual(run(output).wrong_cards.map((c) => c.claims), [[`name "${name}"`]], name);
		}
	});

test('with the directory off, a directory id or a mention settled by the directory is wrong', () => {
	const output = perfect('off');
	output.organisations['ref:kotak'].directory = 'Q2040404';
	doc(output, 'kotak-statement').orgs[0].settled_by = 'directory';
	const result = run(output);
	assert.equal(totals(result, 'org_directory').wrong, 1);
	assert.equal(totals(result, 'mentions').wrong, 1);
});

test('with the directory on, an organisation without its directory id is left out', () => {
	const output = perfect('on');
	output.organisations['ref:kotak'].directory = null;
	assert.equal(totals(run(output), 'org_directory').left_out, 1);
});

test('an empty output leaves everything out and fails nothing', () => {
	const output = { ...perfect(), documents: [], answers: {} };
	const result = run(output);
	assert.equal(sum(result, 'wrong'), 0);
	assert.equal(totals(result, 'links').left_out, 47);
	assert.equal(result.failed, false);
});

test('an output whose span does not read back breaks the format', () => {
	const output = perfect();
	doc(output, 'kotak-statement').orgs[0].start += 1;
	assert.throws(() => run(output), FormatError);
});

test('the committed text sets are what text.mjs writes from the committed files', () => {
	assert.deepEqual(checkTextSets(), []);
});

for (const directory of ['on', 'off']) {
	test(`a key no mention lands on is the bank of the code linked to it, directory ${directory}`, () => {
		const output = perfect(directory);
		assert.ok(!output.documents.some((d) => d.orgs.some((m) => m.org === 'IFSC HDFC')));
		assert.ok(output.documents.some((d) => d.links.some((l) => l.org === 'IFSC HDFC')));
		const result = run(output);
		assert.equal(sum(result, 'wrong'), 0, JSON.stringify(result.wrong));
		assert.deepEqual(
			result.splits.find((s) => s.organisation === 'hdfc-bank').keys,
			['IFSC HDFC', 'ref:hdfc-bank']
		);
	});

	test(`an account never says who a key is, so a key with only an account is no organisation, directory ${directory}`, () => {
		const output = perfect(directory);
		output.organisations['code-only'] = { ...card(output, 'icici-bank') };
		linkOf(output, 'transfer-rows', '50100417263958').org = 'code-only';
		const result = run(output);
		assert.deepEqual(
			wrongs(result, 'links').map((w) => w.what),
			['account 50100417263958 at code-only: key code-only lands on no labelled organisation']
		);
	});

	test(`a wrong link beside the right one is still wrong, directory ${directory}`, () => {
		const output = perfect(directory);
		output.organisations['code-only'] = { ...card(output, 'icici-bank') };
		const d = doc(output, 'transfer-rows');
		d.links.push({ ...linkOf(output, 'transfer-rows', '50100417263958'), org: 'code-only' });
		const result = run(output);
		assert.equal(totals(result, 'links').wrong, 1);
		assert.equal(totals(result, 'links').right, 47);
	});

	test(`a card named for another organisation is a wrong card, and every row on its key is wrong, directory ${directory}`, () => {
		for (const name of ['ICICI Bank', 'HDFC Ltd']) {
			const output = perfect(directory);
			card(output, 'hdfc-bank').name = name;
			const result = run(output);
			assert.deepEqual(result.wrong_cards, [
				{ key: 'ref:hdfc-bank', organisation: 'hdfc-bank', claims: [`name "${name}"`] }
			]);
			assert.equal(totals(result, 'links').right, 47 - 1, name);
			assert.equal(totals(result, 'statement_issuer').wrong, 4, name);
			assert.equal(totals(result, 'same_org').wrong, 4, name);
			assert.equal(result.failed, directory === 'on', 'the directory makes its joins certain');
		}
	});

	test(`a key known only by a code that names another organisation is a wrong card, directory ${directory}`, () => {
		const output = perfect(directory);
		output.organisations['IFSC HDFC'] = { ...card(output, 'icici-bank') };
		let onKey = 0;
		for (const d of output.documents) {
			for (const link of d.links.filter((l) => l.org === 'IFSC HDFC')) {
				link.tier = 'likely';
				onKey++;
			}
			for (const r of d.same_org) if (r.keys.includes('IFSC HDFC')) r.tier = 'likely';
		}
		const result = run(output);
		assert.equal(result.wrong_cards.length, 1);
		assert.equal(result.wrong_cards[0].organisation, 'hdfc-bank');
		assert.equal(totals(result, 'links').wrong, onKey);
		assert.equal(result.failed, false);
	});

	test(`a wrong card is listed even when nothing links to it, and alone fails nothing, directory ${directory}`, () => {
		const output = perfect(directory);
		card(output, 'bundesbank').name = 'Deutsche Bank';
		const result = run(output);
		assert.equal(result.wrong_cards.length, 1);
		assert.equal(totals(result, 'links').wrong, 0);
		assert.equal(totals(result, 'mentions').wrong, 1);
		assert.equal(result.failed, false);
	});

	test(`a card may give no name, or any name the labels give its organisation, directory ${directory}`, () => {
		const output = perfect(directory);
		for (const c of Object.values(output.organisations)) c.name = null;
		card(output, 'hdfc-bank').name = 'hdfc bank  ltd';
		card(output, 'kotak').name = 'Kotak Mahindra Bank';
		const result = run(output);
		assert.equal(sum(result, 'wrong'), 0, JSON.stringify(result.wrong));
		assert.deepEqual(result.wrong_cards, []);
	});

	test(`a key spelt like the scorer's own bookkeeping is only a key, directory ${directory}`, () => {
		const output = perfect(directory);
		const rename = (from, to) => {
			output.organisations[to] = output.organisations[from];
			delete output.organisations[from];
			for (const d of output.documents) {
				for (const e of [...d.orgs, ...d.links]) if (e.org === from) e.org = to;
				if (d.statement?.issuer === from) d.statement.issuer = to;
				for (const r of d.same_org) r.keys = r.keys.map((k) => (k === from ? to : k));
			}
		};
		rename('ref:hdfc-bank', '["labelled","ifsc"]');
		rename('ref:icici-bank', 'key:ref:hdfc-bank\u0000IFSC ICIC');
		const result = run(output);
		assert.equal(sum(result, 'wrong'), 0, JSON.stringify(result.wrong));
		assert.deepEqual(result.merges, []);
		assert.equal(result.failed, false);
	});
}

test("a card that gives another organisation's directory id is a wrong card", () => {
	const output = perfect('on');
	card(output, 'kotak').directory = card(output, 'icici-bank').directory;
	const result = run(output);
	assert.deepEqual(result.wrong_cards, [
		{ key: 'ref:kotak', organisation: 'kotak', claims: ['directory id Q1653258'] }
	]);
	assert.equal(totals(result, 'links').wrong, 1);
});

test("a directory entry's own name is a name of the card only with the directory on", () => {
	const on = perfect('on');
	card(on, 'hdfc-bank').name = 'HDFC銀行';
	assert.deepEqual(run(on).wrong_cards, []);
	const off = perfect('off');
	card(off, 'hdfc-bank').name = 'HDFC銀行';
	assert.equal(run(off).wrong_cards.length, 1);
});

test('an empty key breaks the format', () => {
	const output = perfect();
	output.organisations[''] = output.organisations['ref:kotak'];
	doc(output, 'kotak-statement').links[0].org = '';
	assert.throws(() => run(output), /a key must be a non-empty string/);
});

test('a link from an identifier on a decoy is wrong: no such identifier', () => {
	const output = perfect();
	const d = doc(output, 'neft-receipt-tied');
	Object.assign(d.identifiers[0], spanOf(output, d.id, 'N2026092212345678'));
	const result = run(output);
	assert.deepEqual(
		wrongs(result, 'links').map((w) => w.what),
		['account 50100417263958 at IFSC HDFC (hdfc-bank): no such identifier in the labels']
	);
});

test('a sort code given with its hyphens is wrong, and so is its link', () => {
	const output = perfect();
	doc(output, 'natwest-statement').identifiers.find((i) => i.kind === 'gb_sort_code').value =
		'60-16-13';
	const result = run(output);
	assert.equal(totals(result, 'identifier:gb_sort_code').wrong, 1);
	assert.match(wrongs(result, 'links')[0].what, /gb_sort_code 60-16-13 .*no such identifier/);
});

test('paired-code through a via that matched no labelled code does not hold', () => {
	const output = perfect();
	const d = doc(output, 'neft-receipt-tied');
	const icici = d.identifiers.find((i) => i.value === 'ICIC0SAMPL2');
	const hdfc = d.identifiers.find((i) => i.value === 'HDFC0SAMPL1');
	d.identifiers = d.identifiers.filter((i) => i !== icici);
	d.links = d.links.filter((l) => l.identifier !== icici.id && l.via !== icici.id);
	prune(d);
	Object.assign(hdfc, spanOf(output, d.id, 'ICIC0SAMPL2'));
	const result = run(output);
	assert.ok(
		wrongs(result, 'links').some((w) =>
			/^account 50100417263958 .*no reason given holds \(paired-code\)$/.test(w.what)
		),
		JSON.stringify(result.wrong)
	);
});

test('two mentions or two identifiers on one span break the format, on both text sets', () => {
	for (const set of ['text', 'text-ocr']) {
		const mentions = perfect('on', set);
		const k = doc(mentions, 'kotak-statement');
		k.orgs.push({ ...k.orgs[0], org: 'ref:hdfc-bank' });
		assert.throws(() => run(mentions), /orgs\[1\]: has the span of .*orgs\[0\]/, set);
		const identifiers = perfect('on', set);
		const n = doc(identifiers, 'kotak-statement');
		n.identifiers.push({ ...n.identifiers[0], id: 'again' });
		assert.throws(() => run(identifiers), /identifiers\[1\]: has the span of/, set);
	}
});

test('on a page the OCR read, equal values pair by position, so row order cannot change a score', () => {
	const output = perfect('on', 'text-ocr');
	const d = doc(output, 'scan-bob-letter');
	d.identifiers.push({
		...spanOf(output, d.id, 'BANK OF BARODA'),
		id: 'again',
		kind: 'card',
		value: '12340100005678',
		valid: null,
		tier: 'likely'
	});
	const inOrder = run(output);
	d.identifiers.reverse();
	assert.deepEqual(run(output), inOrder);
	assert.deepEqual(
		wrongs(inOrder, 'identifier:account').map((w) => w.what),
		[
			'account 12340100005678: given kind card',
			'account 12340100005678: not a labelled identifier'
		]
	);
});

test('the same link given twice is one link with the union of its reasons, in either order', () => {
	const results = [0, 1].map((at) => {
		const output = perfect();
		const d = doc(output, 'kotak-statement');
		d.links.splice(at, 0, { ...d.links[0], reasons: ['same-account'] });
		return run(output);
	});
	assert.deepEqual(results[0], results[1]);
	assert.equal(totals(results[0], 'links').right, 47);
	assert.equal(results[0].failed, false);
});

test("the order of an output's rows never changes its score", () => {
	for (const set of ['text', 'text-ocr']) {
		const output = perfect('on', set);
		doc(output, 'transfer-rows').links[1].reasons = ['statement-header'];
		doc(output, 'hdfc-ltd-loan-2019').orgs[0].org = 'ref:hdfc-bank';
		const neft = doc(output, 'neft-receipt-tied');
		const join = (keys, tier) => ({ keys, reason: 'printed-together', tier });
		neft.same_org.push(
			join(['IFSC ICIC', 'ref:hdfc-bank'], 'likely'),
			join(['IFSC HDFC', 'ref:icici-bank'], 'likely'),
			{ ...neft.same_org[0], reason: 'spells-code', tier: 'likely' }
		);
		const inOrder = run(output);
		for (const d of output.documents) {
			for (const list of ['orgs', 'identifiers', 'links', 'same_org']) d[list].reverse();
			for (const r of d.same_org) r.keys.reverse();
		}
		output.documents.reverse();
		assert.deepEqual(run(output), inOrder, set);
	}
});

test("validity is not scored where the label's valid is null, and is scored where it is not", () => {
	for (const valid of [true, false]) {
		const output = perfect();
		doc(output, 'pan-form-hindi').identifiers.find((i) => i.value === 'ABCPK0000F').valid = valid;
		assert.equal(totals(run(output), 'identifier:in_pan').wrong, 0, String(valid));
	}
	const output = perfect();
	doc(output, 'pan-form-hindi').identifiers.find((i) => i.value === 'ABCDK0000F').valid = null;
	assert.equal(totals(run(output), 'identifier:in_pan').wrong, 1);
});

test('a span outside its page breaks the format, even where it would read back', () => {
	const before = perfect();
	const size = [...sets.text.docs.get('kotak-statement').pages[0].text].length;
	const m = doc(before, 'kotak-statement').orgs[0];
	m.start -= size;
	m.end -= size;
	assert.throws(() => run(before), /0 <= start <= end <= 374/);
	const after = perfect();
	Object.assign(doc(after, 'kotak-statement').orgs[0], {
		start: size - 6,
		end: size + 4,
		text: '355.00',
		org: null
	});
	assert.throws(() => run(after), /0 <= start <= end <= 374/);
});

test('every passport field is checked as FORMAT.md gives it', () => {
	const cases = [
		['issued', (p) => (p.issued = '2023-02-30'), /issued must be a date/],
		['expiry', (p) => (p.expiry = '2028-13-04'), /expiry must be a date/],
		['mrz', (p) => (p.mrz = 'nonexistent'), /passport.mrz: "nonexistent" is not an identifier/],
		['number', (p) => (p.number = 'nonexistent'), /passport.number: "nonexistent" is not/],
		['expiry_at', (p) => (p.expiry_at.text = 'WRONG'), /expiry_at: "WRONG" is not the text/]
	];
	for (const [field, change, error] of cases) {
		const output = perfect();
		change(doc(output, 'passport-rohan').passport);
		assert.throws(() => run(output), error, field);
	}
});

test('every link, statement fact, passport fact and same_org row needs its tier', () => {
	const cases = [
		[(o) => delete doc(o, 'kotak-statement').links[0].tier, /links\[0\]\.tier: needs a tier/],
		[(o) => (doc(o, 'kotak-statement').links[0].tier = 'sure'), /links\[0\]\.tier: needs a tier/],
		[
			(o) => delete doc(o, 'kotak-statement').statement.tiers,
			/statement\.tiers: needs a tier for period, accounts, issuer/
		],
		[
			(o) => delete doc(o, 'kotak-statement').statement.tiers.issuer,
			/statement\.tiers\.issuer: needs a tier/
		],
		[
			(o) => delete doc(o, 'passport-rohan').passport.tiers.expiry,
			/passport\.tiers\.expiry: needs a tier/
		],
		[(o) => delete doc(o, 'neft-receipt-tied').same_org[0].tier, /same_org\[0\]\.tier: needs a tier/]
	];
	for (const [change, error] of cases) {
		const output = perfect();
		change(output);
		assert.throws(() => run(output), error);
	}
});

test("only an identifier's characters or the directory's exact code make a claim certain", () => {
	const visa = (o) =>
		(doc(o, 'visa-asha').passport = {
			holder: { surname: 'KALPIT', given: 'ASHA' },
			number: 'i1',
			mrz: 'i2',
			tiers: { holder: 'certain', number: 'likely' }
		});
	const cases = [
		['on', (o) => (doc(o, 'kotak-statement').statement.tiers.period = 'certain'), /statement\.tiers\.period: cannot be certain/],
		['on', (o) => (doc(o, 'passport-rohan').passport.tiers.current = 'certain'), /passport\.tiers\.current: cannot be certain/],
		['on', (o) => (doc(o, 'passport-bad-mrz').passport.tiers.holder = 'certain'), /passport\.tiers\.holder: cannot be certain/],
		['on', visa, /passport\.tiers\.holder: cannot be certain/],
		['off', (o) => (linkOf(o, 'hdfc-statement-sep', 'HDFC0SAMPL1').tier = 'certain'), /no reason passes its test for certain/],
		[
			'off',
			(o) => {
				o.organisations['IFSC HDFC'].directory = 'Q631047';
				linkOf(o, 'hdfc-statement-sep', 'HDFC0SAMPL1').tier = 'certain';
			},
			/no reason passes its test for certain/
		],
		[
			'on',
			(o) =>
				doc(o, 'rechnung-falsche-iban').links.push({
					identifier: 'i1',
					org: 'IBAN DE 37040044',
					reasons: ['iban-bank-code'],
					tier: 'certain'
				}),
			/no reason passes its test for certain/
		],
		['on', (o) => (o.organisations['IFSC HDFC'].directory = card(o, 'icici-bank').directory), /no reason passes its test for certain/],
		[
			'on',
			(o) => {
				const bic = doc(o, 'natwest-statement').identifiers.find((i) => i.kind === 'bic');
				bic.value = 'NWBKGB99';
			},
			/identifiers\[2\]\.tier: kind bic cannot be certain/
		]
	];
	for (const [directory, change, error] of cases) {
		const output = perfect(directory);
		change(output);
		assert.throws(() => run(output), error);
	}
});

test('a same_org row joining two organisations is wrong: likely, it fails nothing; certain, it fails the run', () => {
	const likely = perfect();
	doc(likely, 'neft-receipt-tied').same_org.push({
		keys: ['IFSC ICIC', 'ref:hdfc-bank'],
		reason: 'printed-together',
		tier: 'likely'
	});
	const result = run(likely);
	assert.equal(byTier(result, 'same_org', 'likely').wrong, 1);
	assert.equal(result.failed, false);

	// The code's key claims to be ICICI Bank, so a certain join of it to HDFC
	// Bank's name joins two organisations' cards.
	const certain = perfect();
	certain.organisations['IFSC HDFC'] = { ...card(certain, 'icici-bank') };
	for (const d of certain.documents)
		for (const row of [...d.links, ...d.same_org])
			if (d.id !== 'hdfc-statement-sep' && (row.org === 'IFSC HDFC' || row.keys?.includes('IFSC HDFC')))
				row.tier = 'likely';
	for (const l of doc(certain, 'hdfc-statement-sep').links) l.tier = 'likely';
	assert.deepEqual(doc(certain, 'hdfc-statement-sep').same_org[0].tier, 'certain');
	const failed = run(certain);
	assert.deepEqual(byTier(failed, 'same_org', 'certain').wrong, 1);
	assert.equal(failed.failed, true);
});

test('a join of two keys of one organisation that no row gives is left out, and joins chain', () => {
	const output = perfect();
	doc(output, 'hdfc-statement-sep').same_org = [];
	const natwest = doc(output, 'natwest-statement');
	natwest.same_org = [
		{ keys: ['BIC NWBKGB2L', 'IBAN GB NWBK'], reason: 'printed-together', tier: 'likely' },
		natwest.same_org[0]
	];
	const result = run(output);
	assert.deepEqual(leftOut(result), ['same_org hdfc-statement-sep']);
	assert.equal(totals(result, 'same_org').right, 16);
	assert.equal(result.failed, false);
});

test('same_org rows for the same two keys are one claim, certain when one row is', () => {
	const output = perfect();
	const d = doc(output, 'hdfc-statement-sep');
	d.same_org.unshift({ keys: [...d.same_org[0].keys].reverse(), reason: 'spells-code', tier: 'likely' });
	const result = run(output);
	const alone = run(perfect());
	assert.deepEqual(byTier(result, 'same_org', 'certain'), byTier(alone, 'same_org', 'certain'));
	assert.deepEqual(byTier(result, 'same_org', 'likely'), byTier(alone, 'same_org', 'likely'));
});

test('a same_org row joins two keys the document uses, by a known reason, and only a directory row may be certain', () => {
	const row = (o, change) => Object.assign(doc(o, 'hdfc-statement-sep').same_org[0], change);
	const cases = [
		['on', { keys: ['ref:kotak', 'ref:hdfc-bank'] }, /key ref:kotak is not used/],
		['on', { keys: ['ref:hdfc-bank', 'ref:hdfc-bank'] }, /two different keys/],
		['on', { reason: 'nearby' }, /reason must be one of/],
		['on', { reason: 'spells-code' }, /only a directory row/],
		['off', { reason: 'directory', tier: 'certain' }, /only a directory row/]
	];
	for (const [directory, change, error] of cases) {
		const output = perfect(directory);
		row(output, change);
		assert.throws(() => run(output), error, JSON.stringify(change));
	}
});

// Scores an output, then the same output with every list of rows reversed,
// and requires one verdict.
const inBothOrders = (output) => {
	const forward = run(output);
	const backward = structuredClone(output);
	for (const d of backward.documents) {
		for (const list of ['orgs', 'identifiers', 'links', 'same_org']) d[list].reverse();
		for (const r of d.same_org) r.keys.reverse();
	}
	backward.documents.reverse();
	assert.deepEqual(run(backward), forward, 'rows reversed');
	return forward;
};

for (const directory of ['on', 'off'])
	test(`a valid IBAN held at its own code key is a right certain link, whatever reasons the label lists, directory ${directory}`, () => {
		const output = perfect(directory);
		const d = doc(output, 'rib-fr');
		const iban = d.identifiers.find((i) => i.kind === 'iban');
		assert.equal(iban.tier, 'certain');
		output.organisations['IBAN FR 20041'] = { name: null, directory: null };
		// The label lists only paired-code for this IBAN; drop that row so the
		// code's own reason must hold by itself.
		d.links = d.links.filter((l) => l.identifier !== iban.id);
		d.links.push({ identifier: iban.id, org: 'IBAN FR 20041', reasons: ['iban-bank-code'], tier: 'certain' });
		const result = inBothOrders(output);
		assert.deepEqual(wrongs(result, 'links'), []);
		assert.equal(byTier(result, 'links', 'certain').right, byTier(run(perfect(directory)), 'links', 'certain').right + 1);
		assert.equal(result.failed, false);
	});

test('a same_org row is certain only when the printed name and the code each look up to one directory entry, the same one', () => {
	// HDFC Bank's card and the code's card both give its directory id, but the
	// bare name HDFC looks up to two entries: the document's wording settles it.
	const ambiguous = perfect('on');
	const neft = doc(ambiguous, 'neft-receipt-tied');
	assert.equal(card(ambiguous, 'hdfc-bank').directory, ambiguous.organisations['IFSC HDFC'].directory);
	Object.assign(neft.same_org.find((r) => r.keys.includes('IFSC HDFC')), { reason: 'directory', tier: 'certain' });
	assert.throws(() => inBothOrders(ambiguous), /only a directory row whose printed name and code each look up to one entry/);
	// HDFC Bank Ltd and the code HDFC look up to one entry each, the same one,
	// so the row is certain whatever ids the cards give.
	const exact = perfect('on');
	for (const key of ['ref:hdfc-bank', 'IFSC HDFC']) exact.organisations[key].directory = null;
	for (const d of exact.documents)
		for (const l of d.links) if (l.org === 'IFSC HDFC') l.tier = 'likely';
	assert.equal(doc(exact, 'hdfc-statement-sep').same_org[0].tier, 'certain');
	const result = inBothOrders(exact);
	assert.equal(byTier(result, 'same_org', 'certain').wrong, 0);
	assert.equal(result.failed, false);
});

test('two same_org rows whose keys joined by a separator would spell one claim are two claims', () => {
	const output = perfect('on');
	const d = doc(output, 'neft-receipt-tied');
	const keyOf = { HDFC: 'a', 'HDFC Bank': 'b\u0000c', 'Sample Traders': 'a\u0000b', 'ICICI Bank': 'c' };
	for (const key of Object.values(keyOf)) output.organisations[key] = { name: null, directory: null };
	for (const m of d.orgs) m.org = keyOf[m.text];
	prune(d);
	const join = (keys) => ({ keys, reason: 'printed-together', tier: 'likely' });
	for (const rows of [[['a', 'b\u0000c'], ['a\u0000b', 'c']], [['a\u0000b', 'c'], ['a', 'b\u0000c']]]) {
		d.same_org = rows.map(join);
		const result = inBothOrders(output);
		assert.deepEqual(
			wrongs(result, 'same_org').map((w) => w.what),
			['a\u0000b and c: a\u0000b is sample-traders, c is icici-bank']
		);
	}
});

test('an identifier is certain only when its own strong check proves its kind, and a wrong certain identifier fails the run', () => {
	const certain = (id, value) => (o) => (doc(o, id).identifiers.find((i) => i.value === value).tier = 'certain');
	const refused = [
		['on', certain('neft-receipt-tied', '50100417263958'), /kind account cannot be certain/],
		['on', certain('hdfc-card-statement', '4242424242424242'), /kind card cannot be certain/],
		['on', certain('natwest-statement', '601613'), /kind gb_sort_code cannot be certain/],
		['on', certain('passport-rohan', 'SMP002935'), /kind passport_number cannot be certain/],
		['off', certain('neft-receipt-tied', 'HDFC0SAMPL1'), /kind ifsc cannot be certain/],
		['on', certain('rechnung-falsche-iban', 'DE88370400440532013000'), /kind iban cannot be certain/],
		[
			'on',
			(o) => (doc(o, 'passport-rohan').identifiers.find((i) => i.kind === 'passport_number').value = 'SMP002936'),
			/passport\.tiers\.number: cannot be certain/
		]
	];
	for (const [directory, change, error] of refused) {
		const output = perfect(directory);
		change(output);
		assert.throws(() => run(output), error);
	}
	for (const tier of ['certain', 'likely']) {
		const output = perfect('off');
		const d = doc(output, 'natwest-statement');
		const iban = d.identifiers.find((i) => i.kind === 'iban');
		Object.assign(iban, { value: 'DE89370400440532013000', tier });
		d.links = d.links.filter((l) => l.identifier !== iban.id && l.via !== iban.id);
		prune(d);
		const result = inBothOrders(output);
		assert.deepEqual(byTier(result, 'identifier:iban', tier).wrong, 1, tier);
		assert.equal(result.failed, tier === 'certain', tier);
	}
});

test("a code linked to a printed name's key by layout alone gives the key no code and does not remap it", () => {
	const output = perfect();
	const d = doc(output, 'neft-receipt-tied');
	const ifsc = d.identifiers.find((i) => i.value === 'HDFC0SAMPL1');
	d.links.push({ identifier: ifsc.id, org: 'ref:sample-traders', reasons: ['statement-header'], tier: 'likely' });
	const result = inBothOrders(output);
	assert.deepEqual(result.merges, []);
	assert.deepEqual(
		wrongs(result, 'links').map((w) => w.what),
		['ifsc HDFC0SAMPL1 at ref:sample-traders (sample-traders): a not_link (code-names-other-bank)']
	);
	assert.equal(result.failed, false);
});

test('a card no certain row stands on is counted as resting on likely rows only', () => {
	const on = run(perfect('on'));
	assert.ok(!on.likely_only_cards.includes('IFSC HDFC'));
	assert.ok(!on.likely_only_cards.includes('ref:hdfc-bank'), 'a certain join stands on it');
	assert.ok(on.likely_only_cards.includes('ref:kotak'));
	const off = run(perfect('off'));
	assert.ok(off.likely_only_cards.includes('IFSC HDFC'));
	assert.ok(!off.likely_only_cards.includes('IBAN GB NWBK'), 'a valid IBAN stands on its own');
	assert.match(table(off), new RegExp(`cards on likely rows only +${off.likely_only_cards.length}\\b`));
});

test('every JSON block in FORMAT.md holds against the committed text and the scorer', () => {
	const format = readFileSync(new URL('FORMAT.md', SAMPLE), 'utf8');
	const blocks = [...format.matchAll(/```json\n([\s\S]*?)\n```/g)].map((m) => JSON.parse(m[1]));
	assert.equal(blocks.length, 4);
	const outputOf = (documents, directory = 'on') => ({
		format: 2,
		extractor: 'FORMAT.md',
		directory,
		text_set: 'text',
		organisations: {},
		documents,
		answers: {}
	});
	for (const block of blocks) {
		if (block.pages) {
			const committed = sets.text.docs.get(block.id);
			for (const [field, value] of Object.entries(block))
				if (field !== 'pages') assert.equal(value, committed[field], `${block.id} ${field}`);
			block.pages.forEach((page, i) => {
				let at = 0;
				for (const part of page.text.split('…')) {
					at = committed.pages[i].text.indexOf(part, at);
					assert.ok(at >= 0, `${block.id} page ${i + 1} holds ${JSON.stringify(part)}`);
				}
				assert.equal(page.scanned, committed.pages[i].scanned);
			});
		} else if (block.format) {
			for (const directory of ['on', 'off']) {
				const output = structuredClone(block);
				output.directory = directory;
				// As the text after the example says.
				if (directory === 'off') {
					for (const c of Object.values(output.organisations)) c.directory = null;
					for (const d of output.documents) {
						for (const x of [...d.identifiers, ...d.links]) x.tier = 'likely';
						for (const r of d.same_org) Object.assign(r, { reason: 'spells-code', tier: 'likely' });
					}
				}
				const result = run(output);
				assert.equal(sum(result, 'wrong'), 0, JSON.stringify(result.wrong));
				const given = new Set(block.documents.map((d) => d.id));
				assert.deepEqual(
					result.left_out.filter((i) => given.has(i.document)),
					[],
					directory
				);
				assert.equal(totals(result, 'links').right, 2, directory);
				assert.deepEqual(byTier(result, 'links', 'certain'), directory === 'on' ? { right: 1, wrong: 0 } : undefined);
				assert.equal(totals(result, 'same_org').right, 1, directory);
				assert.equal(totals(result, 'answer_latest_statement').right, 1, directory);
				assert.equal(totals(result, 'org_directory').right, 2, directory);
			}
		} else {
			const result = run(outputOf([block]));
			for (const i of block.identifiers)
				assert.equal(totals(result, `identifier:${i.kind}`).right, 1, `${block.id} ${i.kind}`);
			assert.equal(sum(result, 'wrong'), 0, JSON.stringify(result.wrong));
		}
	}
});

// A copy of sample/ reached through a symlink, as /tmp is on macOS.
function linkedCopy() {
	const dir = mkdtempSync(join(tmpdir(), 'score-test-'));
	cpSync(fileURLToPath(SAMPLE), join(dir, 'sample'), { recursive: true });
	symlinkSync(join(dir, 'sample'), join(dir, 'link'));
	return { dir, sample: join(dir, 'link') };
}
const node = (...args) => spawnSync(process.execPath, args, { encoding: 'utf8' });

test('the tools run when started through a symlinked path', () => {
	const { dir, sample } = linkedCopy();
	try {
		const output = join(dir, 'reference.json');
		const made = node(join(sample, 'tools', 'reference.mjs'), 'on', 'text', output);
		assert.equal(made.status, 0, made.stderr);
		assert.ok(existsSync(output), 'reference.mjs wrote its output');
		const scored = node(join(sample, 'tools', 'score.mjs'), output);
		assert.equal(scored.status, 0, scored.stderr);
		assert.match(scored.stdout, /pass: no certain row is wrong and no key merges/);
		assert.equal(JSON.parse(readFileSync(join(dir, 'reference.score.json'), 'utf8')).failed, false);
		const likely = JSON.parse(readFileSync(output, 'utf8'));
		likely.answers.latest_statement[0].document = 'kotak-statement';
		writeFileSync(output, JSON.stringify(likely));
		assert.equal(node(join(sample, 'tools', 'score.mjs'), output).status, 0, 'a likely row');
		const merged = JSON.parse(readFileSync(output, 'utf8'));
		nameOnCode(merged, 'hdfc-statement-sep', 'IFSC HDFC');
		writeFileSync(output, JSON.stringify(merged));
		const failing = node(join(sample, 'tools', 'score.mjs'), output);
		assert.equal(failing.status, 1);
		assert.match(failing.stdout, /MERGE: key IFSC HDFC is an institution code and holds a printed name/);
		const checked = node(join(sample, 'tools', 'text.mjs'), '--check');
		assert.equal(checked.status, 0, checked.stderr);
		assert.match(checked.stdout, /match what text.mjs writes/);
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
});

test('a root that is not an object, a format 1 output, and paired-code with no via, break the format', () => {
	const dir = mkdtempSync(join(tmpdir(), 'score-test-'));
	try {
		const noVia = perfect();
		delete doc(noVia, 'transfer-rows').links.find((l) => l.reasons.includes('paired-code')).via;
		const cases = [
			['null', /output: must be an object/],
			['[]', /output: must be an object/],
			[JSON.stringify({ ...perfect(), format: 1 }), /format 1 has no tiers. Write format 2/],
			[JSON.stringify(noVia), /links\[\d+\]: paired-code needs a via/]
		];
		for (const [text, error] of cases) {
			const file = join(dir, 'output.json');
			writeFileSync(file, text);
			const scored = node(fileURLToPath(new URL('tools/score.mjs', SAMPLE)), file);
			assert.equal(scored.status, 2, text.slice(0, 20));
			assert.match(scored.stderr, error);
		}
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
});

test('a changed file in files/ makes the text stale even when SHA256SUMS is not updated', () => {
	const { dir, sample } = linkedCopy();
	try {
		const file = join(sample, 'files', 'notes-october.txt');
		writeFileSync(file, readFileSync(file, 'utf8').replace('10,000', '10,001'));
		const checked = node(join(sample, 'tools', 'text.mjs'), '--check');
		assert.equal(checked.status, 1);
		assert.match(checked.stderr, /household-notes\.json: differs/);
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
});
