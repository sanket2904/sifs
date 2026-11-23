// The test for `certain` in sample/FORMAT.md, "Tiers". Only the characters of
// an identifier, or the directory's exact code, make a claim certain.
import { ibanBankCode, ibanBankCodeLength, KINDS, parseMrz } from './ids.mjs';
import { matchKey } from './key.mjs';

export const TIERS = new Set(['certain', 'likely']);

// The reasons that name the bank by a code inside the identifier, with the
// identifier kind each one reads.
export const CODE_REASONS = new Map([
	['iban-bank-code', 'iban'],
	['bic-institution', 'bic'],
	['ifsc-prefix', 'ifsc']
]);

// The canonical key of the institution an IFSC, BIC or IBAN names by its own
// characters (FORMAT.md, "Code keys"), or null when the kind names none or the
// IBAN's country is one whose bank code place is not known here.
export function codeKey({ kind, value }) {
	const v = value.toUpperCase();
	if (kind === 'ifsc') return `IFSC ${v.slice(0, 4)}`;
	if (kind === 'bic') return `BIC ${v.slice(0, 8)}`;
	if (kind === 'iban') {
		const { country, code } = ibanBankCode(v);
		return code === null ? null : `IBAN ${country} ${code}`;
	}
	return null;
}

// Whether a key is spelt as an institution code's key ("Code keys"): an IFSC's
// four letters, a BIC's six letters and two letters or digits, an IBAN's
// country and a bank code of the length iban-registry.json gives it.
export function isCodeKey(key) {
	if (/^IFSC [A-Z]{4}$/.test(key) || /^BIC [A-Z]{6}[A-Z0-9]{2}$/.test(key)) return true;
	const iban = /^IBAN ([A-Z]{2}) ([A-Z0-9]+)$/.exec(key);
	return iban !== null && ibanBankCodeLength(iban[1]) === iban[2].length;
}

// A document key stands for one printed name inside one document, or for an
// IBAN of a country iban-registry.json does not have (FORMAT.md, "No
// merging"): name:<document id>:<the match key of the printed text>.
// documentOfKey gives the document a key names, or null when the key is not
// spelt as a document key.
export const documentKey = (document, text) => `name:${document}:${matchKey(text)}`;
const DOCUMENT_KEY = /^name:([a-z0-9-]+):([\p{L}\p{M}\p{N}]+)$/u;
export const documentOfKey = (key) => DOCUMENT_KEY.exec(key)?.[1] ?? null;
export const nameOfKey = (key) => DOCUMENT_KEY.exec(key)?.[2] ?? null;

// The key a link by the identifier's own code reason goes to: its code's key,
// or for an IBAN of a country the table does not have, the document key spelt
// from the IBAN. null for a kind that names no institution.
export const ownCodeKey = (identifier, document) =>
	codeKey(identifier) ??
	(identifier.kind === 'iban' ? documentKey(document, identifier.value) : null);

// Whether the directory entry holds the identifier's institution code exactly:
// an IFSC's four letters, a BIC's first eight characters, an IBAN's country and
// bank code.
function holdsCode(entry, { kind, value }) {
	if (!entry) return false;
	const v = value.toUpperCase();
	if (kind === 'ifsc') return entry.codes.ifsc_prefix.includes(v.slice(0, 4));
	if (kind === 'bic') return entry.codes.bic.some((b) => b.slice(0, 8) === v.slice(0, 8));
	if (kind === 'iban') {
		const { country, code } = ibanBankCode(v);
		return entry.codes.iban_bank_code.some((c) => c.country === country && c.code === code);
	}
	return false;
}

// The directory entries a code's exact lookup gives, and a printed name's.
const codeLookup = (entries, identifier) =>
	[...entries.values()].filter((e) => holdsCode(e, identifier)).map((e) => e.id);
const nameLookup = (entries, text) => {
	const key = matchKey(text);
	return [...entries.values()].filter((e) => e.names.some((n) => n.key === key)).map((e) => e.id);
};

