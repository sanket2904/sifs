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
import { FormatError, score } from './score.mjs';
import { checkTextSets, readTextSet } from './text.mjs';

const labels = readLabels(new URL('labels.json', SAMPLE));
const sets = { text: readTextSet('text'), 'text-ocr': readTextSet('text-ocr') };

const perfect = (directory = 'on', set = 'text') =>
	structuredClone(reference(labels, sets[set], directory));
const banks = JSON.parse(readFileSync(new URL('directory/banks.json', SAMPLE), 'utf8'));
const run = (output) => score(labels, sets[output.text_set], output, banks);
const doc = (output, id) => output.documents.find((d) => d.id === id);
const totals = (result, measure) => result.measures[measure].total;
const sum = (result, outcome, skip = () => false) =>
	Object.entries(result.measures)
		.filter(([m]) => !skip(m))
		.reduce((n, [, r]) => n + r.total[outcome], 0);
const wrongs = (result, measure) => result.wrong.filter((w) => w.measure === measure);
const card = (output, local) => output.organisations[`ref:${local}`];
// A span of the committed text of a document, found by its text.
const spanOf = (output, id, text) => {
	const pages = sets[output.text_set].docs.get(id).pages;
	const page = pages.findIndex((p) => p.text.includes(text));
	const start = [...pages[page].text.slice(0, pages[page].text.indexOf(text))].length;
	return { page: page + 1, start, end: start + [...text].length, text };
};

for (const directory of ['on', 'off']) {
	test(`the perfect output scores everything right, directory ${directory}`, () => {
		const result = run(perfect(directory));
		for (const outcome of ['wrong', 'left_out', 'impossible'])
			assert.equal(sum(result, outcome), 0, `${outcome}: ${JSON.stringify(result[outcome])}`);
		assert.deepEqual(result.merges, []);
		assert.deepEqual(result.splits, []);
		assert.equal(result.failed, false);
		assert.equal(totals(result, 'links').right, 47);
		assert.equal(totals(result, 'answer_latest_statement').right, 12);
		assert.equal(totals(result, 'answer_current_passport').right, 3);
	});

	test(`on the OCR text set the perfect output loses only what OCR made impossible, directory ${directory}`, () => {
		const result = run(perfect(directory, 'text-ocr'));
		assert.equal(sum(result, 'wrong'), 0, JSON.stringify(result.wrong));
		assert.equal(sum(result, 'left_out'), 0, JSON.stringify(result.left_out));
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
		valid: true
	});
	d.links.push({ identifier: 'i2', org: 'ref:bank-of-baroda', reasons: ['ifsc-prefix'] });
	const result = run(output);
	assert.equal(totals(result, 'identifier:ifsc').right, 12);
	assert.equal(totals(result, 'identifier:ifsc').impossible, 1);
	assert.equal(totals(result, 'links').impossible, 3);
	assert.equal(result.failed, false);
});

test('a link to an organisation the document does not settle is wrong and fails the run', () => {
	const output = perfect();
	doc(output, 'hdfc-statement-sep').links[0].org = 'ref:kotak';
	const result = run(output);
	assert.equal(totals(result, 'links').wrong, 1);
	assert.match(wrongs(result, 'links')[0].what, /does not settle kotak/);
	assert.equal(totals(result, 'links').left_out, 1, 'the account is no longer linked to HDFC Bank');
	assert.equal(result.failed, true);
});

test('a link the labels list under not_links is wrong', () => {
	const output = perfect();
	doc(output, 'neft-receipt-tied').links.push({
		identifier: 'i1',
		org: 'ref:icici-bank',
		reasons: ['paired-code'],
		via: 'i4'
	});
	const result = run(output);
	assert.equal(totals(result, 'links').wrong, 1);
	assert.match(wrongs(result, 'links')[0].what, /not_link \(code-names-other-bank\)/);
	assert.equal(result.failed, true);
});

