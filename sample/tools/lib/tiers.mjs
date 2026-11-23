// The test for `certain` in sample/FORMAT.md, "Tiers". Only the characters of
// an identifier, or the directory's exact code, make a claim certain.
import { ibanBankCode, KINDS, parseMrz } from './ids.mjs';

export const TIERS = new Set(['certain', 'likely']);

// The reasons that name the bank by a code inside the identifier, with the
// identifier kind each one reads.
export const CODE_REASONS = new Map([
	['iban-bank-code', 'iban'],
	['bic-institution', 'bic'],
	['ifsc-prefix', 'ifsc']
]);

// The institution an IFSC, BIC or IBAN names by its own characters, or null
// when the kind names none or the IBAN's country is one whose bank code place
// is not known here.
export function institution({ kind, value }) {
	if (kind === 'ifsc') return `ifsc ${value.slice(0, 4)}`;
	if (kind === 'bic') return `bic ${value.slice(0, 6)}`;
	if (kind === 'iban') {
		const { country, code } = ibanBankCode(value);
		return code === null ? null : `iban ${country} ${code}`;
	}
	return null;
}

// Whether the directory entry holds the code exactly: an IFSC's four letters,
// or a BIC's first eight characters (bank, country and location).
function directoryHolds(entry, { kind, value }) {
	if (!entry) return false;
	if (kind === 'ifsc') return entry.codes.ifsc_prefix.includes(value.slice(0, 4));
	return entry.codes.bic.some((b) => b.slice(0, 8) === value.slice(0, 8));
}

// A link may be certain when one of its reasons passes its test: an IBAN whose
// check digits pass, for iban-bank-code; an IFSC or BIC that the directory
// entry on the key's card holds exactly, in a run with the directory on.
export function mayBeCertainLink({ identifier, reasons, card }, mode, entries) {
	return reasons.some((r) => {
		if (CODE_REASONS.get(r) !== identifier.kind) return false;
		if (r === 'iban-bank-code') return KINDS.iban.valid(identifier.value);
		return mode === 'on' && directoryHolds(entries.get(card.directory), identifier);
	});
}

// A passport's holder, number and expiry may be certain when they come with a
// passport machine-readable zone whose check digits all pass.
export const mayBeCertainPassport = (mrz) => {
	const zone = mrz?.kind === 'mrz' ? parseMrz(mrz.value) : null;
	return zone?.type === 'passport' && zone.failed.length === 0;
};
