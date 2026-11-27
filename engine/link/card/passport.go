package card

import (
	"cmp"
	"context"
	"slices"
	"strings"

	"github.com/sanket2904/sifs/engine/link/load"
	"github.com/sanket2904/sifs/engine/link/pack"
)

// Asked is the answer to a typed question. Only one kind is answered yet:
// when a passport expires, for every passport the store holds, because
// "my" may be any holder in the files.
type Asked struct {
	Question      string     `json:"question"`
	Answer        string     `json:"answer"`
	Message       string     `json:"message,omitempty"`
	ReferenceDate string     `json:"reference_date,omitempty"`
	Passports     []Passport `json:"passports"`
}

// The kinds of answer.
const (
	PassportExpiry = "passport-expiry"
	NotAnswered    = "not-answered-yet"
)

// Passport is one passport's line.
type Passport struct {
	Holder     string  `json:"holder"`
	HolderTier Tier    `json:"holder_tier,omitempty"`
	Document   string  `json:"document"`
	Expiry     Expiry  `json:"expiry"`
	Current    Current `json:"current"`
}

// Expiry gives a date only when the store's tier for it is certain and it
// has the page of the printed date. Status is "in-date", "past" (before the
// reference date) or "cannot-tell", and then Why says what is missing:
// "likely-only", "extractor-cannot-tell", "no-page" or "not-given".
type Expiry struct {
	Status string `json:"status"`
	Why    string `json:"why,omitempty"`
	Date   string `json:"date,omitempty"`
	Tier   Tier   `json:"tier,omitempty"`
	Page   int    `json:"page,omitempty"`
	Start  int    `json:"start,omitempty"`
	End    int    `json:"end,omitempty"`
	Text   string `json:"text,omitempty"`
}

// Current is whether this is the holder's current passport, with its tier
// and the words to show: a likely value is said as "may be", never as a
// fact.
type Current struct {
	Value *bool  `json:"value"`
	Tier  Tier   `json:"tier,omitempty"`
	Says  string `json:"says"`
}

var passportThings = []string{pack.Passport, pack.Holder}
var passportLinks = []string{pack.PassportOf, pack.HolderOf}

// Ask answers a typed question. referenceDate (YYYY-MM-DD) says which
// expiries are past. A question of any other kind gets NotAnswered and
// nothing is read or guessed.
func Ask(ctx context.Context, e *load.Engine, question, referenceDate string) (Asked, error) {
	if !asksPassportExpiry(question) {
		return Asked{Question: question, Answer: NotAnswered, Message: "This kind of question is not answered yet.", Passports: []Passport{}}, nil
	}
	s, err := read(ctx, e, passportThings, passportLinks)
	if err != nil {
		return Asked{}, err
	}
	return Asked{Question: question, Answer: PassportExpiry, ReferenceDate: referenceDate, Passports: s.passports(referenceDate)}, nil
}

// asksPassportExpiry recognises the one question answered, by its words:
// a passport, and expire, expiry (any word that begins with "expir") or
// "valid until".
func asksPassportExpiry(question string) bool {
	words := wordsOf(question)
	passport := slices.ContainsFunc(words, func(w string) bool { return w == "passport" || w == "passports" })
	expires := slices.ContainsFunc(words, func(w string) bool { return strings.HasPrefix(w, "expir") })
	return passport && (expires || nameMatches(question, []string{"valid", "until"}))
}

func (s *snapshot) passports(referenceDate string) []Passport {
	docOf := map[string]string{}
	for _, r := range s.links[pack.PassportOf] {
		docOf[player(r, "passport")] = document(r, "document")
	}
	type holding struct {
		name string
		tier Tier
	}
	holders := map[string]holding{}
	for _, r := range s.links[pack.HolderOf] {
		if h := s.things[player(r, "holder")]; h != nil {
			holders[player(r, "passport")] = holding{str(h.Attributes, pack.Surname) + ", " + str(h.Attributes, pack.Given), tierOf(r)}
		}
	}
	out := []Passport{}
	for _, id := range sortedKeys(s.things) {
		p := s.things[id]
		if p.TypeID != pack.Passport {
			continue
		}
		h := holders[id]
		out = append(out, Passport{Holder: h.name, HolderTier: h.tier, Document: docOf[id], Expiry: expiry(p.Attributes, referenceDate), Current: current(p.Attributes)})
	}
	slices.SortFunc(out, func(a, b Passport) int { return cmp.Or(cmp.Compare(a.Holder, b.Holder), cmp.Compare(a.Document, b.Document)) })
	return out
}

func expiry(attrs map[string][]any, referenceDate string) Expiry {
	date, t := str(attrs, pack.Expiry), tier(attrs, pack.ExpiryTier)
	page, hasPage := integer(attrs, pack.ExpiryPage)
	switch {
	case date == "":
		return Expiry{Status: "cannot-tell", Why: "not-given"}
	case date == "cannot-tell":
		return Expiry{Status: "cannot-tell", Why: "extractor-cannot-tell", Tier: t}
	case t != Certain:
		return Expiry{Status: "cannot-tell", Why: "likely-only", Tier: t}
	case !hasPage:
		return Expiry{Status: "cannot-tell", Why: "no-page", Tier: t}
	}
	e := Expiry{Status: "in-date", Date: date, Tier: t, Page: page, Text: str(attrs, pack.ExpiryText)}
	e.Start, _ = integer(attrs, pack.ExpiryStart)
	e.End, _ = integer(attrs, pack.ExpiryEnd)
	if date < referenceDate {
		e.Status = "past"
	}
	return e
}

func current(attrs map[string][]any) Current {
	c := Current{Value: boolean(attrs, pack.Current), Tier: tier(attrs, pack.CurrentTier)}
	switch {
	case c.Value == nil:
		c.Tier, c.Says = "", "cannot tell whether this is the current passport"
	case c.Tier == Certain && *c.Value:
		c.Says = "the current passport"
	case c.Tier == Certain:
		c.Says = "not the current passport"
	case *c.Value:
		c.Says = "may be the current passport"
	default:
		c.Says = "may not be the current passport"
	}
	return c
}
