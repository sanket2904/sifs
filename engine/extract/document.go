package extract

import (
	"sort"
	"strings"

	"github.com/sanket2904/sifs/engine/extract/orgkey"
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

// Reasons a same_org row gives, as sample/FORMAT.md names them.
const (
	JoinDirectory       = "directory"
	JoinSpellsCode      = "spells-code"
	JoinPrintedTogether = "printed-together"
)

// Link says an identifier is held at the organisation behind a key, for a
// reason the format names. ownLink and heldAt are the only ways to make one.
// An IBAN, a BIC or an IFSC names its institution by its own characters:
// its one link is to its own code's key, and heldAt refuses it.
type Link struct {
	ident  string
	key    orgkey.Key
	reason string
	via    string // the code a paired-code link goes through
	proof  proof
}

// ownLink is a code's link to its own code's key, by the code's own reason.
// An IBAN's is proven as the IBAN is; an IFSC's or a BIC's only by the
// directory. A code with no key, one that fails its check, has none.
func ownLink(id Identifier) (Link, bool) {
	if !id.namesItsInstitution() || id.Code.IsZero() {
		return Link{}, false
	}
	l := Link{ident: id.ID, key: id.Code, reason: codeReasons[id.Kind]}
	if id.Kind == KindIBAN {
		l.proof = id.proof
	}
	return l, true
}

// heldAt places an identifier that names no institution itself, such as an
// account or a domestic code, at a key, for a reason read from the page,
// through the code via when the reason is paired-code.
func heldAt(id Identifier, key orgkey.Key, reason string, via *Identifier) (Link, bool) {
	if id.namesItsInstitution() || key.IsZero() {
		return Link{}, false
	}
	l := Link{ident: id.ID, key: key, reason: reason}
	if via != nil {
		l.via = via.ID
	}
	return l, true
}

// Join says two keys are one organisation, as one document shows it. It is
// a claim of its own: keys are never merged.
type Join struct {
	A, B   orgkey.Key
	Reason string
}

// DocResult is everything found in one document. A printed name's key is
// the document's own; a code's key is the same in every document.
type DocResult struct {
	ID          string
	File        string
	Mentions    []Mention
	Identifiers []Identifier
	Links       []Link
	Joins       []Join
	Statement   *Statement
	Unread      *Statement // a statement whose account was not read
	Passport    *Passport
}

// readDocument reads one document. The directory is not used here:
// everything a document settles, it settles by itself. A printed name's key
// is not spelt yet: it is spelt from the first mention put on it, and only
// groups the mentions until spellNames spells it from all of them.
func readDocument(doc Document, reg *IBANRegistry, ref Date) DocResult {
	lines := splitLines(doc)
	r := DocResult{ID: doc.ID, File: doc.File, Identifiers: findIdentifiers(lines, reg)}
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
		if names[i].kind == wholeName {
			if k, ok := orgkey.Name(doc.ID, names[i].Text); ok {
				names[i].Key, names[i].SettledBy = k, "name"
			}
		}
	}
	r.Joins = codeJoins(names, r.Identifiers)
	for _, n := range names {
		if n.kind == wholeName {
			r.Mentions = append(r.Mentions, n)
		}
	}
	short, joins := shortForms(doc.ID, lines, names, r.Identifiers, r.Joins)
	r.Mentions = append(r.Mentions, short...)
	r.Joins = append(r.Joins, joins...)
	sortSpans(r.Mentions, func(m Mention) Span { return m.Span })
	r.Links = codeLinks(r.Identifiers)
	switch st := findStatement(lines, r.Identifiers, r.Mentions); {
	case st != nil && st.Accounts == nil:
		r.Unread = st
	case st != nil:
		r.Statement = st
		r.Links = append(r.Links, headerLinks(*st, r.Identifiers, r.Joins)...)
	}
	r.Passport = findPassport(lines, r.Identifiers, ref)
	return r
}

