// Package orgkey spells organisation keys, the extractor's identity for an
// organisation (sample/FORMAT.md, "No merging" and "Code keys"). A key is an
// institution code's key, the same in every document, or a document key,
// which stands for a printed name inside one document. This package is the
// only place a key is spelt.
package orgkey

import (
	"strings"
	"unicode"

	"golang.org/x/text/cases"
	"golang.org/x/text/language"
	"golang.org/x/text/unicode/norm"
)

// Key is an organisation's key. The zero Key is no key.
type Key struct {
	s    string
	doc  string // the document a document key belongs to; "" for a code key
	part string // a document key's name part
}

func (k Key) String() string { return k.s }

func (k Key) IsZero() bool { return k.s == "" }

// IsName reports whether k is a document key, a printed name's.
func (k Key) IsName() bool { return k.doc != "" }

// Part is a document key's name part, the match key it is spelt from.
func (k Key) Part() string { return k.part }

// IFSC is the key of the bank an IFSC names: its first four letters.
func IFSC(ifsc string) Key { return Key{s: "IFSC " + ifsc[:4]} }

// BIC is the key of the institution a BIC names: its first eight characters.
func BIC(bic string) Key { return Key{s: "BIC " + bic[:8]} }

// IBAN is the key of the bank an IBAN names: its country and bank code.
func IBAN(country, bank string) Key { return Key{s: "IBAN " + country + " " + bank} }

// Name is the document key of the printed names texts, all of one
// organisation in document doc. Its name part is the longest of their match
// keys in characters, between two of one length the first in code-point
// order, so the key depends only on the set of texts. It fails when no text
// has a match key.
func Name(doc string, texts ...string) (Key, bool) {
	part := ""
	for _, t := range texts {
		part = spelling(part, MatchKey(t))
	}
	if part == "" {
		return Key{}, false
	}
	return Key{s: "name:" + doc + ":" + part, doc: doc, part: part}, true
}

// spelling is the name part two match keys give together: the longer, and
// between two of one length the first in code-point order.
func spelling(a, b string) string {
	na, nb := len([]rune(a)), len([]rune(b))
	if na > nb || na == nb && a <= b {
		return a
	}
	return b
}

var stopWords = map[string]bool{"the": true, "ltd": true, "limited": true, "plc": true, "inc": true, "ag": true, "sa": true, "nv": true}

// MatchKey is the match key of a name, as the scorer's matchKey
// (sample/tools/lib/key.mjs) spells it: compatibility-folded, lower case,
// 株式会社 read as a word break, split on everything that is not a letter,
// mark or number, the legal-form words dropped, and the rest joined.
func MatchKey(s string) string {
	s = strings.ReplaceAll(Lower(Fold(s)), "株式会社", " ")
	var b strings.Builder
	for _, w := range strings.FieldsFunc(s, func(r rune) bool {
		return !unicode.IsLetter(r) && !unicode.IsMark(r) && !unicode.IsNumber(r)
	}) {
		if !stopWords[w] {
			b.WriteString(w)
		}
	}
	return b.String()
}

// Fold is Unicode compatibility folding, NFKC, as the scorer's
// normalize('NFKC') gives it.
func Fold(s string) string { return norm.NFKC.String(s) }

// Lower is lower case as JavaScript's toLowerCase gives it: Unicode's full
// mapping, with no language's own rules. Go's strings.ToLower differs in
// two places: it lowers a capital sigma that ends a word to σ, not ς, and a
// dotted capital I to i, not i and a combining dot above.
func Lower(s string) string { return cases.Lower(language.Und).String(s) }

// Upper is upper case as JavaScript's toUpperCase gives it: Unicode's full
// mapping, so ß is SS, where Go's strings.ToUpper leaves it.
func Upper(s string) string { return cases.Upper(language.Und).String(s) }
