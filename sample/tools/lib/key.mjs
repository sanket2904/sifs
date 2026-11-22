// The directory's match key for a name. Two names with the same key are the
// same name; a key held by two directory entries is a clash.
const DROP_WORDS = new Set(['the', 'ltd', 'limited', 'plc', 'inc', 'ag', 'sa', 'nv']);

export function matchKey(name) {
	return name
		.normalize('NFKC')
		.toLowerCase()
		.replaceAll('株式会社', ' ')
		.split(/[^\p{L}\p{M}\p{N}]+/u)
		.filter((word) => word && !DROP_WORDS.has(word))
		.join('');
}