// spellNames gives each printed name's key its one spelling, once every
// mention is on its key ("No merging"): the longest match key of the texts
// of all the mentions on it. Mentions that print one name, the same match
// key, are on one key. Every row that names the key takes the spelling.
func spellNames(r *DocResult) {
	group := newUnion[orgkey.Key]()
	printed := map[string]orgkey.Key{} // match key -> a key a mention of it is on
	for _, m := range r.Mentions {
		if !m.Key.IsName() {
			continue
		}
		group.find(m.Key)
		mk := orgkey.MatchKey(m.Text)
		if k, ok := printed[mk]; ok {
			group.join(k, m.Key)
		} else {
			printed[mk] = m.Key
		}
	}
	texts := map[orgkey.Key][]string{}
	for _, m := range r.Mentions {
		if m.Key.IsName() {
			root := group.find(m.Key)
			texts[root] = append(texts[root], m.Text)
		}
	}
	spelt := map[orgkey.Key]orgkey.Key{}
	for k := range group.parent {
		spelt[k], _ = orgkey.Name(r.ID, texts[group.find(k)]...)
	}
	spell := func(k *orgkey.Key) {
		if s, ok := spelt[*k]; ok {
			*k = s
		}
	}
	for i := range r.Mentions {
		spell(&r.Mentions[i].Key)
	}
	for i := range r.Links {
		spell(&r.Links[i].key)
	}
	for i := range r.Joins {
		spell(&r.Joins[i].A)
		spell(&r.Joins[i].B)
	}
	for _, st := range []*Statement{r.Statement, r.Unread} {
		if st != nil {
			spell(&st.Issuer.Key)
		}
	}
}

// joined groups the keys that a document's joins say are one organisation.
// It is the document's own reading, used to settle what else it says; the
// keys themselves stay apart.
func joined(joins []Join) *union[orgkey.Key] {
	u := newUnion[orgkey.Key]()
	for _, j := range joins {
		u.join(j.A, j.B)
	}
	return u
}

// headerLinks ties the statement's account to its issuer. An account paired
// with a code of an organisation the document does not tie to the issuer
// may be held elsewhere, so the header alone does not tie it.
func headerLinks(st Statement, ids []Identifier, joins []Join) []Link {
	same := joined(joins)
	var out []Link
	for _, a := range st.Accounts {
		if c := pairedCode(a, ids); c == nil || same.find(c.Code) == same.find(st.Issuer.Key) {
			if l, ok := heldAt(a, st.Issuer.Key, ReasonStatementHeader, nil); ok {
				out = append(out, l)
			}
		}
	}
	return out
}

func sortSpans[T any](xs []T, span func(T) Span) {
	sort.SliceStable(xs, func(i, j int) bool { return span(xs[i]).before(span(xs[j])) })
}

// codeJoins gives the joins the document shows between a name and a code,
// or two codes:
//   - a bank name whose first word is the institution code of an IFSC or BIC
//     in the document (ABCD Bank beside ABCD0…), when only one name fits:
//     the name spells the code;
//   - an IBAN and the BIC paired with it, of the same country, when that is
//     the only reading: the two codes are printed together.
func codeJoins(names []Mention, ids []Identifier) []Join {
	var joins []Join
	for _, c := range ids {
		if !c.isCode() || c.Code.IsZero() {
			continue
		}
		fit := map[orgkey.Key]bool{}
		for _, n := range names {
			if n.bank && !n.Key.IsZero() && strings.EqualFold(orgkey.Fold(n.words[0]), c.Value[:4]) {
				fit[n.Key] = true
			}
		}
		if len(fit) == 1 {
			for key := range fit {
				joins = append(joins, Join{A: key, B: c.Code, Reason: JoinSpellsCode})
			}
		}
	}
	ibans, bics := map[string]map[orgkey.Key]bool{}, map[string]map[orgkey.Key]bool{} // country -> code keys
	for _, id := range ids {
		switch {
		case id.Code.IsZero():
		case id.Kind == KindIBAN:
			addKey(ibans, id.Value[:2], id.Code)
		case id.Kind == KindBIC:
			addKey(bics, id.Value[4:6], id.Code)
		}
	}
	for _, a := range ids {
		if a.Kind != KindIBAN || a.Code.IsZero() {
			continue
		}
		c := pairedCode(a, ids)
		if c == nil || c.Kind != KindBIC || c.Value[4:6] != a.Value[:2] {
			continue
		}
		if spellsBIC(a, *c) || len(ibans[a.Value[:2]]) == 1 && len(bics[a.Value[:2]]) == 1 {
			joins = append(joins, Join{A: a.Code, B: c.Code, Reason: JoinPrintedTogether})
		}
	}
	return joins
}

func addKey(m map[string]map[orgkey.Key]bool, country string, key orgkey.Key) {
	if m[country] == nil {
		m[country] = map[orgkey.Key]bool{}
	}
	m[country][key] = true
}

