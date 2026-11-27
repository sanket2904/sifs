package card

import (
	"cmp"
	"context"
	"slices"
	"strings"
	"unicode"

	"github.com/sanket2904/sifs/engine/link/load"
	"github.com/sanket2904/sifs/engine/link/pack"

	"github.com/sanket2904/ontology-framework/ontology"
)

// Answer is the cards a typed word or phrase gives, best first, and the
// documents that print a matching name on a key the first card does not
// hold.
type Answer struct {
	Query       string  `json:"query"`
	Cards       []Card  `json:"cards"`
	AlsoNamedIn []Named `json:"also_named_in"`
}

// Card is one organisation as far as the store's same_org links join its
// keys, and nothing more. Its head is the key every other key's join is
// measured from. StandsOn is "certain" when at least one fact on the card
// is certain on it, and "likely-only" otherwise (sample/FORMAT.md, "Cards
// on likely rows only").
type Card struct {
	Head       string      `json:"head"`
	StandsOn   string      `json:"stands_on"`
	Keys       []Key       `json:"keys"`
	Joins      []Join      `json:"joins"`
	Held       []Held      `json:"held"`
	Documents  []Doc       `json:"documents"`
	Statements []Statement `json:"statements"`
}

// Key is one organisation key on a card: its code when it is an
// institution code's key, the names documents print on it, and the tier of
// its join to the card's head, the best over every path of same_org links,
// a path being as strong as its weakest link.
type Key struct {
	Key    string   `json:"key"`
	Code   string   `json:"code,omitempty"`
	Names  []string `json:"names"`
	Joined Tier     `json:"joined"`
}

// Join is one same_org link between two keys of a card. CardTier is the
// lower of its tier and the better join of its two keys.
type Join struct {
	One      string   `json:"one"`
	Other    string   `json:"other"`
	Document string   `json:"document"`
	Reasons  []string `json:"reasons"`
	Tier     Tier     `json:"tier"`
	CardTier Tier     `json:"card_tier"`
}

// Held is an identifier held at the card, with every held_at link that says
// so. CardTier is the best of its links' card tiers.
type Held struct {
	Kind     string     `json:"kind"`
	Value    string     `json:"value"`
	CardTier Tier       `json:"card_tier"`
	Links    []HeldLink `json:"links"`
}

// HeldLink is one held_at link. CardTier is the lower of its tier and its
// key's join.
type HeldLink struct {
	Key      string       `json:"key"`
	Document string       `json:"document"`
	Reasons  []string     `json:"reasons"`
	Via      []Identifier `json:"via"`
	Tier     Tier         `json:"tier"`
	CardTier Tier         `json:"card_tier"`
}

// Identifier is an identifier's kind and value.
type Identifier struct {
	Kind  string `json:"kind"`
	Value string `json:"value"`
}

// Doc is a document that mentions the card, with every mention. A mention
// has no tier; CardTier is the best join of the keys its mentions are on.
type Doc struct {
	Document string    `json:"document"`
	CardTier Tier      `json:"card_tier"`
	Mentions []Mention `json:"mentions"`
}

// Mention is a printed name on a key, with its page and span.
type Mention struct {
	Key       string `json:"key"`
	Page      int    `json:"page"`
	Start     int    `json:"start"`
	End       int    `json:"end"`
	Text      string `json:"text"`
	SettledBy string `json:"settled_by,omitempty"`
}

// Statement is a statement issued by the card. Tier is the issuer's tier;
// CardTier the lower of it and the issuer key's join.
type Statement struct {
	Document   string `json:"document"`
	From       string `json:"from"`
	To         string `json:"to"`
	PeriodTier Tier   `json:"period_tier"`
	Issuer     string `json:"issuer"`
	Tier       Tier   `json:"tier"`
	CardTier   Tier   `json:"card_tier"`
}

// Named is a document that prints a matching name on a key the first card
// does not hold: it may be the same organisation, and nothing in the store
// says it is.
type Named struct {
	Document string    `json:"document"`
	Mentions []Mention `json:"mentions"`
}

var cardThings = []string{pack.Organisation, pack.Identifier, pack.Statement}
var cardLinks = []string{pack.SameOrg, pack.Mentions, pack.HeldAt, pack.IssuedBy, pack.StatementOf}

