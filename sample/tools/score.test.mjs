// node --test sample/tools/score.test.mjs
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { SAMPLE } from './generate.mjs';
import { readLabels } from './lib/labels.mjs';
import { reference } from './reference.mjs';
import { FormatError, score } from './score.mjs';
import { checkTextSets, readTextSet } from './text.mjs';

const labels = readLabels(new URL('labels.json', SAMPLE));
const sets = { text: readTextSet('text'), 'text-ocr': readTextSet('text-ocr') };

const perfect = (directory = 'on', set = 'text') =>
	structuredClone(reference(labels, sets[set], directory));
const run = (output) => score(labels, sets[output.text_set], output);
const doc = (output, id) => output.documents.find((d) => d.id === id);
const totals = (result, measure) => result.measures[measure].total;
const sum = (result, outcome, skip = () => false) =>
	Object.entries(result.measures)
		.filter(([m]) => !skip(m))
		.reduce((n, [, r]) => n + r.total[outcome], 0);
const wrongs = (result, measure) => result.wrong.filter((w) => w.measure === measure);

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
		wrongs(result, 'links').map((w) => w.what),
		[
			'card 4111111111111112 at ref:sample-supplies (sample-supplies): a not_link (invalid-identifier)',
			'card 4111111111111112 at ref:natwest (natwest): the identifier is invalid'
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
