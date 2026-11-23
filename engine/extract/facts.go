package extract

import (
	"regexp"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/sanket2904/sifs/engine/extract/orgkey"
)

// Statement is a document's own statement period, its one account (in
// every spelling the header gives), and the organisation that issued it.
type Statement struct {
	From, To Date
	Accounts []Identifier
	Issuer   Mention
}

var (
	rePeriodLabel    = regexp.MustCompile(`(?i)\b(?:period|zeitraum|p[ée]riode)\b|期間|अवधि`)
	reStatementTitle = regexp.MustCompile(`(?i)\b(?:statement|passbook|kontoauszug|relev[ée] de compte|estado de cuenta)\b|明細|पासबुक|विवरण`)
)

var (
	negBefore = set("not", "no", "non", "kein", "keine", "keinen", "nicht", "pas", "geen", "niet")
	negAfter  = []string{"नहीं", "ではありません", "ではない", "じゃない"}
	specimen  = set("real", "genuine", "actual", "original", "vrai", "véritable", "echt", "echter", "echte", "echtes", "verdadero", "असली", "वास्तविक", "本物", "実在")
)

// denies reports whether a line says the document is not what re names: a
// negation right before the role word ("This is not a statement", "kein
// Kontoauszug") or right after it ("विवरण नहीं"). "Not a genuine statement" is
// no denial: it says the paper is a made-up statement, not that it is
// something other than a statement.
func denies(line string, re *regexp.Regexp) bool {
	toks := tokenize(line)
	for _, m := range re.FindAllStringIndex(line, -1) {
		var before, after []token
		for _, t := range toks {
			switch {
			case t.b1 <= m[0]:
				before = append(before, t)
			case t.b0 >= m[1]:
				after = append(after, t)
			}
		}
		if specimenBefore(line[:m[0]]) {
			continue
		}
		for i := max(0, len(before)-3); i < len(before); i++ {
			if negBefore[before[i].low()] && !isNumberMark(line, toks, before[i]) {
				return true
			}
		}
		rest := strings.TrimSpace(line[m[1]:])
		for _, n := range negAfter {
			if strings.HasPrefix(rest, n) || len(after) > 1 && after[1].low() == n {
				return true
			}
		}
	}
	return false
}

// isNumberMark reports whether a "No" is the number sign of "Account No.
// 123" or "No 123", not a negation.
func isNumberMark(line string, toks []token, t token) bool {
	if t.low() != "no" {
		return false
	}
	if strings.HasPrefix(line[t.b1:], ".") {
		return true
	}
	for _, n := range toks {
		if n.b0 >= t.b1 {
			r, _ := utf8.DecodeRuneInString(n.text)
			return unicode.IsDigit(r)
		}
	}
	return false
}

// specimenBefore reports whether the text before a role word ends in a
// word that says real ("a real", "echter", "本物の").
func specimenBefore(prefix string) bool {
	p := strings.TrimSuffix(strings.ToLower(orgkey.Fold(strings.TrimSpace(prefix))), "の")
	for w := range specimen {
		if strings.HasSuffix(p, w) {
			return true
		}
	}
	return false
}

func deniedAnywhere(lines []Line, re *regexp.Regexp) bool {
	for _, l := range lines {
		if denies(l.Text, re) {
			return true
		}
	}
	return false
}

// findStatement reads a statement: a document that calls itself one above
// its period field, the account or accounts above that field that all spell
// one account, and the first organisation the document names as its issuer.
// Without the title, the period or the issuer it is no statement: a bill
// has a period too. A document that says it is no statement is none. When
// the accounts above the period do not spell one account, the statement is
// returned with no accounts: it was not read, and answers that depend on
// it must know so.
func findStatement(lines []Line, ids []Identifier, mentions []Mention) *Statement {
	if deniedAnywhere(lines, reStatementTitle) {
		return nil
	}
	var st Statement
	at, titled := -1, false
	for i, l := range lines {
		titled = titled || reStatementTitle.MatchString(l.Text)
		if !titled {
			continue
		}
		c := strings.IndexAny(l.Text, ":：")
		if c < 0 || len(strings.Fields(l.Text[:c])) > 3 || !rePeriodLabel.MatchString(l.Text[:c]) {
			continue
		}
		if f, t, ok := readPeriod(l.Text[c:]); ok {
			st.From, st.To, at = f, t, i
			break
		}
	}
	if at < 0 || len(mentions) == 0 || mentions[0].Key.IsZero() {
		return nil
	}
	st.Issuer = mentions[0]
	p := lines[at]
	for _, id := range ids {
		if id.isAccount() && (id.Valid == nil || *id.Valid) && (id.Page < p.Page || id.Page == p.Page && id.Line.Index <= p.Index) {
			st.Accounts = append(st.Accounts, id)
		}
	}
	if len(st.Accounts) == 0 || !oneAccount(st.Accounts) {
		st.Accounts = nil
	}
	return &st
}