// Find gives the cards for a typed word or phrase. A card matches when a
// name printed on one of its keys holds the query's words as a run of whole
// words, case ignored, or when the query is one word that begins with the
// code of one of its institution code keys.
func Find(ctx context.Context, e *load.Engine, query string) (Answer, error) {
	s, err := read(ctx, e, cardThings, cardLinks)
	if err != nil {
		return Answer{}, err
	}
	return s.find(query), nil
}

func (s *snapshot) find(query string) Answer {
	words := wordsOf(query)
	names := map[string][]Mention{}
	for _, m := range s.links[pack.Mentions] {
		org := player(m, "organisation")
		names[org] = append(names[org], s.mention(m))
	}
	matched := map[string]bool{}
	for id, thing := range s.things {
		if thing.TypeID != pack.Organisation {
			continue
		}
		if codeMatches(code(s.key(id)), words) || slices.ContainsFunc(names[id], func(m Mention) bool { return nameMatches(m.Text, words) }) {
			matched[id] = true
		}
	}

	groups := s.groups()
	var cards []Card
	seen := map[string]bool{}
	for _, id := range sortedKeys(matched) {
		root := groups.find(id)
		if seen[root] {
			continue
		}
		seen[root] = true
		cards = append(cards, s.card(groups.members(root), names))
	}
	slices.SortFunc(cards, func(a, b Card) int {
		return cmp.Or(cmp.Compare(rank(a.StandsOn), rank(b.StandsOn)), cmp.Compare(len(cardDocuments(b)), len(cardDocuments(a))), cmp.Compare(a.Head, b.Head))
	})

	ans := Answer{Query: query, Cards: orEmpty(cards), AlsoNamedIn: []Named{}}
	onFirst := map[string]bool{}
	if len(cards) > 0 {
		for _, k := range cards[0].Keys {
			onFirst[k.Key] = true
		}
	}
	byDoc := map[string][]Mention{}
	for _, m := range s.links[pack.Mentions] {
		mention := s.mention(m)
		if !onFirst[mention.Key] && nameMatches(mention.Text, words) {
			doc := document(m, "document")
			byDoc[doc] = append(byDoc[doc], mention)
		}
	}
	for _, doc := range sortedKeys(byDoc) {
		ans.AlsoNamedIn = append(ans.AlsoNamedIn, Named{Document: doc, Mentions: sortMentions(byDoc[doc])})
	}
	return ans
}

func rank(standsOn string) int {
	if standsOn == "certain" {
		return 0
	}
	return 1
}

func cardDocuments(c Card) map[string]bool {
	docs := map[string]bool{}
	for _, d := range c.Documents {
		docs[d.Document] = true
	}
	for _, h := range c.Held {
		for _, l := range h.Links {
			docs[l.Document] = true
		}
	}
	for _, j := range c.Joins {
		docs[j.Document] = true
	}
	for _, st := range c.Statements {
		docs[st.Document] = true
	}
	return docs
}

