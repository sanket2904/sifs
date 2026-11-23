package extract

import (
	"sort"

	"github.com/sanket2904/sifs/engine/extract/orgkey"
)

// union groups values that belong together.
type union[T comparable] struct{ parent map[T]T }

func newUnion[T comparable]() *union[T] { return &union[T]{parent: map[T]T{}} }

func (u *union[T]) find(x T) T {
	if _, ok := u.parent[x]; !ok {
		u.parent[x] = x
	}
	for u.parent[x] != x {
		u.parent[x] = u.parent[u.parent[x]]
		x = u.parent[x]
	}
	return x
}

func (u *union[T]) join(a, b T) { u.parent[u.find(b)] = u.find(a) }

// Output is the run file sample/FORMAT.md describes.
type Output struct {
	Format        int             `json:"format"`
	Extractor     string          `json:"extractor"`
	Directory     string          `json:"directory"`
	TextSet       string          `json:"text_set"`
	Organisations map[string]Card `json:"organisations"`
	Documents     []DocOut        `json:"documents"`
	Answers       Answers         `json:"answers"`
}

// Card says who a key is.
type Card struct {
	Name      *string `json:"name"`
	Directory *string `json:"directory"`
}

// DocOut is one document's entry.
type DocOut struct {
	ID          string       `json:"id"`
	Orgs        []OrgOut     `json:"orgs"`
	Identifiers []IdentOut   `json:"identifiers"`
	Links       []LinkOut    `json:"links"`
	Statement   *StmtOut     `json:"statement,omitempty"`
	Passport    *PassportOut `json:"passport,omitempty"`
	SameOrg     []SameOrgOut `json:"same_org"`
}

type OrgOut struct {
	Span
	Org       *string `json:"org"`
	SettledBy *string `json:"settled_by"`
}

type IdentOut struct {
	Span
	ID    string `json:"id"`
	Kind  string `json:"kind"`
	Value string `json:"value"`
	Valid *bool  `json:"valid"`
	Tier  Tier   `json:"tier"`
}

type LinkOut struct {
	Identifier string   `json:"identifier"`
	Org        string   `json:"org"`
	Reasons    []string `json:"reasons"`
	Via        string   `json:"via,omitempty"`
	Tier       Tier     `json:"tier"`
}

// SameOrgOut says two keys are one organisation.
type SameOrgOut struct {
	Keys   [2]string `json:"keys"`
	Reason string    `json:"reason"`
	Tier   Tier      `json:"tier"`
}

type StmtOut struct {
	From     string          `json:"from"`
	To       string          `json:"to"`
	Accounts []string        `json:"accounts"`
	Issuer   string          `json:"issuer"`
	Tiers    map[string]Tier `json:"tiers"`
}

type PassportOut struct {
	Holder   HolderOut       `json:"holder"`
	Number   *string         `json:"number"`
	MRZ      *string         `json:"mrz"`
	Issued   *string         `json:"issued"`
	Expiry   *string         `json:"expiry"`
	ExpiryAt *Span           `json:"expiry_at"`
	Current  *bool           `json:"current"`
	Tiers    map[string]Tier `json:"tiers"`
}

type HolderOut struct {
	Surname string `json:"surname"`
	Given   string `json:"given"`
}

type Answers struct {
	LatestStatement []AccountAnswer `json:"latest_statement"`
	CurrentPassport []HolderAnswer  `json:"current_passport"`
}

type AccountAnswer struct {
	Account  string `json:"account"`
	Document string `json:"document"`
}

type HolderAnswer struct {
	Holder   string `json:"holder"`
	Document string `json:"document"`
}

// Run extracts every document. A printed name's key belongs to its
// document, and nothing joins a name across documents; a code's key is the
// same in every document. With a directory, each document also gets what
// the directory proves. Without one, the program knows no organisation
// names at all.
func Run(docs []Document, dir *Directory, reg *IBANRegistry, ref Date) Output {
	results := make([]DocResult, len(docs))
	for i, d := range docs {
		results[i] = readDocument(d, reg, ref)
		if dir != nil {
			dir.settle(&results[i])
		}
		spellNames(&results[i])
	}
	supersede(results)
	return render(results, dir)
}

