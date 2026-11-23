package extract

import (
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/sanket2904/sifs/engine/extract/orgkey"
)

// Mention is a name of an organisation in a document. Its span is the whole
// name as the document writes it.
type Mention struct {
	Span
	Key       orgkey.Key // the organisation's key; zero when nothing settles it
	SettledBy string     // name, mention, code, directory, or ""
	words     []string
	kind      nameKind
	bank      bool // holds a word that says bank
}

// nameKind says how far a mention's own words settle who it is.
type nameKind int

const (
	// wholeName holds an organisation word: its words settle it.
	wholeName nameKind = iota
	// acronymName is an acronym with only a legal form, such as "XYZ Ltd":
	// an organisation, but only a tie in the document or the directory
	// says which.
	acronymName
	// bareWord is one word with no organisation word in a bank field, such
	// as "Bank: XYZ": only a tie in the document makes it an organisation.
	// The field label alone does not, and neither does the directory.
	bareWord
)

// Organisation-shaped words. They are words of a kind of organisation in
// each language, never names of organisations.
var (
	bankWords    = set("bank", "banque", "banco", "banca", "bankhaus", "sparkasse")
	romanceBanks = set("banque", "banco", "banca") // the noun comes before its adjectives
	typeWords    = set("company", "corporation", "office", "school")
	pairedTypes  = map[string]string{"union": "credit", "society": "building"}
	legalForms   = set("ltd", "limited", "plc", "inc", "llc", "gmbh", "ag", "sa", "s.a.", "n.v.", "nv", "b.v.", "bv", "se", "kg", "pvt", "co", "लिमिटेड")
	connectors   = set("of", "de", "du", "des", "di", "del", "della", "ऑफ़", "ऑफ")
	// Capitalised words that open a sentence or a phrase and are no part of a name.
	notNameWords = set("the", "a", "an", "dear", "to", "from", "pay", "paid", "thank", "thanks", "with", "at", "in", "on", "for",
		"by", "and", "or", "your", "our", "my", "this", "that", "please", "yours", "if", "via", "mit", "bei", "der", "die", "das",
		"dem", "den", "ein", "eine", "und", "von", "für", "zu", "pour", "avec", "et", "chez", "par")
	bankLabelWords = set("bank", "banks", "banque", "banco", "kreditinstitut", "geldinstitut", "बैंक")
	devaBanks      = set("बैंक")
)

func set(words ...string) map[string]bool {
	m := map[string]bool{}
	for _, w := range words {
		m[orgkey.Fold(w)] = true
	}
	return m
}

type token struct {
	text   string
	b0, b1 int
	gap    string // the text between the previous token and this one
}

func (t token) low() string { return strings.ToLower(orgkey.Fold(t.text)) }

// joined reports whether only spaces separate t from the token before it.
func (t token) joined() bool { return t.gap != "" && strings.TrimSpace(t.gap) == "" }

func (t token) script() string {
	r, _ := utf8.DecodeRuneInString(t.text)
	switch {
	case unicode.Is(unicode.Devanagari, r):
		return "deva"
	case unicode.Is(unicode.Latin, r) || r == '&':
		return "latin"
	}
	return "other"
}

func (t token) capital() bool {
	r, _ := utf8.DecodeRuneInString(t.text)
	return unicode.IsUpper(r)
}

func (t token) allCaps() bool {
	n := 0
	for _, r := range t.text {
		if !unicode.IsUpper(r) {
			return false
		}
		n++
	}
	return n >= 2
}

func isWordRune(r rune) bool { return unicode.IsLetter(r) || unicode.IsMark(r) || unicode.IsDigit(r) }

// tokenize cuts a line into words, "&", and dotted abbreviations such as N.V.
func tokenize(s string) []token {
	var out []token
	last := 0
	for i := 0; i < len(s); {
		r, n := utf8.DecodeRuneInString(s[i:])
		if r == '&' || r == '＆' {
			out = append(out, token{text: s[i : i+n], b0: i, b1: i + n, gap: s[last:i]})
			i += n
			last = i
			continue
		}
		if !isWordRune(r) {
			i += n
			continue
		}
		j, seg, dotted := i, 0, false
		for j < len(s) {
			r, n := utf8.DecodeRuneInString(s[j:])
			if isWordRune(r) {
				j += n
				seg++
				continue
			}
			next, _ := utf8.DecodeRuneInString(s[j+n:])
			if r == '.' && seg == 1 && unicode.IsUpper(next) { // N.V., S.A.
				j, seg, dotted = j+n, 0, true
				continue
			}
			if r == '.' && seg == 1 && dotted {
				j += n
				break
			}
			if (r == '\'' || r == '’' || r == '-') && unicode.IsLetter(next) && seg > 0 {
				j += n
				continue
			}
			break
		}
		out = append(out, token{text: s[i:j], b0: i, b1: j, gap: s[last:i]})
		i, last = j, j
	}
	return out
}