// card builds one card from the organisation things one group of same_org
// links joins.
func (s *snapshot) card(members []string, names map[string][]Mention) Card {
	inCard := map[string]bool{}
	for _, id := range members {
		inCard[id] = true
	}
	head := s.head(members)
	joined := s.joined(head, inCard)
	c := Card{Head: s.key(head), StandsOn: "likely-only", Joins: []Join{}, Held: []Held{}, Documents: []Doc{}, Statements: []Statement{}}
	certain := func(t Tier) {
		if t == Certain {
			c.StandsOn = "certain"
		}
	}

	for _, id := range members {
		var printed []string
		for _, m := range names[id] {
			printed = append(printed, m.Text)
		}
		slices.Sort(printed)
		c.Keys = append(c.Keys, Key{Key: s.key(id), Code: code(s.key(id)), Names: orEmpty(slices.Compact(printed)), Joined: joined[id]})
	}
	slices.SortFunc(c.Keys, func(a, b Key) int { return cmp.Compare(a.Key, b.Key) })

	for _, r := range s.links[pack.SameOrg] {
		one, other := player(r, "one"), player(r, "other")
		if !inCard[one] {
			continue
		}
		j := Join{One: s.key(one), Other: s.key(other), Document: document(r, "document"), Reasons: strs(r.Attributes, pack.Reasons), Tier: tierOf(r)}
		j.CardTier = lower(j.Tier, higher(joined[one], joined[other]))
		certain(j.CardTier)
		c.Joins = append(c.Joins, j)
	}
	slices.SortFunc(c.Joins, func(a, b Join) int {
		return cmp.Or(cmp.Compare(a.One, b.One), cmp.Compare(a.Other, b.Other), cmp.Compare(a.Document, b.Document))
	})

	held := map[string]*Held{}
	for _, r := range s.links[pack.HeldAt] {
		org := player(r, "organisation")
		if !inCard[org] {
			continue
		}
		identID := player(r, "identifier")
		h := held[identID]
		if h == nil {
			ident := s.identifier(identID)
			h = &Held{Kind: ident.Kind, Value: ident.Value, CardTier: Likely}
			held[identID] = h
		}
		l := HeldLink{Key: s.key(org), Document: document(r, "document"), Reasons: strs(r.Attributes, pack.Reasons), Via: []Identifier{}, Tier: tierOf(r)}
		for _, v := range strs(r.Attributes, pack.Via) {
			l.Via = append(l.Via, s.identifier(v))
		}
		slices.SortFunc(l.Via, func(a, b Identifier) int { return cmp.Or(cmp.Compare(a.Kind, b.Kind), cmp.Compare(a.Value, b.Value)) })
		l.CardTier = lower(l.Tier, joined[org])
		h.CardTier = higher(h.CardTier, l.CardTier)
		certain(l.CardTier)
		h.Links = append(h.Links, l)
	}
	for _, h := range held {
		slices.SortFunc(h.Links, func(a, b HeldLink) int { return cmp.Or(cmp.Compare(a.Key, b.Key), cmp.Compare(a.Document, b.Document)) })
		c.Held = append(c.Held, *h)
	}
	slices.SortFunc(c.Held, func(a, b Held) int { return cmp.Or(cmp.Compare(a.Kind, b.Kind), cmp.Compare(a.Value, b.Value)) })

	docs := map[string]*Doc{}
	for _, r := range s.links[pack.Mentions] {
		org := player(r, "organisation")
		if !inCard[org] {
			continue
		}
		id := document(r, "document")
		d := docs[id]
		if d == nil {
			d = &Doc{Document: id, CardTier: Likely}
			docs[id] = d
		}
		d.CardTier = higher(d.CardTier, joined[org])
		d.Mentions = append(d.Mentions, s.mention(r))
	}
	for _, id := range sortedKeys(docs) {
		d := docs[id]
		d.Mentions = sortMentions(d.Mentions)
		c.Documents = append(c.Documents, *d)
	}

	statementDoc := map[string]string{}
	for _, r := range s.links[pack.StatementOf] {
		statementDoc[player(r, "statement")] = document(r, "document")
	}
	for _, r := range s.links[pack.IssuedBy] {
		issuer := player(r, "issuer")
		if !inCard[issuer] {
			continue
		}
		stmt := s.things[player(r, "statement")]
		if stmt == nil {
			continue
		}
		st := Statement{Document: statementDoc[stmt.ID], From: str(stmt.Attributes, pack.From), To: str(stmt.Attributes, pack.To),
			PeriodTier: tier(stmt.Attributes, pack.PeriodTier), Issuer: s.key(issuer), Tier: tierOf(r)}
		st.CardTier = lower(st.Tier, joined[issuer])
		certain(st.CardTier)
		c.Statements = append(c.Statements, st)
	}
	slices.SortFunc(c.Statements, func(a, b Statement) int { return cmp.Or(cmp.Compare(a.Document, b.Document), cmp.Compare(a.Issuer, b.Issuer)) })
	return c
}

// head is the key every join on a card is measured from: the first code
// key in key order, because a code's key is the same in every document;
// with no code key, the first key in key order. It does not depend on the
// query, so one store gives one card whatever was typed.
func (s *snapshot) head(members []string) string {
	ordered := slices.Clone(members)
	slices.SortFunc(ordered, func(a, b string) int {
		ca, cb := code(s.key(a)) != "", code(s.key(b)) != ""
		if ca != cb {
			if ca {
				return -1
			}
			return 1
		}
		return cmp.Compare(s.key(a), s.key(b))
	})
	return ordered[0]
}

// joined is each key's join to head: certain when a path of certain
// same_org links reaches it, likely otherwise. A path is as strong as its
// weakest link, and the head is certain to be itself.
func (s *snapshot) joined(head string, inCard map[string]bool) map[string]Tier {
	out := map[string]Tier{}
	for id := range inCard {
		out[id] = Likely
	}
	out[head] = Certain
	next := []string{head}
	for len(next) > 0 {
		at := next[0]
		next = next[1:]
		for _, r := range s.links[pack.SameOrg] {
			if tierOf(r) != Certain {
				continue
			}
			one, other := player(r, "one"), player(r, "other")
			for _, pair := range [][2]string{{one, other}, {other, one}} {
				if pair[0] == at && out[pair[1]] != Certain {
					out[pair[1]] = Certain
					next = append(next, pair[1])
				}
			}
		}
	}
	return out
}