func render(results []DocResult, dir *Directory) Output {
	out := Output{Format: 3, Directory: "off", Organisations: map[string]Card{}, Documents: []DocOut{}, Answers: Answers{LatestStatement: []AccountAnswer{}, CurrentPassport: []HolderAnswer{}}}
	if dir != nil {
		out.Directory = "on"
	}
	keep := carded(results, dir)
	names := map[orgkey.Key]map[string]int{} // key -> texts the key is spelt from
	cards := map[orgkey.Key]bool{}
	for _, r := range results {
		d := DocOut{ID: r.ID, Orgs: []OrgOut{}, Identifiers: []IdentOut{}, Links: []LinkOut{}, SameOrg: []SameOrgOut{}}
		used := map[orgkey.Key]bool{}
		for _, m := range r.Mentions {
			o := OrgOut{Span: m.Span}
			if !m.Key.IsZero() {
				o.Org, o.SettledBy = ptr(m.Key.String()), ptr(m.SettledBy)
				used[m.Key] = true
				if k, _ := orgkey.Name(r.ID, m.Text); k == m.Key {
					if names[m.Key] == nil {
						names[m.Key] = map[string]int{}
					}
					names[m.Key][m.Text]++
				}
			}
			d.Orgs = append(d.Orgs, o)
		}
		for _, id := range r.Identifiers {
			d.Identifiers = append(d.Identifiers, IdentOut{Span: id.Span, ID: id.ID, Kind: id.Kind, Value: id.Value, Valid: id.Valid, Tier: tierOf(identClaim(id.Kind), id.proof)})
		}
		type linkID struct {
			ident string
			key   orgkey.Key
		}
		at := map[linkID]int{}
		for _, l := range r.Links {
			if !keep(l.key) {
				continue
			}
			used[l.key] = true
			i, ok := at[linkID{l.ident, l.key}]
			if !ok {
				i = len(d.Links)
				at[linkID{l.ident, l.key}] = i
				d.Links = append(d.Links, LinkOut{Identifier: l.ident, Org: l.key.String(), Tier: Likely})
			}
			if !contains(d.Links[i].Reasons, l.reason) {
				d.Links[i].Reasons = append(d.Links[i].Reasons, l.reason)
			}
			if l.via != "" && d.Links[i].Via == "" {
				d.Links[i].Via = l.via
			}
			if tierOf(linkClaim(l.reason), l.proof) == Certain {
				d.Links[i].Tier = Certain
			}
		}
		if s := r.Statement; s != nil {
			used[s.Issuer.Key] = true
			so := &StmtOut{From: s.From.String(), To: s.To.String(), Issuer: s.Issuer.Key.String(), Tiers: map[string]Tier{
				"period": tierOf(stmtClaim("period"), read), "accounts": tierOf(stmtClaim("accounts"), read), "issuer": tierOf(stmtClaim("issuer"), read),
			}}
			for _, a := range s.Accounts {
				so.Accounts = append(so.Accounts, a.ID)
			}
			d.Statement = so
		}
		if p := r.Passport; p != nil {
			po := &PassportOut{Holder: HolderOut{p.Surname, p.Given}, ExpiryAt: p.ExpiryAt, Current: p.Current, Tiers: map[string]Tier{
				"holder": tierOf(passportClaim("holder"), p.holderProof),
			}}
			if p.Number != nil {
				po.Number = ptr(p.Number.ID)
				po.Tiers["number"] = tierOf(passportClaim("number"), p.numberProof)
			}
			if p.MRZ != nil {
				po.MRZ = ptr(p.MRZ.ID)
			}
			if p.Issued != nil {
				po.Issued = ptr(p.Issued.String())
			}
			switch {
			case p.CannotTell:
				po.Expiry = ptr("cannot-tell")
			case p.Expiry != nil:
				po.Expiry = ptr(p.Expiry.String())
			}
			if po.Expiry != nil {
				po.Tiers["expiry"] = tierOf(passportClaim("expiry"), p.expiryProof)
			}
			if p.Current != nil {
				po.Tiers["current"] = tierOf(passportClaim("current"), read)
			}
			d.Passport = po
		}
		d.SameOrg = sameOrgRows(r.Joins, used, dir)
		if len(d.Orgs)+len(d.Identifiers) > 0 || d.Statement != nil || d.Passport != nil {
			out.Documents = append(out.Documents, d)
		}
		for k := range used {
			cards[k] = true
		}
	}
	for k := range cards {
		out.Organisations[k.String()] = card(k, names[k], dir)
	}
	out.Answers.LatestStatement = latestStatements(results, func(key orgkey.Key) string {
		if dir == nil {
			return ""
		}
		id, _ := dir.lookupKey(key)
		return id
	})
	out.Answers.CurrentPassport = currentPassports(results)
	return out
}

