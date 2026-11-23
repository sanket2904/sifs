package extract

import (
	"sort"
	"strings"
)

// union joins tokens that name one organisation. It refuses a join that
// would put two directory entries in one set: that would be a merge.
type union struct {
	parent map[string]string
	dir    map[string]string // root -> the directory token in its set
}

func newUnion() *union { return &union{parent: map[string]string{}, dir: map[string]string{}} }

func (u *union) find(x string) string {
	if _, ok := u.parent[x]; !ok {
		u.parent[x] = x
		if strings.HasPrefix(x, "dir:") {
			u.dir[x] = x
		}
	}
	for u.parent[x] != x {
		u.parent[x] = u.parent[u.parent[x]]
		x = u.parent[x]
	}
	return x
}

func (u *union) join(a, b string) bool {
	ra, rb := u.find(a), u.find(b)
	if ra == rb {
		return true
	}
	da, db := u.dir[ra], u.dir[rb]
	if da != "" && db != "" && da != db {
		return false
	}
	u.parent[rb] = ra
	if da == "" {
		u.dir[ra] = db
	}
	return true
}

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
}

type LinkOut struct {
	Identifier string   `json:"identifier"`
	Org        string   `json:"org"`
	Reasons    []string `json:"reasons"`
	Via        string   `json:"via,omitempty"`
}

type StmtOut struct {
	From     string   `json:"from"`
	To       string   `json:"to"`
	Accounts []string `json:"accounts"`
	Issuer   string   `json:"issuer"`
}

type PassportOut struct {
	Holder   HolderOut `json:"holder"`
	Number   *string   `json:"number"`
	MRZ      *string   `json:"mrz"`
	Issued   *string   `json:"issued"`
	Expiry   *string   `json:"expiry"`
	ExpiryAt *Span     `json:"expiry_at"`
	Current  *bool     `json:"current"`
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

// Run extracts every document and joins their organisation tokens into
// keys. With a directory, a token that names exactly one entry joins it;
// without one, the program knows no organisation names at all.
func Run(docs []Document, dir *Directory, ref Date) Output {
	results := make([]DocResult, len(docs))
	for i, d := range docs {
		results[i] = Extract(d, ref)
	}
	u := newUnion()
	if dir != nil {
		for i := range results {
			r := &results[i]
			for j := range r.Mentions {
				m := &r.Mentions[j]
				if m.Token == "" && m.kind == acronymName {
					if id, ok := dir.lookupName(m.Text); ok {
						m.Token, m.SettledBy = "dir:"+id, "directory"
					}
				}
			}
			// a name the document settles that is exactly one entry's name
			for _, m := range r.Mentions {
				if m.Token != "" && !strings.HasPrefix(m.Token, "dir:") {
					if id, ok := dir.lookupName(m.Text); ok {
						u.join("dir:"+id, m.Token)
					}
				}
			}
			for _, tok := range tokens(*r) {
				var id string
				var ok bool
				if k, isName := strings.CutPrefix(tok, "name:"); isName {
					id, ok = only(dir.byName[k])
				} else if !strings.HasPrefix(tok, "dir:") {
					id, ok = dir.lookupCode(tok)
				}
				if ok {
					u.join("dir:"+id, tok)
				}
			}
		}
	}
	for _, r := range results {
		for _, t := range r.Ties {
			u.join(t.A, t.B)
		}
	}
	return render(results, u, dir, ref)
}

// tokens lists every organisation token a document uses.
func tokens(r DocResult) []string {
	var out []string
	for _, m := range r.Mentions {
		if m.Token != "" {
			out = append(out, m.Token)
		}
	}
	for _, l := range r.Links {
		out = append(out, l.Token)
	}
	for _, t := range r.Ties {
		out = append(out, t.A, t.B)
	}
	return out
}

func render(results []DocResult, u *union, dir *Directory, ref Date) Output {
	out := Output{Format: 1, Directory: "off", Organisations: map[string]Card{}, Documents: []DocOut{}, Answers: Answers{LatestStatement: []AccountAnswer{}, CurrentPassport: []HolderAnswer{}}}
	if dir != nil {
		out.Directory = "on"
	}
	keys := map[string]string{} // root -> key
	names := map[string]map[string]int{}
	key := func(tok string) string {
		r := u.find(tok)
		if k, ok := keys[r]; ok {
			return k
		}
		k := "org-" + itoa(len(keys)+1)
		keys[r] = k
		names[r] = map[string]int{}
		return k
	}
	for _, r := range results {
		d := DocOut{ID: r.ID, Orgs: []OrgOut{}, Identifiers: []IdentOut{}, Links: []LinkOut{}}
		for _, m := range r.Mentions {
			o := OrgOut{Span: m.Span}
			if m.Token != "" {
				o.Org, o.SettledBy = ptr(key(m.Token)), ptr(m.SettledBy)
				if m.SettledBy == "name" {
					names[u.find(m.Token)][m.Text]++
				}
			}
			d.Orgs = append(d.Orgs, o)
		}
		for _, id := range r.Identifiers {
			d.Identifiers = append(d.Identifiers, IdentOut{Span: id.Span, ID: id.ID, Kind: id.Kind, Value: id.Value, Valid: id.Valid})
		}
		at := map[string]int{}
		for _, l := range r.Links {
			k := key(l.Token)
			i, ok := at[l.Ident+"|"+k]
			if !ok {
				i = len(d.Links)
				at[l.Ident+"|"+k] = i
				d.Links = append(d.Links, LinkOut{Identifier: l.Ident, Org: k})
			}
			if !contains(d.Links[i].Reasons, l.Reason) {
				d.Links[i].Reasons = append(d.Links[i].Reasons, l.Reason)
			}
			if l.Via != "" && d.Links[i].Via == "" {
				d.Links[i].Via = l.Via
			}
		}
		if s := r.Statement; s != nil {
			so := &StmtOut{From: s.From.String(), To: s.To.String(), Issuer: key(s.Issuer.Token)}
			for _, a := range s.Accounts {
				so.Accounts = append(so.Accounts, a.ID)
			}
			d.Statement = so
		}
		if p := r.Passport; p != nil {
			po := &PassportOut{Holder: HolderOut{p.Surname, p.Given}, ExpiryAt: p.ExpiryAt, Current: p.Current}
			if p.Number != nil {
				po.Number = ptr(p.Number.ID)
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
			d.Passport = po
		}
		if len(d.Orgs)+len(d.Identifiers) > 0 || d.Statement != nil || d.Passport != nil {
			out.Documents = append(out.Documents, d)
		}
	}
	for root, k := range keys {
		c := Card{}
		if dt := u.dir[root]; dt != "" {
			id := strings.TrimPrefix(dt, "dir:")
			c.Directory, c.Name = ptr(id), ptr(dir.entries[id].Name)
		} else if n := mostUsed(names[root]); n != "" {
			c.Name = ptr(n)
		}
		out.Organisations[k] = c
	}
	out.Answers.LatestStatement = latestStatements(results, u.find)
	out.Answers.CurrentPassport = currentPassports(results)
	return out
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
			unread[org(r.Unread.Issuer.Token)] = append(unread[org(r.Unread.Issuer.Token)], r.Unread.To)
		}
		if r.Statement == nil {
			continue
		}
		s := stmt{doc: r.ID, to: r.Statement.To, issuer: org(r.Statement.Issuer.Token)}
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