func isCompoundBank(t token) bool {
	l := t.low()
	if !strings.HasSuffix(l, "bank") || utf8.RuneCountInString(l) <= 5 {
		return false
	}
	for _, r := range l {
		if !unicode.IsLetter(r) {
			return false
		}
	}
	return true
}

// anchorKind says what kind of organisation word toks[i] is: "bank",
// "type", "legal", "compound", or "".
func anchorKind(toks []token, i int) string {
	t := toks[i]
	l := t.low()
	if t.script() == "deva" {
		switch {
		case devaBanks[l]:
			return "bank"
		case legalForms[l]:
			return "legal"
		}
		return ""
	}
	if t.script() != "latin" {
		return ""
	}
	if isCompoundBank(t) {
		return "compound"
	}
	if !t.capital() {
		return ""
	}
	switch {
	case bankWords[l]:
		return "bank"
	case typeWords[l]:
		return "type"
	case legalForms[l]:
		return "legal"
	}
	if prev, ok := pairedTypes[l]; ok && i > 0 && toks[i].joined() && toks[i-1].low() == prev {
		return "type"
	}
	return ""
}

func isNameToken(t token) bool {
	return t.script() == "latin" && t.capital() && !notNameWords[t.low()]
}

// latinName grows the name around an organisation word at toks[a].
func latinName(toks []token, a int) (l, r int) {
	l, r = a, a
	for l > 0 && a-l < 6 && toks[l].joined() {
		p := toks[l-1]
		if isNameToken(p) || anchorKind(toks, l-1) != "" {
			l--
			continue
		}
		if p.text == "&" && l >= 2 && p.joined() && isNameToken(toks[l-2]) {
			l -= 2
			continue
		}
		break
	}
	for r+1 < len(toks) && toks[r+1].joined() {
		n := toks[r+1]
		switch {
		case anchorKind(toks, r+1) != "":
			r++
		case romanceBanks[toks[a].low()] && isNameToken(n) && r-a < 3:
			r++
		case connectors[n.low()] && r+2 < len(toks) && toks[r+2].joined() && isNameToken(toks[r+2]):
			r += 2
			for k := 0; k < 2 && r+1 < len(toks) && toks[r+1].joined() && isNameToken(toks[r+1]) && anchorKind(toks, r+1) == ""; k++ {
				r++
			}
		default:
			return l, r
		}
	}
	return l, r
}

// devaName grows a Devanagari name to the left up to the phrase start, at
// most four words; with no capitals, a name that does not reach a phrase
// start within four words is too unsure to take.
func devaName(toks []token, a int) (l, r int, ok bool) {
	l, r = a, a
	for l > 0 && toks[l].joined() { // a phrase starts at the line start or after punctuation
		if a-l >= 4 || toks[l-1].script() != "deva" {
			return 0, 0, false
		}
		l--
	}
	for r+1 < len(toks) && toks[r+1].joined() && toks[r+1].script() == "deva" {
		n := toks[r+1].low()
		switch {
		case legalForms[n]:
			r++
		case connectors[n] && r+2 < len(toks) && toks[r+2].joined() && toks[r+2].script() == "deva":
			r += 2
		default:
			return l, r, true
		}
	}
	return l, r, true
}

// followedByColon reports whether a field separator follows byte b in s.
func followedByColon(s string, b int) bool { return startsField(strings.TrimLeft(s[b:], " ")) }

// startsField reports whether s starts with a field separator: one colon.
// A run of colons, as in ":: Example Bank ::", is decoration.
func startsField(s string) bool {
	for _, c := range []string{":", "："} {
		if rest, ok := strings.CutPrefix(s, c); ok {
			return !strings.HasPrefix(rest, ":") && !strings.HasPrefix(rest, "：")
		}
	}
	return false
}