// oneAccount reports whether the identifiers spell one account: one alone,
// or an IBAN with its domestic account number.
func oneAccount(ids []Identifier) bool {
	var ibans []Identifier
	for _, id := range ids {
		if id.Kind == KindIBAN {
			ibans = append(ibans, id)
		}
	}
	if len(ibans) > 1 {
		return false
	}
	for _, id := range ids {
		if id.Kind == KindIBAN {
			continue
		}
		if len(ibans) == 0 && len(ids) > 1 || len(ibans) == 1 && !sameAccount(id, ibans[0]) {
			return false
		}
	}
	return true
}

// sameAccount reports whether a domestic account number is the account
// part of the IBAN, leading zeros aside.
func sameAccount(acct, iban Identifier) bool {
	c := ibanParts[iban.Value[:2]]
	if acct.Kind != KindAccount || c.account1 == 0 {
		return false
	}
	part := strings.TrimLeft(iban.Value[c.account0:c.account1], "0")
	return part != "" && part == strings.TrimLeft(acct.Value, "0")
}

// Passport is what a passport page states about its holder and its dates.
type Passport struct {
	Surname, Given string
	Number, MRZ    *Identifier
	Issued         *Date
	Expiry         *Date // nil with CannotTell when the page does not settle it
	CannotTell     bool
	candidates     []Date // the dates the expiry could be when it cannot be told
	ExpiryAt       *Span
	Current        *bool
	// why holder, number and expiry are what they are
	holderProof, numberProof, expiryProof proof
}

var (
	rePassTitle = regexp.MustCompile(`^(?:PASSPORT|PASSEPORT|REISEPASS)$`)
	rePassWord  = regexp.MustCompile(`(?i)\bpass(?:port|eport)\b|reisepass|旅券|पासपोर्ट`)
)

// field is a printed field of a passport page: re finds its label and
// colon, and scanned holds the label spellings a scan may misread.
type field struct {
	re      *regexp.Regexp
	scanned []string
}

var (
	surnameField = field{regexp.MustCompile(`(?i)^\s*(?:surname|nom|familienname)\s*:\s*`), []string{"familienname", "surname"}}
	givenField   = field{regexp.MustCompile(`(?i)^\s*(?:given\s+names?|pr[ée]noms?|vornamen?)\s*:\s*`), []string{"given names", "given name", "vornamen", "prénoms"}}
	issuedField  = field{regexp.MustCompile(`(?i)\b(?:date\s+of\s+issue|date\s+de\s+d[ée]livrance|ausstellungsdatum)\s*:\s*`), []string{"date de délivrance", "ausstellungsdatum", "date of issue"}}
	expiryField  = field{regexp.MustCompile(`(?i)\b(?:date\s+of\s+expiry|date\s+d'expiration|gültig\s+bis)\s*:\s*`), []string{"date d'expiration", "date of expiry", "gültig bis"}}
)

// valueAt gives the byte offset on the line where the field's value
// starts, or -1. On a scanned page, a label at the start of the line may
// have one character misread, its colon counted: OCR may read "SURNAME:"
// as "SURNAHE:", or the colon of "NAMES:" as a letter.
func (f field) valueAt(l Line) int {
	if m := f.re.FindStringIndex(l.Text); m != nil {
		return m[1]
	}
	if !l.Scanned {
		return -1
	}
	lead := len(l.Text) - len(strings.TrimLeft(l.Text, " \t"))
	for _, label := range f.scanned {
		want := []rune(label + ":")
		b, diff := lead, 0
		for _, w := range want {
			r, n := utf8.DecodeRuneInString(l.Text[b:])
			if n == 0 {
				diff = 2
				break
			}
			if unicode.ToLower(r) != w {
				diff++
			}
			b += n
		}
		if diff <= 1 {
			return b + len(l.Text[b:]) - len(strings.TrimLeft(l.Text[b:], " "))
		}
	}
	return -1
}