// card says who a key is. A code's key, with the directory, is the entry
// that holds the code exactly. A printed name's key is the name it is spelt
// from most often, and never a directory entry: the directory lists banks,
// not everyone who prints a name, so a name one entry holds is a hint. A
// code key with no entry has no name.
func card(key orgkey.Key, printed map[string]int, dir *Directory) Card {
	if dir != nil && !key.IsName() {
		if id, ok := dir.lookupCode(key); ok {
			return Card{Name: ptr(dir.entries[id].Name), Directory: ptr(id)}
		}
	}
	if n := mostUsed(printed); n != "" {
		return Card{Name: ptr(n)}
	}
	return Card{}
}

// carded says which keys may carry links. With the directory on, a code
// that no entry holds and that no join ties to a printed name says nothing
// about who holds it: it gets no card, and nothing links to it. With the
// directory off nothing can tell an assigned code from an invented one, so
// a code keeps its card, with no name.
func carded(results []DocResult, dir *Directory) func(orgkey.Key) bool {
	if dir == nil {
		return func(orgkey.Key) bool { return true }
	}
	ties := newUnion[orgkey.Key]()
	for _, r := range results {
		for _, j := range r.Joins {
			ties.join(j.A, j.B)
		}
	}
	named := map[orgkey.Key]bool{}
	for _, r := range results {
		for _, m := range r.Mentions {
			if !m.Key.IsZero() {
				named[ties.find(m.Key)] = true
			}
		}
	}
	return func(key orgkey.Key) bool {
		_, held := dir.lookupKey(key)
		return key.IsName() || held || named[ties.find(key)]
	}
}

// sameOrgRows writes a document's joins between keys it uses, one row per
// two keys and reason, in a fixed order. A join of two keys that look up
// to two directory entries is not written: the directory says they are
// two.
func sameOrgRows(joins []Join, used map[orgkey.Key]bool, dir *Directory) []SameOrgOut {
	rows := []SameOrgOut{}
	seen := map[SameOrgOut]bool{}
	for _, j := range joins {
		if j.A == j.B || !used[j.A] || !used[j.B] {
			continue
		}
		if dir != nil {
			a, okA := dir.lookupKey(j.A)
			b, okB := dir.lookupKey(j.B)
			if okA && okB && a != b {
				continue
			}
		}
		keys := [2]string{j.A.String(), j.B.String()}
		if keys[1] < keys[0] {
			keys[0], keys[1] = keys[1], keys[0]
		}
		row := SameOrgOut{Keys: keys, Reason: j.Reason, Tier: tierOf(joinClaim(j.Reason), read)}
		if !seen[row] {
			seen[row] = true
			rows = append(rows, row)
		}
	}
	sort.Slice(rows, func(i, k int) bool {
		a, b := rows[i], rows[k]
		if a.Keys != b.Keys {
			return a.Keys[0]+"\x00"+a.Keys[1] < b.Keys[0]+"\x00"+b.Keys[1]
		}
		return a.Reason < b.Reason
	})
	return rows
}

func contains(xs []string, x string) bool {
	for _, y := range xs {
		if y == x {
			return true
		}
	}
	return false
}

func mostUsed(counts map[string]int) string {
	best, n := "", 0
	for s, c := range counts {
		if c > n || c == n && s < best {
			best, n = s, c
		}
	}
	return best
}

