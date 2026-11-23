package extract

import (
	"regexp"
	"strings"
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
// negation right before the role word ("Not a statement", "kein
// Kontoauszug") or right after it ("विवरण नहीं"). "Not a real statement"
// is no denial: it says the paper is a made-up statement, not that it is
// something other than a statement.
func denies(line string, re *regexp.Regexp) bool {
	toks := tokenize(line)
	for _, m := range re.FindAllStringIndex(line, -1) {
		var before, after []string
		for _, t := range toks {
			switch {
			case t.b1 <= m[0]:
				before = append(before, t.low())
			case t.b0 >= m[1]:
				after = append(after, t.low())
			}
		}
		if specimenBefore(line[:m[0]]) {
			continue
		}
		for i := max(0, len(before)-3); i < len(before); i++ {
			if negBefore[before[i]] {
				return true
			}
		}
		rest := strings.TrimSpace(line[m[1]:])
		for _, n := range negAfter {
			if strings.HasPrefix(rest, n) || len(after) > 1 && after[1] == n {
				return true
			}
		}
	}
	return false
}

// specimenBefore reports whether the text before a role word ends in a
// word that says real ("a real", "echter", "実在の").
func specimenBefore(prefix string) bool {
	p := strings.TrimSuffix(strings.ToLower(fold(strings.TrimSpace(prefix))), "の")
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
	if at < 0 || len(mentions) == 0 || mentions[0].Token == "" {
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
	c := ibanCountries[iban.Value[:2]]
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
}

var (
	reSurname   = regexp.MustCompile(`(?i)^\s*(?:surname|nom|familienname)\s*:\s*(.+?)\s*$`)
	reGiven     = regexp.MustCompile(`(?i)^\s*(?:given\s+names?|pr[ée]noms?|vornamen?)\s*:\s*(.+?)\s*$`)
	reIssued    = regexp.MustCompile(`(?i)\b(?:date\s+of\s+issue|date\s+de\s+d[ée]livrance|ausstellungsdatum)\s*:\s*`)
	reExpiry    = regexp.MustCompile(`(?i)\b(?:date\s+of\s+expiry|date\s+d'expiration|gültig\s+bis)\s*:\s*`)
	rePassTitle = regexp.MustCompile(`^(?:PASSPORT|PASSEPORT|REISEPASS)$`)
	rePassWord  = regexp.MustCompile(`(?i)\bpass(?:port|eport)\b|reisepass|旅券|पासपोर्ट`)
)

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
		if m := reSurname.FindStringSubmatch(l.Text); m != nil && p.Surname == "" {
			p.Surname = m[1]
		}
		if m := reGiven.FindStringSubmatch(l.Text); m != nil && p.Given == "" {
			p.Given = m[1]
		}
		if m := reIssued.FindStringIndex(l.Text); m != nil && p.Issued == nil {
			if d, _, _, ok := readDate(l.Text[m[1]:]); ok {
				p.Issued = &d
			}
		}
		if m := reExpiry.FindStringIndex(l.Text); m != nil && p.ExpiryAt == nil {
			if d, b0, b1, ok := readDate(l.Text[m[1]:]); ok {
				p.Expiry = &d
				sp := l.span(m[1]+b0, m[1]+b1)
				p.ExpiryAt = &sp
			}
		}
	}
	if p.MRZ == nil && (zones > 0 || !titled || p.Number == nil) {
		return nil
	}
	if p.MRZ != nil {
		l1, l2, _ := strings.Cut(p.MRZ.Value, "\n")
		if *p.MRZ.Valid && p.Surname == "" {
			sur, given, _ := strings.Cut(strings.TrimRight(l1[5:], "<"), "<<")
			p.Surname, p.Given = strings.ReplaceAll(sur, "<", " "), strings.ReplaceAll(given, "<", " ")
		}
		// The zone and the printed date must agree; a zone that fails its
		// check digits cannot say which date is right.
		zd, ok := makeDate(2000+atoi(l2[21:23]), atoi(l2[23:25]), atoi(l2[25:27]))
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
	}
	if p.Surname == "" || p.Given == "" {
		return nil
	}
	p.Current = isCurrent(p, cancelled, ref)
	return &p
}

// Holder is the answer key for a holder: SURNAME, GIVEN NAMES.
func (p Passport) Holder() string {
	return strings.ToUpper(strings.Join(strings.Fields(p.Surname), " ") + ", " + strings.Join(strings.Fields(p.Given), " "))
}

// cancelWords are the words a cancelled passport is stamped with.
var cancelWords = set("CANCELLED", "CANCELED", "VOID", "VOIDED", "INVALID", "ANNULÉ", "ANNULÉE", "ANNULE", "UNGÜLTIG", "ENTWERTET", "ANULADO", "CANCELADO", "रद्द", "無効", "失効")

// hasCancelWord reports whether a line carries a cancellation word written
// as a stamp is: in capitals, or in a script that has none.
func hasCancelWord(s string) bool {
	for _, t := range tokenize(s) {
		if strings.ToUpper(t.text) == t.text && cancelWords[fold(t.text)] {
			return true
		}
	}
	return false
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