// findPassport reads a passport page: one with a passport zone (TD3), or,
// with no zone at all, one titled passport that gives a passport number.
// A visa page has a visa zone and is no passport, and so is a page that
// says it is none.
func findPassport(lines []Line, ids []Identifier, ref Date) *Passport {
	if deniedAnywhere(lines, rePassWord) {
		return nil
	}
	var p Passport
	zones := 0
	for i := range ids {
		switch {
		case ids[i].Kind == KindMRZ:
			zones++
			if ids[i].Value[0] == 'P' && p.MRZ == nil {
				p.MRZ = &ids[i]
			}
		case ids[i].Kind == KindPassportNo && p.Number == nil:
			p.Number = &ids[i]
		}
	}
	titled, cancelled := false, false
	for _, l := range lines {
		t := strings.ToUpper(strings.TrimSpace(l.Text))
		titled = titled || rePassTitle.MatchString(t)
		cancelled = cancelled || hasCancelWord(l.Text)
		if v := surnameField.valueAt(l); v >= 0 && p.Surname == "" {
			p.Surname = strings.TrimSpace(l.Text[v:])
		}
		if v := givenField.valueAt(l); v >= 0 && p.Given == "" {
			p.Given = strings.TrimSpace(l.Text[v:])
		}
		if v := issuedField.valueAt(l); v >= 0 && p.Issued == nil {
			if d, _, _, ok := readDate(l.Text[v:]); ok {
				p.Issued = &d
			}
		}
		if v := expiryField.valueAt(l); v >= 0 && p.ExpiryAt == nil {
			if d, b0, b1, ok := readDate(l.Text[v:]); ok {
				p.Expiry = &d
				sp := l.span(v+b0, v+b1)
				p.ExpiryAt = &sp
			}
		}
	}
	if p.MRZ == nil && (zones > 0 || !titled || p.Number == nil) {
		return nil
	}
	if p.MRZ != nil {
		l1, l2, _ := strings.Cut(p.MRZ.Value, "\n")
		sur, given, _ := strings.Cut(strings.TrimRight(l1[5:], "<"), "<<")
		sur, given = strings.ReplaceAll(sur, "<", " "), strings.ReplaceAll(given, "<", " ")
		if *p.MRZ.Valid && p.Surname == "" {
			p.Surname, p.Given = sur, given
		}
		// The zone and the printed date must agree; a zone that fails its
		// check digits cannot say which date is right. The zone gives two
		// digits of the year: a printed date that agrees gives the century,
		// and without one the century is a guess.
		yy := atoi(l2[21:23])
		century := 2000
		if p.Expiry != nil && p.Expiry.Y%100 == yy {
			century = p.Expiry.Y - yy
		}
		zd, ok := makeDate(century+yy, atoi(l2[23:25]), atoi(l2[25:27]))
		if !*p.MRZ.Valid || !ok || p.Expiry != nil && *p.Expiry != zd {
			if p.Expiry != nil {
				p.candidates = append(p.candidates, *p.Expiry)
			}
			if ok {
				p.candidates = append(p.candidates, zd)
			}
			p.Expiry, p.CannotTell = nil, true
		} else {
			p.Expiry = &zd
		}
		// A fact is the zone's when a zone whose check digits pass says it.
		if p.MRZ.proof == zoneChecked {
			if sameZoneName(p.Surname, sur) && sameZoneName(p.Given, given) {
				p.holderProof = zoneSays
			}
			if p.Number != nil && p.Number.Value == strings.ReplaceAll(l2[0:9], "<", "") {
				p.numberProof = zoneSays
			}
			p.expiryProof = expiryProof(l2[21:27], p.ExpiryAt, p.Expiry)
		}
	}
	if p.Surname == "" || p.Given == "" {
		return nil
	}
	p.Current = isCurrent(p, cancelled, ref)
	return &p
}