test('a link on an identifier the labels mark invalid is wrong', () => {
	const output = perfect();
	const d = doc(output, 'invoice-lookalike-numbers');
	d.links.push({ identifier: 'i2', org: 'ref:sample-supplies', reasons: ['statement-header'] });
	d.links.push({ identifier: 'i2', org: 'ref:natwest', reasons: ['statement-header'] });
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

test('one key over two labelled organisations is a wrong merge and fails the run', () => {
	const output = perfect();
	for (const m of doc(output, 'hdfc-ltd-loan-2019').orgs) m.org = 'ref:hdfc-bank';
	doc(output, 'hdfc-ltd-loan-2019').statement.issuer = 'ref:hdfc-bank';
	const result = run(output);
	assert.deepEqual(result.merges, [
		{ key: 'ref:hdfc-bank', organisations: ['hdfc-bank', 'hdfc-ltd'] }
	]);
	assert.ok(totals(result, 'links').wrong > 0, 'links to the merged key are wrong');
	assert.equal(result.failed, true);
});

test('one labelled organisation over two keys is a split, counted and not wrong', () => {
	const output = perfect();
	output.organisations['hdfc-2'] = { name: 'HDFC Bank Limited', directory: 'Q631047' };
	for (const m of doc(output, 'hdfc-branch-letter').orgs) m.org = 'hdfc-2';
	const result = run(output);
	assert.deepEqual(result.splits, [
		{ organisation: 'hdfc-bank', keys: ['hdfc-2', 'ref:hdfc-bank'] }
	]);
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
		current: true
	};
	const result = run(output);
	for (const m of ['passport_holder', 'passport_number', 'passport_expiry', 'passport_current'])
		assert.equal(totals(result, m).wrong, 1, m);
});

test('a wrong latest statement is wrong and fails the run', () => {
	const output = perfect();
	output.answers.latest_statement.find((a) => a.account === '50100417263958').document =
		'hdfc-statement-jul';
	const result = run(output);
	assert.equal(totals(result, 'answer_latest_statement').wrong, 1);
	assert.equal(result.failed, true);
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
		valid: null
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
		output.organisations['code-only'] = { ...card(output, 'hdfc-bank') };
		for (const link of doc(output, 'transfer-rows').links.slice(0, 2)) link.org = 'code-only';
		const result = run(output);
		assert.equal(sum(result, 'wrong'), 0, JSON.stringify(result.wrong));
		assert.deepEqual(result.splits, [
			{ organisation: 'hdfc-bank', keys: ['code-only', 'ref:hdfc-bank'] }
		]);
		assert.equal(result.failed, false);
	});

	test(`an account never says who a key is, so a key with only an account is no organisation, directory ${directory}`, () => {
		const output = perfect(directory);
		output.organisations['code-only'] = { ...card(output, 'icici-bank') };
		doc(output, 'transfer-rows').links[1].org = 'code-only';
		const result = run(output);
		assert.deepEqual(
			wrongs(result, 'links').map((w) => w.what),
			['account 50100417263958 at code-only: key code-only lands on no labelled organisation']
		);
		assert.equal(result.failed, true);
	});

	test(`a wrong link beside the right one is still wrong, directory ${directory}`, () => {
		const output = perfect(directory);
		output.organisations['code-only'] = { ...card(output, 'icici-bank') };
		const d = doc(output, 'transfer-rows');
		d.links.push({ ...d.links[1], org: 'code-only' });
		const result = run(output);
		assert.equal(totals(result, 'links').wrong, 1);
		assert.equal(totals(result, 'links').right, 47);
		assert.equal(result.failed, true);
	});

	test(`a card named for another organisation is a wrong card, directory ${directory}`, () => {
		for (const name of ['ICICI Bank', 'HDFC Ltd']) {
			const output = perfect(directory);
			card(output, 'hdfc-bank').name = name;
			const result = run(output);
			assert.deepEqual(result.wrong_cards, [
				{ key: 'ref:hdfc-bank', organisation: 'hdfc-bank', claims: [`name "${name}"`] }
			]);
			assert.equal(totals(result, 'links').right, 47 - 15, name);
			assert.equal(totals(result, 'statement_issuer').wrong, 4, name);
			assert.equal(result.failed, true);
		}
	});

	test(`a key known only by a code that names another organisation is a wrong card, directory ${directory}`, () => {
		const output = perfect(directory);
		output.organisations['code-only'] = { ...card(output, 'icici-bank') };
		for (const link of doc(output, 'transfer-rows').links.slice(0, 2)) link.org = 'code-only';
		const result = run(output);
		assert.equal(result.wrong_cards.length, 1);
		assert.equal(result.wrong_cards[0].organisation, 'hdfc-bank');
		assert.equal(totals(result, 'links').wrong, 2);
		assert.equal(result.failed, true);
	});

	test(`a wrong card fails the run even when nothing links to it, directory ${directory}`, () => {
		const output = perfect(directory);
		card(output, 'bundesbank').name = 'Deutsche Bank';
		const result = run(output);
		assert.equal(result.wrong_cards.length, 1);
		assert.equal(totals(result, 'links').wrong, 0);
		assert.equal(totals(result, 'mentions').wrong, 1);
		assert.equal(result.failed, true);
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
			}
		};
		rename('ref:hdfc-bank', 'link:icici-code-only');
		output.organisations['icici-code-only'] = { ...card(output, 'icici-bank') };
		for (const link of doc(output, 'transfer-rows').links.slice(2)) link.org = 'icici-code-only';
		const result = run(output);
		assert.equal(sum(result, 'wrong'), 0, JSON.stringify(result.wrong));
		assert.deepEqual(result.merges, []);
		assert.equal(result.failed, false);
	});
}