// findNames finds the organisation names of one line written in words:
// a run of name words around an organisation word.
func findNames(l Line) []Mention {
	toks := tokenize(l.Text)
	var out []Mention
	for a := range toks {
		kind := anchorKind(toks, a)
		if kind == "" {
			continue
		}
		var lo, hi int
		if toks[a].script() == "deva" {
			var ok bool
			if lo, hi, ok = devaName(toks, a); !ok {
				continue
			}
		} else {
			lo, hi = latinName(toks, a)
		}
		var words []string
		names, bank, typed := 0, false, false
		for i := lo; i <= hi; i++ {
			words = append(words, toks[i].text)
			switch k := anchorKind(toks, i); {
			case k == "bank" || k == "compound":
				bank, typed = true, true
				if k == "compound" {
					names++
				}
			case k == "type":
				typed = true
			case k == "legal" || connectors[toks[i].low()] || toks[i].text == "&":
			default:
				names++
			}
		}
		if names == 0 {
			continue
		}
		if followedByColon(l.Text, toks[hi].b1) && (bank || names < 2) {
			continue // a field label such as "Bank:" or "Remitting Bank:"
		}
		m := Mention{Span: l.span(toks[lo].b0, toks[hi].b1), words: words, bank: bank}
		if !typed && names == 1 && toks[lo].allCaps() && utf8.RuneCountInString(toks[lo].text) <= 6 {
			m.kind = acronymName
		}
		out = appendMention(out, m)
	}
	return append(out, findCJKNames(l)...)
}

// appendMention keeps the longer of two overlapping names.
func appendMention(ms []Mention, m Mention) []Mention {
	for i, have := range ms {
		if have.Span.overlaps(m.Span) {
			if m.End-m.Start > have.End-have.Start {
				ms[i] = m
			}
			return ms
		}
	}
	return append(ms, m)
}

var cjkBank = []rune("銀行")

func isCJKNameRune(r rune) bool {
	return unicode.Is(unicode.Han, r) || unicode.Is(unicode.Katakana, r) || r == 'ー' || r == '・' || r == '＆' ||
		r == 'ﾞ' || r == 'ﾟ' ||
		(r < 0x80 && (unicode.IsLetter(r) || unicode.IsDigit(r))) || (r >= 0xFF10 && r <= 0xFF5A)
}

func isCJKBoundary(r rune) bool { return strings.ContainsRune(" \t:：、。「」（）()・,", r) }

// findCJKNames finds names that end in 銀行. Japanese has no spaces, so the
// name runs left over kanji, katakana and Latin letters; a run of hiragana
// joins it only when it reaches the start of the phrase, as in さくら銀行.
func findCJKNames(l Line) []Mention {
	rs := []rune(l.Text)
	var out []Mention
	for i := 0; i+len(cjkBank) <= len(rs); i++ {
		if string(rs[i:i+len(cjkBank)]) != string(cjkBank) {
			continue
		}
		end := i + len(cjkBank)
		s := i
		for s > 0 && isCJKNameRune(rs[s-1]) {
			s--
		}
		h := s
		for h > 0 && unicode.Is(unicode.Hiragana, rs[h-1]) {
			h--
		}
		if h < s && (h == 0 || isCJKBoundary(rs[h-1])) {
			s = h
		}
		if s == i {
			continue
		}
		if startsField(strings.TrimLeft(string(rs[end:]), " ")) {
			continue // a field label such as 支払銀行:
		}
		end = legalTail(rs, s, end)
		text := string(rs[s:end])
		out = append(out, Mention{Span: Span{Page: l.Page, Start: l.Start + s, End: l.Start + end, Text: text}, words: []string{text}, bank: true})
		i = end - 1
	}
	return out
}

// legalTail is where the name rs[start:end] ends with the legal forms
// printed after it that its key drops, such as 株式会社 or Ltd: they are part
// of the name as the page writes it, so its span keeps them, and its key
// stays the same.
func legalTail(rs []rune, start, end int) int {
	key := orgkey.MatchKey(string(rs[start:end]))
	for {
		i := end
		for i < len(rs) && (rs[i] == ' ' || rs[i] == '\u3000') {
			i++
		}
		j := i
		if strings.HasPrefix(string(rs[i:]), "株式会社") {
			j += len([]rune("株式会社"))
		} else {
			for j < len(rs) && isWordRune(rs[j]) {
				j++
			}
			if j == i || !unicode.IsUpper(rs[i]) || !legalForms[strings.ToLower(string(rs[i:j]))] {
				return end
			}
		}
		if orgkey.MatchKey(string(rs[start:j])) != key {
			return end
		}
		end = j
	}
}

