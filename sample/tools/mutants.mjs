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
		'directory lookup changed to another bank',
		(l) => (doc(l, 'hdfc-branch-letter').orgs[0].directory.id = 'Q1446319'),
		'directory exact Q1446319, but the directory gives Q631047'
	],
	[
		'mention settled as an organisation it does not name',
		(l) => (doc(l, 'hdfc-branch-letter').orgs[0].resolved_by_document.org = 'hdfc-ltd'),
		'is not a name of hdfc-ltd'
	],
	[
		'mention settled as an organisation that does not exist',
		(l) => (doc(l, 'transfer-untied').orgs[1].resolved_by_document.org = 'no-such-bank'),
		'names no organisation'
	],
	[
		'alias clash looked up as one bank',
		(l) => (doc(l, 'household-notes').orgs[0].directory = { lookup: 'exact', id: 'Q2004439' }),
		'directory exact Q2004439, but the directory gives Q2004439, Q28457664'
	],
	[
		'unsettled short name settled with no evidence',
		(l) =>
			(doc(l, 'household-notes').orgs[2].resolved_by_document = {
				org: 'hdfc-bank',
				evidence: [{ by: 'name' }]
			}),
		'"HDFC" is not a name of hdfc-bank'
	],
	[
		"document evidence pointed at the other bank's code",
		(l) =>
			(doc(l, 'neft-receipt-tied').orgs[0].resolved_by_document.evidence = [
				{ by: 'code', identifier: 'i4' }
			]),
		'ICIC0SAMPL2 does not spell "HDFC"'
	],
	[
		'look-alike label removed',
		(l) => doc(l, 'school-fee-banks').not_orgs.pop(),
		'which no org or not_org label of Q2004439 covers'
	],
	[
		'exclusion of the second alias holder removed',
		(l) => {
			const d = doc(l, 'hdfc-statement-sep');
			d.not_orgs = d.not_orgs.filter((n) => n.entry !== 'Q1446319');
		},
		'which no org or not_org label of Q1446319 covers'
	],
	[
		'merchant left unlabelled',
		(l) => {
			const d = doc(l, 'natwest-statement');
			d.orgs.pop();
			d.not_links = d.not_links.filter((n) => n.org !== 'example-cafe');
		},
		'planted org "Example Cafe" at page 1'
	],
	[
		'organisation span clipped to the directory name',
		(l) =>
			Object.assign(doc(l, 'deutsche-bank-brief').orgs[1], {
				start: 214,
				text: 'Bundesbank',
				directory: { lookup: 'exact', id: 'Q162222' }
			}),
		'planted org "Deutschen Bundesbank"'
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
		'account label truncated by one digit',
		(l) =>
			Object.assign(doc(l, 'neft-receipt-tied').identifiers[2], {
				end: 292,
				text: '00210123456',
				value: '00210123456'
			}),
		'covers part of a token, not the whole token'
	],
	[
		'decoy label removed',
		(l) => doc(l, 'rent-receipt').not_identifiers.pop(),
		'which no identifier or not_identifier label holds whole'
	],
	[
		'sort code left unlabelled',
		(l) => {
			const d = doc(l, 'natwest-statement');
			d.identifiers = d.identifiers.filter((i) => i.id !== 'i4');
			d.links = d.links.filter((k) => k.identifier !== 'i4');
			d.not_links = d.not_links.filter((k) => k.identifier !== 'i4');
		},
		'planted identifier "60-16-13"'
	],
	[
		'domestic account not tied to its IBAN',
		(l) => delete doc(l, 'rib-fr').identifiers[2].same_account_as,
		'so same_account_as is i1'
	],
	[
		'IBAN linked to the wrong bank',
		(l) => (doc(l, 'commerzbank-kontoauszug').links[0].org = 'deutsche-bank'),
		'is not a bank code of deutsche-bank'
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
				org: 'mizuho-bank',
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
				org: 'natwest',
				mention: 'o1',
				reasons: ['paired-code'],
				via: 'i3'
			}),
		'is not on the same or the next line'
	],
	[
		'account paired with the nearer code of the next row',
		(l) =>
			(doc(l, 'transfer-rows').links[1] = {
				identifier: 'i2',
				org: 'icici-bank',
				mention: null,
				reasons: ['paired-code'],
				via: 'i3'
			}),
		'has a code of its own on its line'
	],
	[
		'wrong-bank pair removed',
		(l) => doc(l, 'neft-receipt-tied').not_links.splice(4, 1),
		'i2: has neither a link nor a not_link to icici-bank'
	],
	[
		'not_link reason that is false',
		(l) => (doc(l, 'transfer-untied').not_links[2].why = 'invalid-identifier'),
		'invalid-identifier, but the identifier passes its check'
	],
	[
		'code-backed exclusion given the weaker reason',
		(l) => (doc(l, 'neft-receipt-tied').not_links[0].why = 'held-at-other'),
		'so why is code-names-other-bank'
	],
	[
		'statement period changed',
		(l) => (doc(l, 'hdfc-statement-sep').statement.to = '2026-09-29'),
		'the text gives 2026-09-01 and 2026-09-30'
	],
	[
		'statement period pointed at the next-statement reminder',
		(l) =>
			Object.assign(doc(l, 'hdfc-statement-sep').statement, {
				start: 630,
				end: 656,
				text: '01 Oct 2026 to 31 Oct 2026',
				from: '2026-10-01',
				to: '2026-10-31'
			}),
		"is not the statement's own period field"
	],
	[
		'statement issuer set to a merchant',
		(l) => (doc(l, 'hdfc-card-statement').statement.issuer = 'o2'),
		'is not the first organisation the document names'
	],
	[
		'latest statement answer changed',
		(l) => (l.answers.latest_statement[0].document = 'hdfc-statement-jul'),
		'latest_statement should be'
	],
	[
		'domestic spelling dropped from the answer',
		(l) =>
			(l.answers.latest_statement.find((a) => a.document === 'natwest-statement').spellings = [
				'GB29NWBK60161331926819'
			]),
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
