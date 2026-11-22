// Breaks one label (or file) at a time in a copy of sample/ and shows that
// check.mjs reports it. Exits 1 if any break goes unreported.
//   node sample/tools/mutants.mjs
import { spawnSync } from 'node:child_process';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const sample = fileURLToPath(new URL('../', import.meta.url));
const doc = (labels, id) => labels.documents.find((d) => d.id === id);

const MUTANTS = [
	['span moved by one', (l) => doc(l, 'kotak-statement').orgs[0].start++, 'reads'],
	[
		'organisation id changed to another bank',
		(l) => (doc(l, 'hdfc-branch-letter').orgs[0].org = 'Q1446319'),
		'by_name exact, but the directory gives Q631047'
	],
	[
		'organisation id not in the directory',
		(l) => (doc(l, 'transfer-untied').orgs[1].org = 'Q999999999'),
		'is not in the directory'
	],
	[
		'alias clash resolved to one bank',
		(l) =>
			Object.assign(doc(l, 'household-notes').orgs[0], {
				org: 'Q2004439',
				by_name: 'exact',
				candidates: undefined
			}),
		'by_name exact, but the directory gives Q2004439, Q28457664'
	],
	[
		'look-alike label removed',
		(l) => doc(l, 'school-fee-banks').not_orgs.pop(),
		'which no org or not_org label covers'
	],
	[
		'invalid IBAN marked valid',
		(l) => (doc(l, 'rechnung-falsche-iban').identifiers[0].valid = true),
		'marked valid true, its check gives false'
	],
	[
		'valid card marked invalid',
		(l) => (doc(l, 'hdfc-card-statement').identifiers[0].valid = false),
		'marked valid false, its check gives true'
	],
	[
		'broken MRZ marked valid',
		(l) => (doc(l, 'passport-bad-mrz').identifiers[1].valid = true),
		'marked valid true, its check gives false'
	],
	[
		'identifier value changed',
		(l) => (doc(l, 'rent-receipt').identifiers[0].value = '33445566778898'),
		'is not "33445566778899" normalized'
	],
	[
		'decoy label removed',
		(l) => doc(l, 'rent-receipt').not_identifiers.pop(),
		'which no identifier or not_identifier label covers'
	],
	[
		'IBAN linked to the wrong bank',
		(l) => Object.assign(doc(l, 'deutsche-bank-brief').links[0], { org: 'Q162222', mention: 'o2' }),
		'is not a bank code of Q162222'
	],
	[
		'reason that holds left out',
		(l) => (doc(l, 'kotak-statement').links[0].reasons = ['paired-code']),
		'statement-header holds but is not listed'
	],
	[
		'IFSC link removed',
		(l) => doc(l, 'utility-bill').links.pop(),
		'its code names Q631047, but there is no link'
	],
	[
		'link end not in the document',
		(l) => (doc(l, 'neft-receipt-tied').links[0].identifier = 'i9'),
		'identifier is not in this document'
	],
	[
		'untied account linked',
		(l) =>
			doc(l, 'transfer-untied').links.push({
				identifier: 'i2',
				org: 'Q2882956',
				mention: 'o3',
				reasons: ['statement-header']
			}),
		'the same pair is also a link'
	],
	[
		'paired code two lines away',
		(l) =>
			(doc(l, 'natwest-statement').links[0] = {
				identifier: 'i1',
				org: 'Q2740021',
				mention: 'o1',
				reasons: ['paired-code'],
				via: 'i3'
			}),
		'is not on the same or the next line'
	],
	[
		'not_link reason that is false',
		(l) => (doc(l, 'transfer-untied').not_links[2].why = 'invalid-identifier'),
		'invalid-identifier, but the identifier passes its check'
	],
	[
		'statement period changed',
		(l) => (doc(l, 'hdfc-statement-sep').statement.to = '2026-09-29'),
		'the text gives 2026-09-01 and 2026-09-30'
	],
	[
		'latest statement answer changed',
		(l) => (l.answers.latest_statement[0].document = 'hdfc-statement-jul'),
		'latest_statement should be'
	],
	[
		'passport expiry changed',
		(l) => (doc(l, 'passport-rohan').passport.expiry = '2028-01-05'),
		'expiry_at reads 2028-01-04, not 2028-01-05'
	],
	[
		'expired passport marked current',
		(l) => (doc(l, 'passport-asha-expired').passport.current = true),
		'current is true, the dates give false'
	],
	[
		'cannot-tell expiry given a date',
		(l) =>
			Object.assign(doc(l, 'passport-bad-mrz').passport, {
				expiry: '2031-04-15',
				expiry_candidates: undefined
			}),
		'fails its check'
	],
	['generated file changed', null, 'differs from what the generator writes']
];

let survived = 0;
for (const [name, mutate, expect] of MUTANTS) {
	const dir = mkdtempSync(join(tmpdir(), 'sample-mutant-'));
	try {
		cpSync(sample, dir, { recursive: true });
		if (mutate) {
			const path = join(dir, 'labels.json');
			const labels = JSON.parse(readFileSync(path, 'utf8'));
			mutate(labels);
			writeFileSync(path, JSON.stringify(labels));
		} else {
			const path = join(dir, 'files', 'lemon-rice.pdf');
			const bytes = readFileSync(path);
			bytes[bytes.length - 10] ^= 1;
			writeFileSync(path, bytes);
		}
		const run = spawnSync(process.execPath, [join(dir, 'tools', 'check.mjs')], {
			encoding: 'utf8'
		});
		const line = run.stderr.split('\n').find((l) => l.includes(expect));
		if (run.status === 1 && line) console.log(`caught   ${name}\n         ${line}`);
		else {
			survived++;
			console.log(`MISSED   ${name} (exit ${run.status})\n${run.stderr}`);
		}
	} finally {
		rmSync(dir, { recursive: true, force: true });
	}
}
console.log(
	survived
		? `${survived} of ${MUTANTS.length} breaks went unreported`
		: `all ${MUTANTS.length} breaks reported`
);
process.exit(survived ? 1 : 0);