test("a card that gives another organisation's directory id is a wrong card", () => {
	const output = perfect('on');
	card(output, 'hdfc-bank').directory = card(output, 'icici-bank').directory;
	const result = run(output);
	assert.deepEqual(result.wrong_cards, [
		{ key: 'ref:hdfc-bank', organisation: 'hdfc-bank', claims: ['directory id Q1653258'] }
	]);
	assert.equal(result.failed, true);
});

test("a directory entry's own name is a name of the card only with the directory on", () => {
	const on = perfect('on');
	card(on, 'hdfc-bank').name = 'HDFC銀行';
	assert.deepEqual(run(on).wrong_cards, []);
	const off = perfect('off');
	card(off, 'hdfc-bank').name = 'HDFC銀行';
	assert.equal(run(off).wrong_cards.length, 1);
});

test('a code linked to a key its mentions already settle is a wrong link, not a merge', () => {
	const output = perfect();
	doc(output, 'transfer-rows').links[2].org = 'ref:hdfc-bank';
	const result = run(output);
	assert.deepEqual(result.merges, []);
	assert.match(wrongs(result, 'links')[0].what, /ifsc ICIC0SAMPL2 .*code-names-other-bank/);
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
		['account 50100417263958 at ref:hdfc-bank (hdfc-bank): no such identifier in the labels']
	);
	assert.equal(result.failed, true);
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

test('on the OCR text set, equal texts pair by position, so row order cannot hide a merge', () => {
	const output = perfect('on', 'text-ocr');
	const d = doc(output, 'hdfc-letter-hindi');
	d.orgs.filter((m) => m.text === 'एचडीएफसी बैंक')[1].org = 'ref:kotak';
	const inOrder = run(output);
	d.orgs.reverse();
	const reversed = run(output);
	assert.deepEqual(reversed, inOrder);
	assert.deepEqual(inOrder.merges, [{ key: 'ref:kotak', organisations: ['hdfc-bank', 'kotak'] }]);
	assert.deepEqual(
		inOrder.wrong.filter((w) => w.document === d.id).map((w) => w.what),
		['एचडीएफसी बैंक at 1:291 (ref:kotak): key ref:kotak merges kotak and hdfc-bank']
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
		const inOrder = run(output);
		for (const d of output.documents)
			for (const list of ['orgs', 'identifiers', 'links']) d[list].reverse();
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

test('every JSON block in FORMAT.md holds against the committed text and the scorer', () => {
	const format = readFileSync(new URL('FORMAT.md', SAMPLE), 'utf8');
	const blocks = [...format.matchAll(/```json\n([\s\S]*?)\n```/g)].map((m) => JSON.parse(m[1]));
	assert.equal(blocks.length, 4);
	const outputOf = (documents, directory = 'on') => ({
		format: 1,
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
				if (directory === 'off')
					for (const c of Object.values(output.organisations)) c.directory = null;
				const result = run(output);
				assert.equal(sum(result, 'wrong'), 0, JSON.stringify(result.wrong));
				const given = new Set(block.documents.map((d) => d.id));
				assert.deepEqual(
					result.left_out.filter((i) => given.has(i.document)),
					[],
					directory
				);
				assert.equal(totals(result, 'links').right, 1, directory);
				assert.equal(totals(result, 'answer_latest_statement').right, 1, directory);
				assert.equal(totals(result, 'org_directory').right, 1, directory);
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
		assert.equal(JSON.parse(readFileSync(join(dir, 'reference.score.json'), 'utf8')).failed, false);
		const wrong = JSON.parse(readFileSync(output, 'utf8'));
		wrong.answers.latest_statement[0].document = 'kotak-statement';
		writeFileSync(output, JSON.stringify(wrong));
		assert.equal(node(join(sample, 'tools', 'score.mjs'), output).status, 1);
		const checked = node(join(sample, 'tools', 'text.mjs'), '--check');
		assert.equal(checked.status, 0, checked.stderr);
		assert.match(checked.stdout, /match what text.mjs writes/);
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
