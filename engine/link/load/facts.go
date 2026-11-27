package load

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"slices"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/pack"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
)

// ReadRun reads a format 3 run and refuses one whose documents repeat an id
// or point at an identifier they do not hold, before anything is written.
func ReadRun(path string) (*extract.Output, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var run extract.Output
	if err := json.Unmarshal(raw, &run); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	if run.Format != 3 {
		return nil, fmt.Errorf("%s: format %d, want 3", path, run.Format)
	}
	seen := map[string]bool{}
	for _, doc := range run.Documents {
		if doc.ID == "" || seen[doc.ID] {
			return nil, fmt.Errorf("%s: document id %q is empty or repeated", path, doc.ID)
		}
		seen[doc.ID] = true
		if err := checkRefs(doc); err != nil {
			return nil, fmt.Errorf("%s: document %s: %w", path, doc.ID, err)
		}
	}
	return &run, nil
}

func checkRefs(doc extract.DocOut) error {
	held := map[string]bool{}
	for _, ident := range doc.Identifiers {
		held[ident.ID] = true
	}
	var refs []string
	for _, l := range doc.Links {
		refs = append(refs, l.Identifier)
		if l.Via != "" {
			refs = append(refs, l.Via)
		}
	}
	if s := doc.Statement; s != nil {
		refs = append(refs, s.Accounts...)
	}
	if p := doc.Passport; p != nil {
		for _, r := range []*string{p.Number, p.MRZ} {
			if r != nil {
				refs = append(refs, *r)
			}
		}
	}
	for _, r := range refs {
		if !held[r] {
			return fmt.Errorf("no identifier %q", r)
		}
	}
	return nil
}

// Facts is everything one document of a run says, as one source record's
// facts. Rows that format 3 calls one claim ("Ids": a link's document,
// identifier and key; a same_org row's document and two keys) become one
// link holding the union of their reasons, certain when one row is. An
// organisation holds, as printed, the text of each of this document's
// mentions of it: what this document prints for that key, nothing more.
func Facts(doc extract.DocOut, cards map[string]extract.Card) interpreter.RecordFacts {
	f := newFacts(doc.ID, cards)
	for _, m := range doc.Orgs {
		if m.Org != nil {
			f.printed[*m.Org] = append(f.printed[*m.Org], m.Span.Text)
		}
	}
	idents := map[string]string{}
	for _, ident := range doc.Identifiers {
		idents[ident.ID] = f.identifier(ident.Kind, ident.Value)
	}
	for _, m := range doc.Orgs {
		if m.Org == nil {
			continue
		}
		attrs := spanAttrs(m.Span)
		if m.SettledBy != nil {
			attrs[pack.SettledBy] = *m.SettledBy
		}
		f.relation(ref("mentions", doc.ID, m.Page, m.Start, m.End), pack.Mentions, attrs,
			player(pack.Mentions, "document", f.doc), player(pack.Mentions, "organisation", f.org(*m.Org)))
	}
	for _, ident := range doc.Identifiers {
		attrs := spanAttrs(ident.Span)
		attrs[pack.Tier] = string(ident.Tier)
		if ident.Valid != nil {
			attrs[pack.Valid] = *ident.Valid
		}
		f.relation(ref("shows", doc.ID, ident.Page, ident.Start, ident.End), pack.Shows, attrs,
			player(pack.Shows, "document", f.doc), player(pack.Shows, "identifier", idents[ident.ID]))
	}
	for _, l := range doc.Links {
		var via []string
		if l.Via != "" {
			via = []string{idents[l.Via]}
		}
		ident := idents[l.Identifier]
		f.claim(ref("heldAt", doc.ID, ident, l.Org), pack.HeldAt, l.Reasons, via, l.Tier,
			player(pack.HeldAt, "identifier", ident), player(pack.HeldAt, "organisation", f.org(l.Org)), player(pack.HeldAt, "document", f.doc))
	}
	for _, s := range doc.SameOrg {
		keys := []string{s.Keys[0], s.Keys[1]}
		slices.Sort(keys)
		f.claim(ref("sameOrg", doc.ID, keys[0], keys[1]), pack.SameOrg, []string{s.Reason}, nil, s.Tier,
			player(pack.SameOrg, "one", f.org(keys[0])), player(pack.SameOrg, "other", f.org(keys[1])), player(pack.SameOrg, "document", f.doc))
	}
	if s := doc.Statement; s != nil {
		stmt := f.entity("statement:"+doc.ID, pack.Statement, map[string]any{pack.From: s.From, pack.To: s.To, pack.PeriodTier: string(s.Tiers["period"])})
		f.relation(ref("statementOf", doc.ID), pack.StatementOf, nil, player(pack.StatementOf, "statement", stmt), player(pack.StatementOf, "document", f.doc))
		for _, a := range s.Accounts {
			f.relation(ref("statementAccount", doc.ID, idents[a]), pack.StatementAccount, map[string]any{pack.Tier: string(s.Tiers["accounts"])},
				player(pack.StatementAccount, "statement", stmt), player(pack.StatementAccount, "account", idents[a]))
		}
		if s.Issuer != "" {
			f.relation(ref("issuedBy", doc.ID, s.Issuer), pack.IssuedBy, map[string]any{pack.Tier: string(s.Tiers["issuer"])},
				player(pack.IssuedBy, "statement", stmt), player(pack.IssuedBy, "issuer", f.org(s.Issuer)))
		}
	}
	if p := doc.Passport; p != nil {
		f.passport(p, idents)
	}
	return f.done()
}