// expiryProof is why a passport's expiry is what it is, given the zone's
// date of expiry (YYMMDD), the span of the printed date of expiry, and the
// expiry the extractor gives. The zone proves a day, a month and the last
// two digits of a year, never the century, so the expiry is the zone's
// only when the printed date at its span gives the century
// (printedExpiry) and the expiry is that date. Without the span it is read.
func expiryProof(zone string, at *Span, expiry *Date) proof {
	if at == nil || expiry == nil {
		return read
	}
	if d, ok := printedExpiry(at.Text, zone); ok && d == *expiry {
		return zoneAndPrintedYear
	}
	return read
}

// printedExpiry is the zone's date of expiry with the century a printed
// date gives (sample/FORMAT.md, "Tiers"): the printed text's digits, ASCII
// or full width, hold one four-digit year ending in the zone's two digits
// and, besides it, only the zone's day, or its day and month in either
// order.
func printedExpiry(text, zone string) (Date, bool) {
	var years, rest []int
	for _, n := range strings.FieldsFunc(orgkey.Fold(text), func(r rune) bool { return r < '0' || r > '9' }) {
		switch {
		case len(n) == 4:
			years = append(years, atoi(n))
		case len(n) > 2:
			return Date{}, false
		default:
			rest = append(rest, atoi(n))
		}
	}
	yy, mm, dd := atoi(zone[0:2]), atoi(zone[2:4]), atoi(zone[4:6])
	if len(years) != 1 || years[0]%100 != yy {
		return Date{}, false
	}
	switch {
	case len(rest) == 1 && rest[0] == dd:
	case len(rest) == 2 && (rest[0] == dd && rest[1] == mm || rest[0] == mm && rest[1] == dd):
	default:
		return Date{}, false
	}
	return makeDate(years[0], mm, dd)
}

// sameZoneName reports whether a printed name is the zone's, case and
// spacing ignored. A zone writes A to Z only, so a printed name with any
// other letter is never the same.
func sameZoneName(printed, zone string) bool {
	up := strings.Map(func(r rune) rune {
		if r >= 'a' && r <= 'z' {
			return r - 'a' + 'A'
		}
		return r
	}, printed)
	return strings.Join(strings.Fields(up), " ") == strings.Join(strings.Fields(zone), " ")
}

// Holder is the answer key for a holder: SURNAME, GIVEN NAMES.
func (p Passport) Holder() string {
	return strings.ToUpper(strings.Join(strings.Fields(p.Surname), " ") + ", " + strings.Join(strings.Fields(p.Given), " "))
}

// cancelWords are the words a cancelled passport is stamped with.
var cancelWords = set("CANCELLED", "CANCELED", "VOID", "VOIDED", "ANNULÉ", "ANNULÉE", "ANNULE", "UNGÜLTIG", "ENTWERTET", "ANULADO", "CANCELADO", "रद्द", "無効", "失効")

// hasCancelWord reports whether a line is a cancellation stamp: its words,
// or the value after its label ("Status: VOID"), are cancellation words
// only, in capitals or in a script that has none. "VOID IF ALTERED" is
// a warning printed on every passport, not a stamp.
func hasCancelWord(s string) bool {
	if i := strings.LastIndexAny(s, ":："); i >= 0 {
		s = s[i+1:]
	}
	toks := tokenize(s)
	for _, t := range toks {
		if strings.ToUpper(t.text) != t.text || !cancelWords[orgkey.Fold(t.text)] {
			return false
		}
	}
	return len(toks) > 0
}

// isCurrent: a cancelled passport is not current; otherwise it is current
// when its expiry, or every date its expiry could be, is not before the
// reference date, and not current when they all are. Else nobody can tell.
func isCurrent(p Passport, cancelled bool, ref Date) *bool {
	if cancelled {
		return ptr(false)
	}
	dates := p.candidates
	if p.Expiry != nil {
		dates = []Date{*p.Expiry}
	}
	after := 0
	for _, d := range dates {
		if !d.before(ref) {
			after++
		}
	}
	switch {
	case len(dates) == 0:
		return nil
	case after == len(dates):
		return ptr(true)
	case after == 0:
		return ptr(false)
	}
	return nil
}
