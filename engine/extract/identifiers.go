package extract

import (
	"regexp"
	"strings"
	"unicode"

	"github.com/sanket2904/sifs/engine/extract/orgkey"
)

// Identifier kinds, as the label schema names them.
const (
	KindIBAN       = "iban"
	KindBIC        = "bic"
	KindIFSC       = "ifsc"
	KindCard       = "card"
	KindPAN        = "in_pan"
	KindMRZ        = "mrz"
	KindSortCode   = "gb_sort_code"
	KindFRBank     = "fr_bank_code"
	KindFRBranch   = "fr_branch_code"
	KindFRKey      = "fr_rib_key"
	KindAccount    = "account"
	KindPassportNo = "passport_number"
)

// Identifier is one identifier found in a document.
type Identifier struct {
	Span
	ID    string
	Kind  string
	Value string
	Valid *bool
	Line  Line // the line it starts on
	// Code is the key of the institution a valid IFSC, BIC or IBAN names by
	// its own characters ("Code keys"), or zero.
	Code  orgkey.Key
	proof proof // why its kind is what it is
}

// codeKey spells the key of the institution a valid identifier carries
// inside it: an IFSC's first four letters, a BIC's first eight characters,
// an IBAN's country and bank code.
func codeKey(kind, value string, valid *bool, reg *IBANRegistry) orgkey.Key {
	if valid == nil || !*valid {
		return orgkey.Key{}
	}
	switch kind {
	case KindIFSC:
		return orgkey.IFSC(value)
	case KindBIC:
		return orgkey.BIC(value)
	case KindIBAN:
		return reg.codeKey(value)
	}
	return orgkey.Key{}
}

// kindProof is what a document alone proves about an identifier's kind: an
// IBAN by the registry and its check digits, a zone by its check digits.
// The directory's proof of an IFSC or BIC comes later, in Run.
func kindProof(kind, value string, reg *IBANRegistry) proof {
	switch {
	case kind == KindIBAN && reg.proves(value):
		return ibanRegistered
	case kind == KindMRZ:
		l1, l2, _ := strings.Cut(value, "\n")
		if mrzValid(l1, l2) {
			return zoneChecked
		}
	}
	return read
}

func (id Identifier) isAccount() bool {
	return id.Kind == KindAccount || id.Kind == KindIBAN || id.Kind == KindCard
}

func (id Identifier) isCode() bool { return id.Kind == KindIFSC || id.Kind == KindBIC }

// namesItsInstitution: an IBAN, a BIC or an IFSC, which carries its
// institution's code in its own characters ("Code keys").
func (id Identifier) namesItsInstitution() bool { return id.Kind == KindIBAN || id.isCode() }

func ptr[T any](v T) *T { return &v }

// found is a match inside one line, as byte offsets into the line's text.
type found struct {
	b0, b1 int
	value  string
	valid  *bool
}

// scheme finds one identifier kind in a line. Schemes run in table order and
// a later scheme never claims text an earlier one took.
type scheme struct {
	kind string
	find func(l Line, prior []Identifier) []found
}

// schemes run after the IBAN scheme, which needs the registry.
var schemes = []scheme{
	{KindBIC, findBIC},
	{KindIFSC, findIFSC},
	{KindCard, findCard},
	{KindPAN, findPAN},
	{KindSortCode, labelled(`(?i)\bsort\s*code\b\s*:?\s*`, `[0-9]{2}-[0-9]{2}-[0-9]{2}\b`, digitsOnly, shapeValid)},
	{KindFRBank, labelled(`(?i)\bcode\s+banque\s*:?\s*`, `[0-9]{5}\b`, same, shapeValid)},
	{KindFRBranch, labelled(`(?i)\bcode\s+guichet\s*:?\s*`, `[0-9]{5}\b`, same, shapeValid)},
	{KindFRKey, findRIBKey},
	{KindPassportNo, labelled(`(?i)(?:\bpassport\s*(?:no\.?|number|nr\.?|n°)|\breisepass(?:-?nr\.?|nummer)|n°\s*de\s*passeport|旅券番号|पासपोर्ट\s*(?:संख्या|नंबर))\s*:?\s*`, `[A-Z0-9]{6,9}\b`, same, noCheck)},
	{KindAccount, findLabelledAccount},
	{KindAccount, findRowAccount},
}

func same(s string) string { return s }

func digitsOnly(s string) string {
	return strings.Map(func(r rune) rune {
		if r >= '0' && r <= '9' {
			return r
		}
		return -1
	}, s)
}

func shapeValid(string) *bool { return ptr(true) }
func noCheck(string) *bool    { return nil }

