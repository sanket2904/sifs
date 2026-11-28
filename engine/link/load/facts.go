package load

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path"
	"slices"
	"strings"
	"time"

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
	f := newFacts(doc.ID, doc.File, cards)
	for _, m := range doc.Orgs {
		if m.Org != nil {
			f.printed[*m.Org] = append(f.printed[*m.Org], m.Span.Text)
		}
	}
	idents := map[string]string{}
	for _, ident := range doc.Identifiers {
		idents[ident.ID] = f.identifier(ident.Kind, ident.Value)
	}
	public := func(id string) bool { return f.entities[idents[id]].TypeID == pack.BankCode }
	for _, m := range doc.Orgs {
		if m.Org == nil {
			continue
		}
		attrs := spanAttrs(m.Span, pack.PrintedText)
		if m.SettledBy != nil {
			attrs[pack.SettledBy] = *m.SettledBy
		}
		f.relation(ref("mentions", doc.ID, m.Page, m.Start, m.End), pack.Mentions, attrs,
			player(pack.Mentions, "document", f.doc), player(pack.Mentions, "organisation", f.org(*m.Org)))
	}
	for _, ident := range doc.Identifiers {
		kind, typeID, role, text := "shows", pack.Shows, "identifier", pack.Text
		if public(ident.ID) {
			kind, typeID, role, text = "showsCode", pack.ShowsCode, "code", pack.PrintedText
		}
		attrs := spanAttrs(ident.Span, text)
		attrs[pack.Tier] = string(ident.Tier)
		if ident.Valid != nil {
			attrs[pack.Valid] = *ident.Valid
		}
		f.relation(ref(kind, doc.ID, ident.Page, ident.Start, ident.End), typeID, attrs,
			player(typeID, "document", f.doc), player(typeID, role, idents[ident.ID]))
	}
	for _, l := range doc.Links {
		var via []string
		if l.Via != "" {
			via = []string{idents[l.Via]}
		}
		ident := idents[l.Identifier]
		kind, typeID, role := "heldAt", pack.HeldAt, "identifier"
		if public(l.Identifier) {
			kind, typeID, role = "codeOf", pack.CodeOf, "code"
		}
		f.claim(ref(kind, doc.ID, ident, l.Org), typeID, l.Reasons, via, l.Tier,
			player(typeID, role, ident), player(typeID, "organisation", f.org(l.Org)), player(typeID, "document", f.doc))
	}
	for _, s := range doc.SameOrg {
		keys := []string{s.Keys[0], s.Keys[1]}
		slices.Sort(keys)
		f.claim(ref("sameOrg", doc.ID, keys[0], keys[1]), pack.SameOrg, []string{s.Reason}, nil, s.Tier,
			player(pack.SameOrg, "one", f.org(keys[0])), player(pack.SameOrg, "other", f.org(keys[1])), player(pack.SameOrg, "document", f.doc))
	}
	if s := doc.Statement; s != nil {
		attrs := map[string]any{pack.PeriodTier: string(s.Tiers["period"])}
		putDate(attrs, pack.From, &s.From)
		putDate(attrs, pack.To, &s.To)
		if p := period(s.From, s.To); p != "" {
			attrs[pack.Period] = p
		}
		stmt := f.entity("statement:"+doc.ID, pack.Statement, attrs)
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
	putDate(attrs, pack.Issued, p.Issued)
	if p.Expiry != nil {
		putDate(attrs, pack.Expiry, p.Expiry)
		attrs[pack.ExpiryTier] = string(p.Tiers["expiry"])
	}
	if at := p.ExpiryAt; at != nil {
		attrs[pack.ExpiryPage], attrs[pack.ExpiryStart], attrs[pack.ExpiryEnd], attrs[pack.ExpiryText] = int64(at.Page), int64(at.Start), int64(at.End), at.Text
	} else if p.Expiry != nil && isDay(*p.Expiry) {
		attrs[pack.ExpiryDay] = *p.Expiry
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

func newFacts(docID, filePath string, cards map[string]extract.Card) *facts {
	f := &facts{docID: docID, cards: cards, printed: map[string][]string{}, entities: map[string]ontology.ObservedEntity{}, relations: map[string]ontology.ObservedRelation{}, claims: map[string]*claim{}}
	f.doc = f.entity(DocumentPrefix+docID, pack.Document, file(filePath))
	return f
}

// file is a document's file name and, when the file is in a folder, that
// folder, from a format 3 path with "/" between its parts.
func file(p string) map[string]any {
	if p == "" {
		return nil
	}
	attrs := map[string]any{pack.FileName: path.Base(p)}
	if dir := path.Dir(p); dir != "." {
		attrs[pack.Folder] = dir
	}
	return attrs
}

// DocumentPrefix starts the id of every document's thing.
const DocumentPrefix = "document:"

func (f *facts) entity(id, typeID string, attrs map[string]any) string {
	f.entities[id] = ontology.ObservedEntity{ID: id, TypeID: typeID, Attributes: attrs}
	return id
}

// putDate puts the day a format 3 date names, at midnight UTC, under attr.
// A date it does not spell as YYYY-MM-DD, such as an expiry the extractor
// marks `cannot-tell`, is left out: the claim's tier still says one was
// given.
func putDate(attrs map[string]any, attr string, date *string) {
	if date == nil {
		return
	}
	if day, err := time.Parse(time.DateOnly, *date); err == nil {
		attrs[attr] = day
	}
}

// period is a statement's title: its first and last day as format 3 spells
// them, or the one of the two that is a day.
func period(from, to string) string {
	days := slices.DeleteFunc([]string{from, to}, func(d string) bool { return !isDay(d) })
	return strings.Join(days, " to ")
}

// isDay reports whether a format 3 date is a day, spelt YYYY-MM-DD.
func isDay(date string) bool {
	_, err := time.Parse(time.DateOnly, date)
	return err == nil
}

func (f *facts) org(key string) string {
	attrs := map[string]any{pack.Key: key}
	if !strings.HasPrefix(key, documentKeyPrefix) {
		attrs[pack.Institution] = key
	}
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

// documentKeyPrefix starts a document key (sample/FORMAT.md, "No
// merging"); every other key is an institution code's key, spelt from a
// code that is printed for anyone to read.
const documentKeyPrefix = "name:"

// identifier is the thing a number or code of kind is: a bank code when
// the pack lists kind as public, otherwise a person's own number.
func (f *facts) identifier(kind, value string) string {
	if pack.Public(kind) {
		return f.entity(BankCodeID(kind, value), pack.BankCode, map[string]any{pack.Kind: kind, pack.Code: value})
	}
	return f.entity(IdentifierID(kind, value), pack.Identifier, map[string]any{pack.Kind: kind, pack.Value: value})
}

// OrgID is the id of the organisation with key, IdentifierID the id of the
// number of kind with value, and BankCodeID the id of the bank code of
// kind with value. None holds its key, value or code: the id is a digest
// of it, the same on every load and every machine.
func OrgID(key string) string { return "org:" + digest(key) }

func IdentifierID(kind, value string) string { return "identifier:" + digest(kind+":"+value) }

func BankCodeID(kind, code string) string { return "bankcode:" + digest(kind+":"+code) }

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

// spanAttrs is where a span is, and its text under text.
func spanAttrs(s extract.Span, text string) map[string]any {
	return map[string]any{pack.Page: int64(s.Page), pack.Start: int64(s.Start), pack.End: int64(s.End), text: s.Text}
}

// ref is a link's id: its kind and the parts format 3 names the claim by.
func ref(kind string, parts ...any) string {
	b, err := json.Marshal(parts)
	if err != nil {
		panic(err)
	}
	return kind + ":" + string(b)
}