// latestStatements answers, for every account, which statement is its
// latest by the statement's own period. Statements are one account when
// they share any spelling of it. A tie between two documents is no answer,
// and so is a later statement whose account was not read and whose issuer
// may be this one's: it may be this account's. A printed name's key is its
// document's own, so only the directory, through entry, which gives the one
// entry a key looks up to or "", tells two issuers apart.
func latestStatements(results []DocResult, entry func(orgkey.Key) string) []AccountAnswer {
	type stmt struct {
		doc       string
		to        Date
		issuer    string // the issuer's directory entry, or ""
		spellings []string
		iban      string
	}
	var all, unread []stmt
	for _, r := range results {
		if r.Unread != nil {
			unread = append(unread, stmt{to: r.Unread.To, issuer: entry(r.Unread.Issuer.Key)})
		}
		if r.Statement == nil {
			continue
		}
		s := stmt{doc: r.ID, to: r.Statement.To, issuer: entry(r.Statement.Issuer.Key)}
		for _, a := range r.Statement.Accounts {
			s.spellings = append(s.spellings, a.Value)
			if a.Kind == KindIBAN {
				s.iban = a.Value
			}
		}
		all = append(all, s)
	}
	u := newUnion[string]()
	for _, s := range all {
		for _, sp := range s.spellings {
			u.join(s.spellings[0], sp)
		}
	}
	groups := map[string][]stmt{}
	var order []string
	for _, s := range all {
		r := u.find(s.spellings[0])
		if _, ok := groups[r]; !ok {
			order = append(order, r)
		}
		groups[r] = append(groups[r], s)
	}
	out := []AccountAnswer{}
	for _, r := range order {
		g := groups[r]
		sort.SliceStable(g, func(i, j int) bool { return g[j].to.before(g[i].to) })
		withheld := len(g) > 1 && g[0].to == g[1].to && g[0].doc != g[1].doc
		for _, u := range unread {
			apart := u.issuer != "" && g[0].issuer != "" && u.issuer != g[0].issuer
			withheld = withheld || g[0].to.before(u.to) && !apart
		}
		if withheld {
			continue
		}
		acct := g[0].spellings[0]
		for _, s := range g {
			if s.iban != "" {
				acct = s.iban
			}
		}
		out = append(out, AccountAnswer{Account: acct, Document: g[0].doc})
	}
	return out
}

// supersede applies renewal: of one holder's passports that are current
// by their dates, only the one issued last is current. When the issue
// dates cannot order them, none of them can be told current.
func supersede(results []DocResult) {
	live := map[string][]*Passport{}
	for _, r := range results {
		if p := r.Passport; p != nil && p.Current != nil && *p.Current {
			live[p.Holder()] = append(live[p.Holder()], p)
		}
	}
	for _, ps := range live {
		if len(ps) < 2 {
			continue
		}
		var last *Passport
		ordered := true
		for _, p := range ps {
			if p.Issued == nil {
				ordered = false
			} else if last == nil || last.Issued.before(*p.Issued) {
				last = p
			}
		}
		for _, p := range ps {
			if ordered && p != last && *p.Issued == *last.Issued {
				ordered = false
			}
		}
		for _, p := range ps {
			switch {
			case !ordered:
				p.Current = nil
			case p != last:
				p.Current = ptr(false)
			}
		}
	}
}

// currentPassports answers, for every holder, which passport is current:
// only when exactly one is, and none of theirs leaves it open.
func currentPassports(results []DocResult) []HolderAnswer {
	current := map[string][]string{}
	open := map[string]bool{}
	var order []string
	for _, r := range results {
		p := r.Passport
		if p == nil {
			continue
		}
		h := p.Holder()
		if _, ok := current[h]; !ok {
			order = append(order, h)
			current[h] = nil
		}
		switch {
		case p.Current == nil:
			open[h] = true
		case *p.Current:
			current[h] = append(current[h], r.ID)
		}
	}
	out := []HolderAnswer{}
	for _, h := range order {
		if !open[h] && len(current[h]) == 1 {
			out = append(out, HolderAnswer{Holder: h, Document: current[h][0]})
		}
	}
	return out
}