// labelled finds a value right after a field label on the same line.
func labelled(label, value string, norm func(string) string, check func(string) *bool) func(Line, []Identifier) []found {
	re := regexp.MustCompile(label + `(` + value + `)`)
	return func(l Line, _ []Identifier) []found {
		var out []found
		for _, m := range re.FindAllStringSubmatchIndex(l.Text, -1) {
			v := norm(l.Text[m[2]:m[3]])
			out = append(out, found{m[2], m[3], v, check(v)})
		}
		return out
	}
}

var reIBANStart = regexp.MustCompile(`\b[A-Z]{2}[0-9]{2}[ ]?[A-Z0-9]`)

// findIBAN reads the registry length of the country's IBAN, allowing single
// spaces between characters, and requires the IBAN to end at a word end.
func (reg *IBANRegistry) findIBAN(l Line, _ []Identifier) []found {
	var out []found
	t := l.Text
	for _, m := range reIBANStart.FindAllStringIndex(t, -1) {
		c, ok := reg.countries[t[m[0]:m[0]+2]]
		if !ok {
			continue
		}
		var b strings.Builder
		i := m[0]
		for i < len(t) && b.Len() < c.length {
			ch := t[i]
			if isUpperAlnum(ch) {
				b.WriteByte(ch)
			} else if !(ch == ' ' && i+1 < len(t) && isUpperAlnum(t[i+1]) && b.Len() > 0) {
				break
			}
			i++
		}
		if b.Len() != c.length || (i < len(t) && isUpperAlnum(t[i])) {
			continue
		}
		v := b.String()
		out = append(out, found{m[0], i, v, ptr(reg.proves(v))})
	}
	return out
}

