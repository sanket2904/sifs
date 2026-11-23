package extract

import (
	"sort"
	"strings"
)

// Link reasons, as the label schema names them.
const (
	ReasonIBANBankCode    = "iban-bank-code"
	ReasonBICInstitution  = "bic-institution"
	ReasonIFSCPrefix      = "ifsc-prefix"
	ReasonPairedCode      = "paired-code"
	ReasonInsideIBAN      = "inside-iban"
	ReasonSameAccount     = "same-account"
	ReasonStatementHeader = "statement-header"
)

// Link says an identifier is held at the organisation behind a token, for
// a reason the format names.
type Link struct {
	Ident  string
	Token  string
	Reason string
	Via    string // the code a paired-code link goes through
}

// Tie says two tokens name one organisation, from evidence in one document.
type Tie struct{ A, B string }

// DocResult is everything found in one document. Organisation tokens are
// local evidence ("name:<match key>", "ifsc:HDFC", "bic:COBADE",
// "iban:DE:37040044"); Run joins them across documents into keys.
type DocResult struct {
	ID          string
	Mentions    []Mention
	Identifiers []Identifier
	Links       []Link
	Ties        []Tie
	Statement   *Statement
	Unread      *Statement // a statement whose account was not read
	Passport    *Passport
}

// Extract reads one document. The directory is not used here: everything a
// document settles, it settles by itself.
func Extract(doc Document, ref Date) DocResult {
	lines := splitLines(doc)
	r := DocResult{ID: doc.ID, Identifiers: findIdentifiers(lines)}
	var names []Mention
	for _, l := range lines {
		for _, m := range findNames(l) {
			names = appendMention(names, m)
		}
	}
	for _, l := range lines {
		names = append(names, findFieldBanks(l, names)...)
	}
	for i := range names {
		if !names[i].weak {
			if k := matchKey(names[i].Text); k != "" {
				names[i].Token, names[i].SettledBy = "name:"+k, "name"
			}
		}
	}
	r.Ties = codeTies(names, r.Identifiers)
	for _, n := range names {
		if !n.weak {
			r.Mentions = append(r.Mentions, n)
		}
	}
	r.Mentions = append(r.Mentions, shortForms(lines, names, r.Identifiers, r.Ties)...)
	sortSpans(r.Mentions, func(m Mention) Span { return m.Span })
	r.Links = codeLinks(r.Identifiers)
	switch st := findStatement(lines, r.Identifiers, r.Mentions); {
	case st != nil && st.Accounts == nil:
		r.Unread = st
	case st != nil:
		r.Statement = st
		r.Links = append(r.Links, headerLinks(*st, r.Links, r.Ties)...)
	}
	r.Passport = findPassport(lines, r.Identifiers, ref)
	return r
}

// headerLinks ties the statement's account to its issuer. An account a code
// ties to an organisation the document does not tie to the issuer may be
// held elsewhere, so the header alone does not tie it.
func headerLinks(st Statement, links []Link, ties []Tie) []Link {
	same := newUnion()
	for _, t := range ties {
		same.join(t.A, t.B)
	}
	var out []Link
	for _, a := range st.Accounts {
		ok := true
		for _, l := range links {
			if l.Ident == a.ID && l.Reason == ReasonPairedCode && same.find(l.Token) != same.find(st.Issuer.Token) {
				ok = false
			}
		}
		if ok {
			out = append(out, Link{Ident: a.ID, Token: st.Issuer.Token, Reason: ReasonStatementHeader})
		}
	}
	return out
}

func sortSpans[T any](xs []T, span func(T) Span) {
	sort.SliceStable(xs, func(i, j int) bool { return span(xs[i]).before(span(xs[j])) })
}

// codeTies ties tokens that the document shows are one organisation:
//   - a bank name whose first word is the institution code of an IFSC or BIC
//     in the document (HDFC Bank beside HDFC0…), when only one name fits;
//   - an IBAN and the BIC paired with it, of the same country.
func codeTies(names []Mention, ids []Identifier) []Tie {
	var ties []Tie
	for _, c := range ids {
		inst := c.institution()
		if !c.isCode() || inst == "" {
			continue
		}
		fit := map[string]bool{}
		for _, n := range names {
			if n.bank && n.Token != "" && strings.EqualFold(fold(n.words[0]), c.Value[:4]) {
				fit[n.Token] = true
			}
		}
		if len(fit) == 1 {
			for tok := range fit {
				ties = append(ties, Tie{tok, inst})
			}
		}
	}
	for _, a := range ids {
		if a.Kind != KindIBAN || a.institution() == "" {
			continue
		}
		if c := pairedCode(a, ids); c != nil && c.Kind == KindBIC && c.Value[4:6] == a.Value[:2] {
			ties = append(ties, Tie{a.institution(), c.institution()})
		}
	}
	return ties
}

// pairedCode is the IFSC or BIC an account or IBAN sits with: the only code
// on its line, or, when its line holds nothing else, the only identifier
// on the line next to it.
func pairedCode(a Identifier, ids []Identifier) *Identifier {
	onLine := func(page, index int) (codes, others []int) {
		for i, id := range ids {
			if id.Line.Page != page || id.Line.Index != index {
				continue
			}
			if id.isCode() && id.institution() != "" {
				codes = append(codes, i)
			} else {
				others = append(others, i)
			}
		}
		return
	}
	accounts := 0
	codes, others := onLine(a.Line.Page, a.Line.Index)
	for _, i := range others {
		if ids[i].isAccount() {
			accounts++
		}
	}
	switch {
	case accounts != 1 || len(codes) > 1:
		return nil
	case len(codes) == 1:
		return &ids[codes[0]]
	case len(others) > 1:
		return nil
	}
	var got []int
	for _, d := range []int{-1, 1} {
		if c, o := onLine(a.Line.Page, a.Line.Index+d); len(c) == 1 && len(o) == 0 {
			got = append(got, c[0])
		}
	}
	if len(got) != 1 {
		return nil
	}
	return &ids[got[0]]
}

