// Identifier kinds: how a value is written down (normalize) and whether it
// passes its own check (valid). A kind with valid === null has no check.
const IBAN_LENGTH = { CH: 21, DE: 22, ES: 24, FR: 27, GB: 22, IE: 22, NL: 18 };

const mod97 = (digits) => [...digits].reduce((r, d) => (r * 10 + Number(d)) % 97, 0);
const ibanDigits = (s) =>
	[...s].map((c) => (/[A-Z]/.test(c) ? String(c.charCodeAt(0) - 55) : c)).join('');

export function ibanCheckDigits(country, bban) {
	return String(98 - mod97(ibanDigits(bban + country + '00'))).padStart(2, '0');
}

function validIban(v) {
	return (
		/^[A-Z]{2}\d{2}[A-Z0-9]+$/.test(v) &&
		IBAN_LENGTH[v.slice(0, 2)] === v.length &&
		mod97(ibanDigits(v.slice(4) + v.slice(0, 4))) === 1
	);
}

export function luhnOk(digits) {
	let sum = 0;
	for (let i = 0; i < digits.length; i++) {
		let d = Number(digits[digits.length - 1 - i]);
		if (i % 2 === 1) d = d * 2 > 9 ? d * 2 - 9 : d * 2;
		sum += d;
	}
	return sum % 10 === 0;
}

const MRZ_VALUE = (c) => (c === '<' ? 0 : /\d/.test(c) ? Number(c) : c.charCodeAt(0) - 55);
export const mrzCheck = (field) =>
	String([...field].reduce((sum, c, i) => sum + MRZ_VALUE(c) * [7, 3, 1][i % 3], 0) % 10);

// TD3 passport (P) or MRV-A visa (V): two lines of 44 characters.
export function parseMrz(value) {
	const lines = value.split('\n');
	if (lines.length !== 2 || !lines.every((l) => /^[A-Z0-9<]{44}$/.test(l))) return null;
	const [l1, l2] = lines;
	const type = l1[0] === 'P' ? 'passport' : l1[0] === 'V' ? 'visa' : null;
	if (!type) return null;
	const [surname, given = ''] = l1.slice(5).split('<<');
	const fields = {
		type,
		issuer: l1.slice(2, 5),
		surname: surname.replaceAll('<', ' ').trim(),
		given: given.replaceAll('<', ' ').trim(),
		number: l2.slice(0, 9).replaceAll('<', ''),
		birth: l2.slice(13, 19),
		expiry: l2.slice(21, 27)
	};
	const checks = [
		['number', l2.slice(0, 9), l2[9]],
		['birth', l2.slice(13, 19), l2[19]],
		['expiry', l2.slice(21, 27), l2[27]]
	];
	if (type === 'passport') {
		const personal = l2.slice(28, 42);
		checks.push(['personal', personal, /^<+$/.test(personal) && l2[42] === '<' ? '0' : l2[42]]);
		checks.push(['composite', l2.slice(0, 10) + l2.slice(13, 20) + l2.slice(21, 43), l2[43]]);
	}
	fields.failed = checks
		.filter(([, field, digit]) => mrzCheck(field) !== digit)
		.map(([name]) => name);
	return fields;
}

// The French RIB key: 97 minus (89 x bank code + 15 x branch code + 3 x account)
// mod 97, with the account's letters read as digits (A and J are 1, B, K and S
// are 2, and so on).
const RIB_LETTERS = {
	A: 1,
	J: 1,
	B: 2,
	K: 2,
	S: 2,
	C: 3,
	L: 3,
	T: 3,
	D: 4,
	M: 4,
	U: 4,
	E: 5,
	N: 5,
	V: 5,
	F: 6,
	O: 6,
	W: 6,
	G: 7,
	P: 7,
	X: 7,
	H: 8,
	Q: 8,
	Y: 8,
	I: 9,
	R: 9,
	Z: 9
};
export function ribKey(bank, branch, account) {
	const digits = [...account].map((c) => RIB_LETTERS[c] ?? c).join('');
	const sum = 89n * BigInt(bank) + 15n * BigInt(branch) + 3n * BigInt(digits);
	return String(97n - (sum % 97n)).padStart(2, '0');
}

export const KINDS = {
	iban: { normalize: (t) => t.replaceAll(' ', ''), valid: validIban },
	bic: { normalize: (t) => t, valid: (v) => /^[A-Z]{4}[A-Z]{2}[A-Z0-9]{2}([A-Z0-9]{3})?$/.test(v) },
	card: {
		normalize: (t) => t.replaceAll(' ', ''),
		valid: (v) => /^\d{12,19}$/.test(v) && luhnOk(v)
	},
	ifsc: { normalize: (t) => t, valid: (v) => /^[A-Z]{4}0[A-Z0-9]{6}$/.test(v) },
	in_pan: { normalize: (t) => t, valid: (v) => /^[A-Z]{3}[ABCFGHJLPT][A-Z]\d{4}[A-Z]$/.test(v) },
	mrz: { normalize: (t) => t, valid: (v) => parseMrz(v)?.failed.length === 0 },
	gb_sort_code: { normalize: (t) => t.replaceAll('-', ''), valid: (v) => /^\d{6}$/.test(v) },
	fr_bank_code: { normalize: (t) => t, valid: (v) => /^\d{5}$/.test(v) },
	fr_branch_code: { normalize: (t) => t, valid: (v) => /^\d{5}$/.test(v) },
	// Shape only here; check.mjs also recomputes the key from the codes beside it.
	fr_rib_key: { normalize: (t) => t, valid: (v) => /^\d{2}$/.test(v) },
	account: { normalize: (t) => t.replaceAll(' ', ''), valid: null },
	passport_number: { normalize: (t) => t, valid: null }
};