// spellsBIC: the IBAN's bank identifier is the four letters the BIC begins
// with, so the IBAN's own characters name the BIC's institution. With two
// IBANs or two BICs of one country in a document, only such a pair is the
// one reading of which code goes with which.
func spellsBIC(iban, bic Identifier) bool {
	return strings.Fields(iban.Code.String())[2] == bic.Value[:4]
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
			if id.isCode() && !id.Code.IsZero() {
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

// codeLinks gives the links whose reason is a code: each IBAN, BIC and IFSC
// at its own code's key, an account paired with a code, and the parts and
// account number of an IBAN. What an IBAN's nearness to another code
// suggests is a same_org row between the two keys, never a link.
func codeLinks(ids []Identifier) []Link {
	var out []Link
	add := func(l Link, ok bool) {
		if ok {
			out = append(out, l)
		}
	}
	for _, id := range ids {
		add(ownLink(id))
	}
	for _, id := range ids {
		if !id.isAccount() || id.Valid != nil && !*id.Valid {
			continue
		}
		if c := pairedCode(id, ids); c != nil {
			add(heldAt(id, c.Code, ReasonPairedCode, c))
		}
	}
	for _, iban := range ids {
		if iban.Kind != KindIBAN || iban.Code.IsZero() {
			continue
		}
		p := ibanParts[iban.Value[:2]]
		parts := map[string]string{} // the domestic codes the IBAN holds
		switch iban.Value[:2] {
		case "GB", "IE":
			parts[KindSortCode] = iban.Value[p.branch0:p.branch1]
		case "FR", "MC":
			parts[KindFRBank], parts[KindFRBranch] = strings.Fields(iban.Code.String())[2], iban.Value[p.branch0:p.branch1]
		}
		for _, id := range ids {
			switch {
			case id.Kind == KindAccount && sameAccount(id, iban):
				add(heldAt(id, iban.Code, ReasonSameAccount, nil))
			case parts[id.Kind] != "" && parts[id.Kind] == id.Value:
				add(heldAt(id, iban.Code, ReasonInsideIBAN, nil))
			}
		}
	}
	return out
}

// shortForms settles the all-capitals words, and the acronym names with
// only a legal form such as "ABCD Ltd", that the document ties to one
// organisation: by the institution code of an IFSC or BIC in it, as the
// leading words of a full name in it, or as the initials of one. A word
// settled by a code alone keeps a key of its own, spelt from its text, and
// a join says it spells the code: a mention never stands on a code's key.
// A word settled by a full name stands on the name's key, which spellNames
// spells from both: initials with a legal form the key keeps, such as
// "TQ N.V." for "T Q Limited", spell the longer match key.
func shortForms(docID string, lines []Line, names []Mention, ids []Identifier, joins []Join) ([]Mention, []Join) {
	same := joined(joins)
	var full []Mention
	for _, n := range names {
		if !n.Key.IsZero() {
			full = append(full, n)
		}
	}
	var out []Mention
	var spelt []Join
	settle := func(m Mention, words []string) (Mention, bool) {
		found := map[orgkey.Key]orgkey.Key{} // root -> key
		by := ""
		add := func(key orgkey.Key, how string) {
			r := same.find(key)
			if _, ok := found[r]; !ok || key.IsName() {
				found[r] = key
			}
			if by == "" || how == "mention" {
				by = how
			}
		}
		for _, c := range ids {
			if c.isCode() && !c.Code.IsZero() && len(words) == 1 && words[0] == c.Value[:4] {
				add(c.Code, "code")
			}
		}
		for _, n := range full {
			if isWordPrefix(words, n.words) || isInitials(words, n.words) {
				add(n.Key, "mention")
			}
		}
		if len(found) != 1 {
			return m, false
		}
		for _, key := range found {
			if !key.IsName() {
				own, ok := orgkey.Name(docID, m.Text)
				if !ok {
					return m, false
				}
				spelt = append(spelt, Join{A: own, B: key, Reason: JoinSpellsCode})
				key = own
			}
			m.Key, m.SettledBy = key, by
		}
		return m, true
	}
	for _, n := range names {
		if n.kind != wholeName {
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
				if orgkey.Fold(c.toks[c.i].text) != orgkey.Fold(n.words[0]) {
					continue
				}
				j := 1
				for c.i+j < len(c.toks) && j < len(n.words)-1 && c.toks[c.i+j].joined() && orgkey.Fold(c.toks[c.i+j].text) == orgkey.Fold(n.words[j]) {
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
	return out, spelt
}

// isWordPrefix: the words are the leading words of a longer name, spelt
// as the name spells them: "EXAMPLE" in a heading is no short form of
// "Example Harbour Bank".
func isWordPrefix(words, name []string) bool {
	if len(words) >= len(name) {
		return false
	}
	for i, w := range words {
		if orgkey.Fold(w) != orgkey.Fold(name[i]) {
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
		if !legalForms[strings.ToLower(orgkey.Fold(w))] {
			return false
		}
	}
	return true
}