var codeReasons = map[string]string{KindIFSC: ReasonIFSCPrefix, KindBIC: ReasonBICInstitution, KindIBAN: ReasonIBANBankCode}

// codeLinks gives the links whose reason is a code: an identifier that
// carries its institution, an account paired with a code, and the parts
// and account number of an IBAN.
func codeLinks(ids []Identifier) []Link {
	var out []Link
	for _, id := range ids {
		if inst := id.institution(); inst != "" {
			out = append(out, Link{Ident: id.ID, Token: inst, Reason: codeReasons[id.Kind]})
		}
	}
	for _, id := range ids {
		if !id.isAccount() || id.Valid != nil && !*id.Valid {
			continue
		}
		if c := pairedCode(id, ids); c != nil {
			out = append(out, Link{Ident: id.ID, Token: c.institution(), Reason: ReasonPairedCode, Via: c.ID})
		}
	}
	for _, iban := range ids {
		if iban.Kind != KindIBAN || iban.institution() == "" {
			continue
		}
		c := ibanCountries[iban.Value[:2]]
		parts := map[string]string{} // the domestic codes the IBAN holds
		switch iban.Value[:2] {
		case "GB", "IE":
			parts[KindSortCode] = iban.Value[c.branch0:c.branch1]
		case "FR", "MC":
			parts[KindFRBank], parts[KindFRBranch] = iban.Value[c.bank0:c.bank1], iban.Value[c.branch0:c.branch1]
		}
		for _, id := range ids {
			switch {
			case id.Kind == KindAccount && sameAccount(id, iban):
				out = append(out, Link{Ident: id.ID, Token: iban.institution(), Reason: ReasonSameAccount})
			case parts[id.Kind] != "" && parts[id.Kind] == id.Value:
				out = append(out, Link{Ident: id.ID, Token: iban.institution(), Reason: ReasonInsideIBAN})
			}
		}
	}
	return out
}

// shortForms settles the all-capitals words, and the acronym names with
// only a legal form such as "HDFC Ltd", that the document ties to one
// organisation: by the institution code of an IFSC or BIC in it, as the
// leading words of a full name in it, or as the initials of one.
func shortForms(lines []Line, names []Mention, ids []Identifier, ties []Tie) []Mention {
	same := newUnion()
	for _, t := range ties {
		same.join(t.A, t.B)
	}
	var full []Mention
	for _, n := range names {
		if n.Token != "" {
			full = append(full, n)
		}
	}
	var out []Mention
	settle := func(m Mention, words []string) (Mention, bool) {
		found := map[string]string{} // root -> token
		by := ""
		add := func(tok, how string) {
			r := same.find(tok)
			if _, ok := found[r]; !ok || strings.HasPrefix(tok, "name:") {
				found[r] = tok
			}
			if by == "" || how == "mention" {
				by = how
			}
		}
		for _, c := range ids {
			if c.isCode() && c.institution() != "" && len(words) == 1 && words[0] == c.Value[:4] {
				add(c.institution(), "code")
			}
		}
		for _, n := range full {
			if isWordPrefix(words, n.words) || isInitials(words, n.words) {
				add(n.Token, "mention")
			}
		}
		if len(found) != 1 {
			return m, false
		}
		for _, tok := range found {
			m.Token, m.SettledBy = tok, by
		}
		return m, true
	}
	for _, n := range names {
		if n.weak {
			m, _ := settle(n, n.words)
			out = append(out, m)
		}
	}
	for _, l := range lines {
		for _, c := range findShortCandidates(l, names, ids) {
			if overlapsMention(c.Span, out) {
				continue
			}
			// grow the word while it reads as the leading words of a name
			best := c.Mention
			for _, n := range full {
				if fold(c.toks[c.i].text) != fold(n.words[0]) {
					continue
				}
				j := 1
				for c.i+j < len(c.toks) && j < len(n.words)-1 && c.toks[c.i+j].joined() && fold(c.toks[c.i+j].text) == fold(n.words[j]) {
					j++
				}
				if end := c.toks[c.i+j-1].b1; j > len(best.words) {
					best = Mention{Span: l.span(c.toks[c.i].b0, end), words: strings.Fields(l.Text[c.toks[c.i].b0:end])}
				}
			}
			if m, ok := settle(best, best.words); ok {
				out = append(out, m)
			}
		}
	}
	return out
}

// isWordPrefix: the words are the leading words of a longer name, spelt
// as the name spells them: "SAMPLE" in a heading is no short form of
// "Sample Nagar Sahakari Bank".
func isWordPrefix(words, name []string) bool {
	if len(words) >= len(name) {
		return false
	}
	for i, w := range words {
		if fold(w) != fold(name[i]) {
			return false
		}
	}
	return true
}

// isInitials: the first word is the initials of the name, and any other
// word is a legal form.
func isInitials(words, name []string) bool {
	in := initials(name)
	if len(in) < 2 || words[0] != in {
		return false
	}
	for _, w := range words[1:] {
		if !legalForms[strings.ToLower(fold(w))] {
			return false
		}
	}
	return true
}