func isUpperAlnum(c byte) bool { return c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' }

var (
	reBICLabel = regexp.MustCompile(`(?i)\b(?:bic|swift)`)
	reBIC      = regexp.MustCompile(`\b[A-Z]{6}[A-Z0-9]{2}(?:[A-Z0-9]{3})?\b`)
)

// findBIC takes a BIC only after a BIC or SWIFT label on its line: eight
// capitals alone are too often a word.
func findBIC(l Line, _ []Identifier) []found {
	lab := reBICLabel.FindStringIndex(l.Text)
	if lab == nil {
		return nil
	}
	var out []found
	for _, m := range reBIC.FindAllStringIndex(l.Text, -1) {
		if m[0] < lab[1] {
			continue
		}
		v := l.Text[m[0]:m[1]]
		out = append(out, found{m[0], m[1], v, ptr(isoCountries[v[4:6]])})
	}
	return out
}

var (
	reIFSC     = regexp.MustCompile(`\b[A-Z]{4}0[A-Z0-9]{6}\b`)
	reIFSCScan = regexp.MustCompile(`\b[A-Z]{4}[0OØ][A-Z0-9]{6}\b`)
)

// findIFSC finds the Indian branch code. Its fifth character is always the
// digit zero, so on a scanned page a letter O read there is that zero.
func findIFSC(l Line, _ []Identifier) []found {
	re := reIFSC
	if l.Scanned {
		re = reIFSCScan
	}
	var out []found
	for _, m := range re.FindAllStringIndex(l.Text, -1) {
		raw := []rune(l.Text[m[0]:m[1]])
		raw[4] = '0'
		out = append(out, found{m[0], m[1], string(raw), ptr(true)})
	}
	return out
}

var (
	reCard      = regexp.MustCompile(`\b[0-9]{4}(?:[ -][0-9]{4}){3}\b|\b[0-9]{4}[ -][0-9]{6}[ -][0-9]{5}\b`)
	reCardLabel = regexp.MustCompile(`(?i)\bcard\b|karte|carte|カード|कार्ड`)
)

// findCard takes a grouped 15- or 16-digit number as a card when its line
// says card, or when it passes Luhn. A number whose own field label names
// something else ("Invoice number:") is that thing, whatever its shape.
func findCard(l Line, _ []Identifier) []found {
	labelled := reCardLabel.MatchString(l.Text)
	var out []found
	for _, m := range reCard.FindAllStringIndex(l.Text, -1) {
		if label := fieldLabel(l.Text[:m[0]]); label != "" && !reCardLabel.MatchString(label) {
			continue
		}
		v := digitsOnly(l.Text[m[0]:m[1]])
		ok := luhn(v)
		if labelled || ok {
			out = append(out, found{m[0], m[1], v, ptr(ok)})
		}
	}
	return out
}

var (
	rePAN      = regexp.MustCompile(`\b[A-Z]{5}[0-9]{4}[A-Z]\b`)
	rePANLabel = regexp.MustCompile(`\bPAN\b|पैन|(?i:permanent account number)`)
)

// findPAN takes a PAN-shaped value only on a line whose field says PAN: the
// same shape in a TAN field is no PAN.
func findPAN(l Line, _ []Identifier) []found {
	if !rePANLabel.MatchString(l.Text) {
		return nil
	}
	var out []found
	for _, m := range rePAN.FindAllStringIndex(l.Text, -1) {
		v := l.Text[m[0]:m[1]]
		out = append(out, found{m[0], m[1], v, ptr(panValid(v))})
	}
	return out
}

var reRIBKey = regexp.MustCompile(`(?i)\bcl[ée]\s*(?:rib)?\s*:?\s*([0-9]{2})\b`)

// findRIBKey checks the clé RIB against the bank, branch and account codes
// on its own line; with any of them missing its check cannot run.
func findRIBKey(l Line, prior []Identifier) []found {
	var bank, branch, account string
	for _, id := range prior {
		if id.Line.Page != l.Page || id.Line.Index != l.Index {
			continue
		}
		switch id.Kind {
		case KindFRBank:
			bank = id.Value
		case KindFRBranch:
			branch = id.Value
		}
	}
	if m := reFRAccount.FindStringSubmatch(l.Text); m != nil {
		account = m[1]
	}
	var out []found
	for _, m := range reRIBKey.FindAllStringSubmatchIndex(l.Text, -1) {
		v := l.Text[m[2]:m[3]]
		var valid *bool
		if bank != "" && branch != "" && account != "" {
			if k, ok := ribKey(bank, branch, account); ok {
				valid = ptr(k == v)
			}
		}
		out = append(out, found{m[2], m[3], v, valid})
	}
	return out
}

var reFRAccount = regexp.MustCompile(`(?i)num[ée]ro\s+de\s+compte\s*:?\s*([0-9A-Z]{11})\b`)

var (
	reAccountValue = regexp.MustCompile(`\b[0-9][0-9A-Z]{5,19}\b`)
	reAccountWord  = regexp.MustCompile(`(?i)\b(?:account|a/c|acct|konto|kontonummer|compte)\b|खाता|口座`)
	reNotAccount   = regexp.MustCompile(`(?i)card|policy|consumer|customer|invoice|reference|rechnung|transaction|visa|phone|telefon|ifsc|bic|swift|iban|sort`)
)

// findLabelledAccount takes a number as an account when the field label
// right before it names an account: "Account number:", "खाता संख्या:",
// "口座番号:". One word of account type may sit between label and number.
// A number written in groups ("1234 5678 90") is one account when its
// groups run to the end of the field; its value drops the spaces.
func findLabelledAccount(l Line, _ []Identifier) []found {
	var out []found
	isAccount := func(b0 int) bool {
		label := fieldLabel(l.Text[:b0])
		return label != "" && reAccountWord.MatchString(label) && !reNotAccount.MatchString(label)
	}
	for _, m := range reGroupedValue.FindAllStringIndex(l.Text, -1) {
		v := digitsOnly(l.Text[m[0]:m[1]])
		if len(v) >= 6 && len(v) <= 20 && endsField(l.Text[m[1]:]) && isAccount(m[0]) {
			out = append(out, found{m[0], m[1], v, nil})
		}
	}
	for _, m := range reAccountValue.FindAllStringIndex(l.Text, -1) {
		v := l.Text[m[0]:m[1]]
		if countDigits(v) < 6 || len(v)-countDigits(v) > 2 || overlapsFound(m, out) {
			continue
		}
		if isAccount(m[0]) {
			out = append(out, found{m[0], m[1], v, nil})
		}
	}
	return out
}

var reGroupedValue = regexp.MustCompile(`\b[0-9]{2,}(?: [0-9]{2,})+\b`)

// endsField reports whether the rest of a line starts where a field value
// ends: the end of the line, a wide gap, or a list mark. A single space
// and more digits or a word could be the next value: "123456 01 Jan".
func endsField(rest string) bool {
	rest = strings.TrimSuffix(rest, "\r")
	if rest == "" || strings.HasPrefix(rest, "  ") || strings.HasPrefix(rest, "\t") {
		return true
	}
	r := strings.TrimPrefix(rest, " ")
	return r != "" && strings.ContainsAny(r[:1], ",;|()")
}

func overlapsFound(m []int, fs []found) bool {
	for _, f := range fs {
		if m[0] < f.b1 && f.b0 < m[1] {
			return true
		}
	}
	return false
}

// fieldLabel returns the label of the field whose value starts where prefix
// ends, or "" when the prefix does not end in a label. A label ends in a
// colon or in "No."; one word (an account type) may follow the colon. The
// label is at most four words, after the previous field's value.
func fieldLabel(prefix string) string {
	p := strings.TrimRightFunc(prefix, unicode.IsSpace)
	if q, ok := cutColon(p); ok {
		p = q
	} else if low := strings.ToLower(p); !strings.HasSuffix(low, "no.") && !strings.HasSuffix(low, " no") {
		i := strings.LastIndex(p, " ")
		if i < 0 || strings.ContainsAny(p[i+1:], "0123456789") {
			return ""
		}
		if p, ok = cutColon(strings.TrimRightFunc(p[:i], unicode.IsSpace)); !ok {
			return ""
		}
	}
	words := strings.Fields(p)
	start := len(words)
	for start > 0 && len(words)-start < 4 && !strings.ContainsAny(words[start-1], "0123456789:()") {
		start--
	}
	return strings.Join(words[start:], " ")
}

func cutColon(s string) (string, bool) {
	for _, c := range []string{":", "："} {
		if strings.HasSuffix(s, c) {
			return strings.TrimRightFunc(strings.TrimSuffix(s, c), unicode.IsSpace), true
		}
	}
	return s, false
}

func countDigits(s string) int {
	n := 0
	for i := 0; i < len(s); i++ {
		if s[i] >= '0' && s[i] <= '9' {
			n++
		}
	}
	return n
}

var reRowAccount = regexp.MustCompile(`(?:^|\s)([0-9]{9,18})(?:$|[\s.,;])`)

// findRowAccount takes a bare run of 9 to 18 digits as an account when its
// line holds exactly one IFSC: an Indian account number is 9 to 18 digits
// and a transfer row gives the branch code of the account beside it.
func findRowAccount(l Line, prior []Identifier) []found {
	n := 0
	for _, id := range prior {
		if id.Kind == KindIFSC && id.Line.Page == l.Page && id.Line.Index == l.Index {
			n++
		}
	}
	if n != 1 {
		return nil
	}
	var out []found
	for _, m := range reRowAccount.FindAllStringSubmatchIndex(l.Text, -1) {
		out = append(out, found{m[2], m[3], l.Text[m[2]:m[3]], nil})
	}
	return out
}

var reMRZLine = regexp.MustCompile(`^[A-Z0-9<]{44}$`)

// findMRZ finds a passport (TD3) or visa (MRV-A) zone: two lines of 44
// characters, the first starting with P or V.
func findMRZ(lines []Line) []Identifier {
	var out []Identifier
	for i := 0; i+1 < len(lines); i++ {
		a, b := lines[i], lines[i+1]
		if a.Page != b.Page || !reMRZLine.MatchString(a.Text) || !reMRZLine.MatchString(b.Text) {
			continue
		}
		if a.Text[0] != 'P' && a.Text[0] != 'V' {
			continue
		}
		v := a.Text + "\n" + b.Text
		out = append(out, Identifier{
			Span:  Span{Page: a.Page, Start: a.Start, End: b.Start + len(b.Text), Text: v}, // the zone is ASCII
			Kind:  KindMRZ,
			Value: v,
			Valid: ptr(mrzValid(a.Text, b.Text)),
			Line:  a,
		})
		i++
	}
	return out
}

// findIdentifiers runs every scheme over the document and numbers the
// identifiers in reading order.
func findIdentifiers(lines []Line, reg *IBANRegistry) []Identifier {
	ids := findMRZ(lines)
	for _, s := range append([]scheme{{KindIBAN, reg.findIBAN}}, schemes...) {
		for _, l := range lines {
			for _, f := range s.find(l, ids) {
				sp := l.span(f.b0, f.b1)
				if overlapsAny(sp, ids) {
					continue
				}
				ids = append(ids, Identifier{Span: sp, Kind: s.kind, Value: f.value, Valid: f.valid, Line: l})
			}
		}
	}
	sortSpans(ids, func(i Identifier) Span { return i.Span })
	for i := range ids {
		ids[i].ID = "i" + itoa(i+1)
		ids[i].Code = codeKey(ids[i].Kind, ids[i].Value, ids[i].Valid, reg)
		ids[i].proof = kindProof(ids[i].Kind, ids[i].Value, reg)
	}
	return ids
}

func overlapsAny(sp Span, ids []Identifier) bool {
	for _, id := range ids {
		if id.Span.overlaps(sp) {
			return true
		}
	}
	return false
}