// An identifier's kind may be certain when its own strong check proves it: an
// IBAN's check digits, a machine-readable zone's check digits, or an IFSC or
// BIC that the directory holds exactly, in a run with the directory on.
export function mayBeCertainIdentifier(identifier, mode, entries) {
	const { kind, value } = identifier;
	if (kind === 'iban' || kind === 'mrz') return KINDS[kind].valid(value) === true;
	if (kind === 'ifsc' || kind === 'bic')
		return mode === 'on' && KINDS[kind].valid(value) && codeLookup(entries, identifier).length > 0;
	return false;
}

// A link may be certain when one of its reasons passes its test: an IBAN whose
// check digits pass, for iban-bank-code; an IFSC or BIC that the directory
// entry on the key's card holds exactly, in a run with the directory on. Each
// of these tests proves the identifier itself, which must pass its own test.
export function mayBeCertainLink({ identifier, reasons, card }, mode, entries) {
	if (!mayBeCertainIdentifier(identifier, mode, entries)) return false;
	return reasons.some((r) => {
		if (CODE_REASONS.get(r) !== identifier.kind) return false;
		if (r === 'iban-bank-code') return true;
		return mode === 'on' && holdsCode(entries.get(card.directory), identifier);
	});
}

// The zone's date of expiry (YYMMDD) with the century the page prints, or null
// when the printed date does not prove it. The printed text must hold one
// four-digit year ending in the zone's two digits, and besides it only the
// zone's day, or its day and month in either order. The zone's check digit
// proves its six digits, never the century.
export function printedExpiry(text, yymmdd) {
	const numbers = text.normalize('NFKC').match(/\d+/g) ?? [];
	const years = numbers.filter((n) => n.length === 4);
	const rest = numbers.filter((n) => n.length !== 4);
	if (years.length !== 1 || years[0].slice(2) !== yymmdd.slice(0, 2)) return null;
	if (rest.some((n) => n.length > 2)) return null;
	const [month, day] = [Number(yymmdd.slice(2, 4)), Number(yymmdd.slice(4, 6))];
	const given = rest.map(Number).sort((a, b) => a - b);
	const want = rest.length === 1 ? [day] : [day, month].sort((a, b) => a - b);
	if (given.length !== want.length || given.some((n, i) => n !== want[i])) return null;
	return `${years[0]}-${yymmdd.slice(2, 4)}-${yymmdd.slice(4, 6)}`;
}

// A passport's holder, number and expiry may be certain when each is what the
// passport's machine-readable zone says, and the zone is a certain passport
// zone whose check digits all pass. A certain zone that is not the page's own
// fails the run as a wrong identifier, so only the page's zone proves a fact.
// The number rests on the zone, not on the printed number. A name ignores case
// and spacing. The zone gives an expiry's year in two digits, so a certain
// expiry also cites the printed date that gives its century (expiryAt).
const name = (v) => v.normalize('NFKC').toUpperCase().replace(/\s+/gu, ' ').trim();
export function mayBeCertainPassport(fact, { mrz, number, holder, expiry, expiryAt }) {
	if (fact === 'current') return false;
	if (mrz?.tier !== 'certain') return false;
	const zone = mrz.kind === 'mrz' ? parseMrz(mrz.value) : null;
	if (zone?.type !== 'passport' || zone.failed.length) return false;
	if (fact === 'number') return number?.value === zone.number;
	if (fact === 'holder')
		return name(holder.surname) === name(zone.surname) && name(holder.given) === name(zone.given);
	if (!expiryAt) return false;
	return printedExpiry(expiryAt.text, zone.expiry) === expiry;
}

// A same_org row may be certain when its reason is directory, the directory is
// on, and one key's printed names and the other key's codes each look up to
// exactly one entry, the same one. names: the texts of the document's mentions
// on one key; codes: the identifiers linked to the other key by their own code
// reason in the document.
export function mayBeCertainJoin({ reason, names, codes }, mode, entries) {
	if (reason !== 'directory' || mode !== 'on' || !names.length || !codes.length) return false;
	const found = [
		...names.map((t) => nameLookup(entries, t)),
		...codes.map((c) => codeLookup(entries, c))
	];
	return found.every((ids) => ids.length === 1 && ids[0] === found[0][0]);
}
