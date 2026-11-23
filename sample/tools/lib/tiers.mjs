// The test for `certain` in sample/FORMAT.md, "Tiers". Only the characters of
// an identifier, or the directory's exact code, make a claim certain.
import { ibanBankCode, KINDS, parseMrz } from './ids.mjs';
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
const nameLookup = (entries, text) =>
	[...entries.values()]
		.filter((e) => e.names.some((n) => n.key === matchKey(text)))
		.map((e) => e.id);

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

// A passport's holder, number and expiry may be certain when they come with a
// passport machine-readable zone whose check digits all pass. The number rests
// on the zone, not on the printed number, so it must be the zone's number.
export function mayBeCertainPassport(fact, { mrz, number }) {
	if (fact === 'current') return false;
	const zone = mrz?.kind === 'mrz' ? parseMrz(mrz.value) : null;
	if (zone?.type !== 'passport' || zone.failed.length) return false;
	return fact !== 'number' || number?.value === zone.number;
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