func (f *facts) passport(p *extract.PassportOut, idents map[string]string) {
	attrs := map[string]any{}
	if p.Issued != nil {
		attrs[pack.Issued] = *p.Issued
	}
	if p.Expiry != nil {
		attrs[pack.Expiry] = *p.Expiry
		attrs[pack.ExpiryTier] = string(p.Tiers["expiry"])
	}
	if at := p.ExpiryAt; at != nil {
		attrs[pack.ExpiryPage], attrs[pack.ExpiryStart], attrs[pack.ExpiryEnd], attrs[pack.ExpiryText] = int64(at.Page), int64(at.Start), int64(at.End), at.Text
	}
	if p.Current != nil {
		attrs[pack.Current] = *p.Current
		attrs[pack.CurrentTier] = string(p.Tiers["current"])
	}
	pass := f.entity("passport:"+f.docID, pack.Passport, attrs)
	f.relation(ref("passportOf", f.docID), pack.PassportOf, nil, player(pack.PassportOf, "passport", pass), player(pack.PassportOf, "document", f.doc))
	if tier, ok := p.Tiers["holder"]; ok {
		holder := f.entity("holder:"+f.docID, pack.Holder, map[string]any{pack.Surname: p.Holder.Surname, pack.Given: p.Holder.Given})
		f.relation(ref("holderOf", f.docID), pack.HolderOf, map[string]any{pack.Tier: string(tier)},
			player(pack.HolderOf, "holder", holder), player(pack.HolderOf, "passport", pass))
	}
	if p.Number != nil {
		f.relation(ref("passportNumber", f.docID), pack.PassportNumber, map[string]any{pack.Tier: string(p.Tiers["number"])},
			player(pack.PassportNumber, "passport", pass), player(pack.PassportNumber, "identifier", idents[*p.Number]))
	}
	if p.MRZ != nil {
		f.relation(ref("passportMRZ", f.docID), pack.PassportMRZ, nil,
			player(pack.PassportMRZ, "passport", pass), player(pack.PassportMRZ, "identifier", idents[*p.MRZ]))
	}
}

// facts gathers one document's things and links by id, so a thing several
// rows name is given once.
type facts struct {
	docID, doc string
	cards      map[string]extract.Card
	printed    map[string][]string
	entities   map[string]ontology.ObservedEntity
	relations  map[string]ontology.ObservedRelation
	claims     map[string]*claim
}

// claim is a link that several rows may state: the union of their reasons
// and via codes, certain when one row is.
type claim struct {
	relation ontology.ObservedRelation
	reasons  []string
	via      []string
	tiers    []extract.Tier
}

// tier is certain when one row is, as sample/FORMAT.md says under "Tiers":
// "A link with several reasons is certain when one of them passes its
// test." Its "Ids" section makes rows with one id one claim. A tier format
// 3 does not have is passed on as given, so the schema refuses the record
// rather than the loader choosing for it.
func (c *claim) tier() extract.Tier {
	tier := extract.Likely
	for _, t := range c.tiers {
		switch t {
		case extract.Certain:
			tier = extract.Certain
		case extract.Likely:
		default:
			return t
		}
	}
	return tier
}

