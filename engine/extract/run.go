package extract

import (
	"sort"
	"strings"
)

// union groups strings that belong together.
type union struct{ parent map[string]string }

func newUnion() *union { return &union{parent: map[string]string{}} }

func (u *union) find(x string) string {
	if _, ok := u.parent[x]; !ok {
		u.parent[x] = x
	}
	for u.parent[x] != x {
		u.parent[x] = u.parent[u.parent[x]]
		x = u.parent[x]
	}
	return x
}

func (u *union) join(a, b string) { u.parent[u.find(b)] = u.find(a) }

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

// Run extracts every document. Each document keeps its own keys, spelt
// from its text, so a key means the same in every document and nothing is
// merged; with a directory, each document also gets what the directory
// proves. Without one, the program knows no organisation names at all.
func Run(docs []Document, dir *Directory, reg *IBANRegistry, ref Date) Output {
	results := make([]DocResult, len(docs))
	for i, d := range docs {
		results[i] = Extract(d, reg, ref)
		if dir != nil {
			dir.settle(&results[i])
		}
	}
	supersede(results)
	return render(results, dir)
}

func render(results []DocResult, dir *Directory) Output {
	out := Output{Format: 2, Directory: "off", Organisations: map[string]Card{}, Documents: []DocOut{}, Answers: Answers{LatestStatement: []AccountAnswer{}, CurrentPassport: []HolderAnswer{}}}
	if dir != nil {
		out.Directory = "on"
	}
	keep := carded(results, dir)
	names := map[string]map[string]int{} // key -> texts the key is spelt from
	cards := map[string]bool{}
	for _, r := range results {
		d := DocOut{ID: r.ID, Orgs: []OrgOut{}, Identifiers: []IdentOut{}, Links: []LinkOut{}, SameOrg: []SameOrgOut{}}
		used := map[string]bool{}
		for _, m := range r.Mentions {
			o := OrgOut{Span: m.Span}
			if m.Key != "" {
				o.Org, o.SettledBy = ptr(m.Key), ptr(m.SettledBy)
				used[m.Key] = true
				if "name:"+matchKey(m.Text) == m.Key {
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
		at := map[[2]string]int{}
		for _, l := range r.Links {
			if !keep(l.Key) {
				continue
			}
			used[l.Key] = true
			i, ok := at[[2]string{l.Ident, l.Key}]
			if !ok {
				i = len(d.Links)
				at[[2]string{l.Ident, l.Key}] = i
				d.Links = append(d.Links, LinkOut{Identifier: l.Ident, Org: l.Key, Tier: Likely})
			}
			if !contains(d.Links[i].Reasons, l.Reason) {
				d.Links[i].Reasons = append(d.Links[i].Reasons, l.Reason)
			}
			if l.Via != "" && d.Links[i].Via == "" {
				d.Links[i].Via = l.Via
			}
			if tierOf(linkClaim(l.Reason), l.proof) == Certain {
				d.Links[i].Tier = Certain
			}
		}
		if s := r.Statement; s != nil {
			used[s.Issuer.Key] = true
			so := &StmtOut{From: s.From.String(), To: s.To.String(), Issuer: s.Issuer.Key, Tiers: map[string]Tier{
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
		out.Organisations[k] = card(k, names[k], dir)
	}
	out.Answers.LatestStatement = latestStatements(results, func(key string) string {
		if c := out.Organisations[key]; c.Directory != nil {
			return "dir:" + *c.Directory
		}
		return key
	})
	out.Answers.CurrentPassport = currentPassports(results)
	return out
}

// card says who a key is: with the directory, the entry its own spelling
// looks up to; else the printed name it is spelt from most often. A code
// key with no entry has no name.
func card(key string, printed map[string]int, dir *Directory) Card {
	if dir != nil {
		if id, ok := dir.lookupKey(key); ok {
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
func carded(results []DocResult, dir *Directory) func(string) bool {
	if dir == nil {
		return func(string) bool { return true }
	}
	ties := newUnion()
	for _, r := range results {
		for _, j := range r.Joins {
			ties.join(j.A, j.B)
		}
	}
	named := map[string]bool{}
	for _, r := range results {
		for _, m := range r.Mentions {
			if m.Key != "" {
				named[ties.find(m.Key)] = true
			}
		}
	}
	return func(key string) bool {
		_, held := dir.lookupKey(key)
		return strings.HasPrefix(key, "name:") || held || named[ties.find(key)]
	}
}

// sameOrgRows writes a document's joins between keys it uses, one row per
// two keys and reason, in a fixed order. A join of two keys whose cards
// give two directory entries is not written: the directory says they are
// two.
func sameOrgRows(joins []Join, used map[string]bool, dir *Directory) []SameOrgOut {
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
		keys := [2]string{j.A, j.B}
		if keys[1] < keys[0] {
			keys[0], keys[1] = keys[1], keys[0]
		}
		row := SameOrgOut{Keys: keys, Reason: j.Reason, Tier: tierOf(joinClaim(j.Reason), j.proof)}
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
// and so is a later statement of the same issuer whose account was not
// read: it may be this account's. org gives a token's organisation.
func latestStatements(results []DocResult, org func(string) string) []AccountAnswer {
	type stmt struct {
		doc       string
		to        Date
		issuer    string
		spellings []string
		iban      string
	}
	var all []stmt
	unread := map[string][]Date{} // issuer -> periods' ends
	for _, r := range results {
		if r.Unread != nil {
			unread[org(r.Unread.Issuer.Key)] = append(unread[org(r.Unread.Issuer.Key)], r.Unread.To)
		}
		if r.Statement == nil {
			continue
		}
		s := stmt{doc: r.ID, to: r.Statement.To, issuer: org(r.Statement.Issuer.Key)}
		for _, a := range r.Statement.Accounts {
			s.spellings = append(s.spellings, a.Value)
			if a.Kind == KindIBAN {
				s.iban = a.Value
			}
		}
		all = append(all, s)
	}
	u := newUnion()
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
		if len(g) > 1 && g[0].to == g[1].to && g[0].doc != g[1].doc || laterThan(unread[g[0].issuer], g[0].to) {
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

func laterThan(ds []Date, d Date) bool {
	for _, e := range ds {
		if d.before(e) {
			return true
		}
	}
	return false
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