// findFieldBanks reads the value of a field whose label says bank, such as
// "Banks involved: Alpha Group, Beta Trust": each listed item of two or
// more words shaped like a name is a name of an organisation. A one-word
// item is a bare word: the label may be wrong about it.
func findFieldBanks(l Line, have []Mention) []Mention {
	var out []Mention
	t := l.Text
	for c := 0; c < len(t); c++ {
		if t[c] != ':' && !strings.HasPrefix(t[c:], "：") {
			continue
		}
		sep := 1
		if t[c] != ':' {
			sep = len("：")
		}
		if !isBankLabel(t[:c]) {
			c += sep - 1
			continue
		}
		end := len(t)
		if n := strings.IndexAny(t[c+sep:], ":："); n >= 0 {
			end = strings.LastIndex(t[:c+sep+n], " ")
			if end <= c {
				end = c + sep
			}
		}
		for _, item := range splitItems(t, c+sep, end) {
			m := Mention{Span: l.span(item[0], item[1]), words: strings.Fields(t[item[0]:item[1]])}
			if len(m.words) == 1 {
				m.kind = bareWord
			}
			if nameShaped(t[item[0]:item[1]]) && !overlapsMention(m.Span, have) && !overlapsMention(m.Span, out) {
				out = append(out, m)
			}
		}
		c += sep - 1
	}
	return out
}

func isBankLabel(prefix string) bool {
	words := strings.Fields(prefix)
	for i := len(words) - 1; i >= 0 && len(words)-i <= 5; i-- {
		w := strings.ToLower(orgkey.Fold(strings.Trim(words[i], ".,;()")))
		if strings.ContainsAny(w, "0123456789:") {
			return false
		}
		if bankLabelWords[w] || strings.HasSuffix(w, "銀行") {
			return true
		}
	}
	return false
}

// splitItems cuts t[b0:b1] at commas and list words into trimmed items.
func splitItems(t string, b0, b1 int) [][2]int {
	var out [][2]int
	start := b0
	cut := func(e int) {
		s, x := start, e
		for s < x && (t[s] == ' ' || t[s] == '\t') {
			s++
		}
		for x > s && (t[x-1] == ' ' || t[x-1] == '.' || t[x-1] == '\t') {
			x--
		}
		if x > s {
			out = append(out, [2]int{s, x})
		}
	}
	for i := b0; i < b1; i++ {
		switch {
		case t[i] == ',' || t[i] == ';':
			cut(i)
			start = i + 1
		case strings.HasPrefix(t[i:], "、"):
			cut(i)
			start = i + len("、")
		case strings.HasPrefix(t[i:], " and ") || strings.HasPrefix(t[i:], " und ") || strings.HasPrefix(t[i:], " et "):
			cut(i)
			start = i + strings.Index(t[i+1:], " ") + 2
		}
	}
	cut(b1)
	return out
}

// nameShaped: one to five capitalised Latin words, or a short run of one
// other script, and no digits.
func nameShaped(s string) bool {
	if strings.ContainsAny(s, "0123456789@") {
		return false
	}
	toks := tokenize(s)
	if len(toks) == 0 || len(toks) > 5 {
		return false
	}
	for i, t := range toks {
		if t.script() == "latin" {
			if !(isNameToken(t) || (i > 0 && (connectors[t.low()] || t.text == "&"))) {
				return false
			}
		} else if utf8.RuneCountInString(s) > 20 {
			return false
		}
	}
	return true
}

func overlapsMention(sp Span, ms []Mention) bool {
	for _, m := range ms {
		if m.Span.overlaps(sp) {
			return true
		}
	}
	return false
}

// shortForm is an all-capitals word that may be a short form of a name in
// the same document, or the institution code of an IFSC or BIC in it.
type shortForm struct {
	Mention
	toks []token
	i    int // the word's index in toks
}

// findShortCandidates gives the all-capitals words of a line that no name
// or identifier covers and that are no field label.
func findShortCandidates(l Line, names []Mention, ids []Identifier) []shortForm {
	toks := tokenize(l.Text)
	var out []shortForm
	for i, t := range toks {
		if t.script() != "latin" || !t.allCaps() || utf8.RuneCountInString(t.text) > 6 || followedByColon(l.Text, t.b1) {
			continue
		}
		sp := l.span(t.b0, t.b1)
		if overlapsMention(sp, names) || overlapsAny(sp, ids) {
			continue
		}
		out = append(out, shortForm{Mention: Mention{Span: sp, words: []string{t.text}}, toks: toks, i: i})
	}
	return out
}

// initials of a name's words, leaving out legal forms and connectors.
func initials(words []string) string {
	var b strings.Builder
	for _, w := range words {
		l := strings.ToLower(orgkey.Fold(w))
		if legalForms[l] || connectors[l] || w == "&" {
			continue
		}
		r, _ := utf8.DecodeRuneInString(w)
		b.WriteRune(unicode.ToUpper(r))
	}
	return b.String()
}