func newFacts(docID string, cards map[string]extract.Card) *facts {
	f := &facts{docID: docID, cards: cards, printed: map[string][]string{}, entities: map[string]ontology.ObservedEntity{}, relations: map[string]ontology.ObservedRelation{}, claims: map[string]*claim{}}
	f.doc = f.entity(DocumentPrefix+docID, pack.Document, nil)
	return f
}

// DocumentPrefix starts the id of every document's thing.
const DocumentPrefix = "document:"

func (f *facts) entity(id, typeID string, attrs map[string]any) string {
	f.entities[id] = ontology.ObservedEntity{ID: id, TypeID: typeID, Attributes: attrs}
	return id
}

func (f *facts) org(key string) string {
	attrs := map[string]any{pack.Key: key}
	if card, ok := f.cards[key]; ok {
		if card.Name != nil {
			attrs[pack.Name] = *card.Name
		}
		if card.Directory != nil {
			attrs[pack.DirectoryID] = *card.Directory
		}
	}
	if printed := set(f.printed[key]); len(printed) > 0 {
		attrs[pack.Printed] = printed
	}
	return f.entity(OrgID(key), pack.Organisation, attrs)
}

func (f *facts) identifier(kind, value string) string {
	return f.entity(IdentifierID(kind, value), pack.Identifier, map[string]any{pack.Kind: kind, pack.Value: value})
}

// OrgID is the id of the organisation with key, and IdentifierID the id of
// the number or code of kind with value. The schema hides a key and masks a
// value, so neither is in the id: the id is a digest of it, the same on
// every load and every machine.
func OrgID(key string) string { return "org:" + digest(key) }

func IdentifierID(kind, value string) string { return "identifier:" + digest(kind+":"+value) }

// digest is the first 64 bits of the SHA-256 of s, in hex. Among a million
// things, the odds that two share one are below one in thirty million.
func digest(s string) string {
	sum := sha256.Sum256([]byte(s))
	return hex.EncodeToString(sum[:8])
}

func (f *facts) relation(id, typeID string, attrs map[string]any, players ...ontology.RolePlayer) {
	f.relations[id] = ontology.ObservedRelation{ID: id, TypeID: typeID, Players: players, Attributes: attrs}
}

func (f *facts) claim(id, typeID string, reasons, via []string, tier extract.Tier, players ...ontology.RolePlayer) {
	c := f.claims[id]
	if c == nil {
		c = &claim{relation: ontology.ObservedRelation{ID: id, TypeID: typeID, Players: players}}
		f.claims[id] = c
	}
	c.reasons = append(c.reasons, reasons...)
	c.via = append(c.via, via...)
	c.tiers = append(c.tiers, tier)
}

func (f *facts) done() interpreter.RecordFacts {
	for id, c := range f.claims {
		r := c.relation
		r.Attributes = map[string]any{pack.Tier: string(c.tier())}
		if reasons := set(c.reasons); len(reasons) > 0 {
			r.Attributes[pack.Reasons] = reasons
		}
		if via := set(c.via); len(via) > 0 {
			r.Attributes[pack.Via] = via
		}
		f.relations[id] = r
	}
	var out interpreter.RecordFacts
	for _, id := range sortedKeys(f.entities) {
		out.Entities = append(out.Entities, f.entities[id])
	}
	for _, id := range sortedKeys(f.relations) {
		out.Relations = append(out.Relations, f.relations[id])
	}
	return out
}

func set(values []string) []any {
	values = slices.Clone(values)
	slices.Sort(values)
	var out []any
	for _, v := range slices.Compact(values) {
		out = append(out, v)
	}
	return out
}

func sortedKeys[V any](m map[string]V) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	slices.Sort(keys)
	return keys
}

func player(relation, role, id string) ontology.RolePlayer {
	return ontology.RolePlayer{RoleTypeID: relation + ":" + role, PlayerID: id, PlayerKind: ontology.PlayerEntity}
}

func spanAttrs(s extract.Span) map[string]any {
	return map[string]any{pack.Page: int64(s.Page), pack.Start: int64(s.Start), pack.End: int64(s.End), pack.Text: s.Text}
}

// ref is a link's id: its kind and the parts format 3 names the claim by.
func ref(kind string, parts ...any) string {
	b, err := json.Marshal(parts)
	if err != nil {
		panic(err)
	}
	return kind + ":" + string(b)
}