// groups joins organisation things through the store's same_org links and
// nothing else.
func (s *snapshot) groups() *unionFind {
	u := &unionFind{parent: map[string]string{}}
	for id, thing := range s.things {
		if thing.TypeID == pack.Organisation {
			u.parent[id] = id
		}
	}
	for _, r := range s.links[pack.SameOrg] {
		u.union(player(r, "one"), player(r, "other"))
	}
	return u
}

type unionFind struct{ parent map[string]string }

func (u *unionFind) find(id string) string {
	for u.parent[id] != id {
		u.parent[id] = u.parent[u.parent[id]]
		id = u.parent[id]
	}
	return id
}

func (u *unionFind) union(a, b string) {
	if _, ok := u.parent[a]; !ok {
		return
	}
	if _, ok := u.parent[b]; !ok {
		return
	}
	u.parent[u.find(a)] = u.find(b)
}

func (u *unionFind) members(root string) []string {
	var out []string
	for id := range u.parent {
		if u.find(id) == root {
			out = append(out, id)
		}
	}
	slices.Sort(out)
	return out
}

func (s *snapshot) key(orgID string) string {
	if thing := s.things[orgID]; thing != nil {
		return str(thing.Attributes, pack.Key)
	}
	return strings.TrimPrefix(orgID, "org:")
}

func (s *snapshot) identifier(id string) Identifier {
	if thing := s.things[id]; thing != nil {
		return Identifier{Kind: str(thing.Attributes, pack.Kind), Value: str(thing.Attributes, pack.Value)}
	}
	return Identifier{}
}

func (s *snapshot) mention(r *ontology.Relation) Mention {
	m := Mention{Key: s.key(player(r, "organisation")), Text: str(r.Attributes, pack.Text), SettledBy: str(r.Attributes, pack.SettledBy)}
	m.Page, _ = integer(r.Attributes, pack.Page)
	m.Start, _ = integer(r.Attributes, pack.Start)
	m.End, _ = integer(r.Attributes, pack.End)
	return m
}

func sortMentions(ms []Mention) []Mention {
	slices.SortFunc(ms, func(a, b Mention) int {
		return cmp.Or(cmp.Compare(a.Page, b.Page), cmp.Compare(a.Start, b.Start), cmp.Compare(a.Key, b.Key))
	})
	return ms
}

func sortedKeys[V any](m map[string]V) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	slices.Sort(keys)
	return keys
}

// orEmpty gives an empty list for none, so the JSON says [] and not null.
func orEmpty[T any](v []T) []T {
	if v == nil {
		return []T{}
	}
	return v
}

// codePrefixes are the institution code keys' kinds (sample/FORMAT.md,
// "Code keys"). Every other key is a document key.
var codePrefixes = []string{"IFSC ", "BIC ", "IBAN "}

// code is an institution code key's code, its spaces removed ("IFSC HDFC"
// gives "HDFC", "IBAN DE 37040044" gives "DE37040044"), or "" for a
// document key.
func code(key string) string {
	for _, p := range codePrefixes {
		if rest, ok := strings.CutPrefix(key, p); ok {
			return strings.ReplaceAll(rest, " ", "")
		}
	}
	return ""
}

// wordsOf splits text into lower-case words at every character that is
// not a letter, a mark or a number.
func wordsOf(text string) []string {
	return strings.FieldsFunc(strings.ToLower(text), func(r rune) bool {
		return !unicode.IsLetter(r) && !unicode.IsMark(r) && !unicode.IsNumber(r)
	})
}

// nameMatches reports whether a printed name holds the query's words as a
// run of whole words.
func nameMatches(name string, query []string) bool {
	if len(query) == 0 {
		return false
	}
	words := wordsOf(name)
	for i := 0; i+len(query) <= len(words); i++ {
		if slices.Equal(words[i:i+len(query)], query) {
			return true
		}
	}
	return false
}

// codeMatches reports whether the query is one word that begins with the
// code, as a typed IFSC begins with its bank's four letters.
func codeMatches(code string, query []string) bool {
	return code != "" && len(query) == 1 && strings.HasPrefix(query[0], strings.ToLower(code))
}
