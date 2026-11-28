package show

import (
	"context"
	"fmt"
	"slices"
	"strings"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
)

// Proof is the engine's proof of one fact: the rule that derived it, the
// facts it rests on and what the rule found none of, or the source records
// that state it.
type Proof struct {
	Fact     string     `json:"fact"`
	Status   string     `json:"status"`
	Rule     string     `json:"rule,omitempty"`
	Cut      string     `json:"cut,omitempty"`
	Sources  []string   `json:"sources,omitempty"`
	Claim    *Claim     `json:"claim,omitempty"`
	Because  []Proof    `json:"because,omitempty"`
	RuledOut []RuledOut `json:"ruled_out,omitempty"`
}

// RuledOut is one condition a rule checked and found nothing for, and the
// things it was checked for.
type RuledOut struct {
	Line  string  `json:"line"`
	Claim *Claim  `json:"claim,omitempty"`
	For   []Thing `json:"for"`
}

// Claim is a fact about a link or an attribute's value, or a condition a
// rule checked, as data in the schema's terms: what it tests, the type,
// link and attribute it is about with their labels, the things on the
// link, the value as the page shows that attribute, and the time a rule
// adds to a time it tests. With is what the thing at a link's other end
// has; Of are the parts of a condition of several.
type Claim struct {
	Test      string  `json:"test"`
	Type      string  `json:"type,omitempty"`
	Link      string  `json:"link,omitempty"`
	LinkLabel string  `json:"link_label,omitempty"`
	Ends      []End   `json:"ends,omitempty"`
	Attribute string  `json:"attribute,omitempty"`
	Label     string  `json:"label,omitempty"`
	Value     string  `json:"value,omitempty"`
	Plus      string  `json:"plus,omitempty"`
	With      *Claim  `json:"with,omitempty"`
	Of        []Claim `json:"of,omitempty"`
}

// proofLimits bound the proof of one derived fact.
var proofLimits = interpreter.ProofLimits{MaxDepth: 4, MaxNodes: 64}

func (r *Reader) why(ctx context.Context, fact interpreter.ProofFact) (Proof, error) {
	why, err := r.interp.Why(ctx, fact, proofLimits)
	if err != nil {
		return Proof{}, err
	}
	return r.proof(ctx, why.Proof)
}

func (r *Reader) proof(ctx context.Context, p *interpreter.Proof) (Proof, error) {
	if p == nil {
		return Proof{Status: string(interpreter.ProofUnknown)}, nil
	}
	fact, claim, err := r.fact(ctx, p.Fact)
	if err != nil {
		return Proof{}, err
	}
	out := Proof{Fact: fact, Claim: claim, Status: string(p.Status), Cut: string(p.Cut)}
	if p.Rule != nil {
		out.Rule = p.Rule.ID
	}
	for _, s := range p.Sources {
		out.Sources = append(out.Sources, s.SourceRecordID)
	}
	slices.Sort(out.Sources)
	out.Sources = slices.Compact(out.Sources)
	for _, premise := range p.Premises {
		sub, err := r.proof(ctx, premise)
		if err != nil {
			return out, err
		}
		out.Because = append(out.Because, sub)
	}
	for _, n := range p.Negations {
		ruled, err := r.ruledOut(ctx, n)
		if err != nil {
			return out, err
		}
		out.RuledOut = append(out.RuledOut, ruled)
	}
	return out, nil
}

// named is the thing with id as the page names it.
func (r *Reader) named(ctx context.Context, id string) (Thing, bool, error) {
	e, err := r.interp.GetEntity(ctx, id)
	if err != nil || e == nil {
		return Thing{ID: id, Title: id}, false, err
	}
	return r.thing(e), true, nil
}

// fact is a proof's fact in the words of the schema, each thing by its
// title, and, for a fact about a link or that a value is present, the
// same as data.
func (r *Reader) fact(ctx context.Context, f interpreter.ProofFact) (string, *Claim, error) {
	switch f.Kind {
	case interpreter.ProofLinkFact, interpreter.ProofLinkValueFact:
		return r.linkClaim(ctx, f)
	case interpreter.ProofTypeFact, interpreter.ProofValueFact, interpreter.ProofHasValueFact:
	default:
		return string(f.Kind), nil, nil
	}
	t, _, err := r.named(ctx, f.Thing)
	switch {
	case f.Kind == interpreter.ProofTypeFact:
		return fmt.Sprintf("%s is %s", t.Title, r.label(f.Type)), nil, err
	case f.Kind == interpreter.ProofHasValueFact:
		claim := &Claim{Test: testHas, Attribute: f.Attribute, Label: r.label(f.Attribute)}
		return fmt.Sprintf("%s %s", t.Title, r.saidValue(claim)), claim, err
	case f.Value == nil || r.shown.hidden(f.Attribute):
		return fmt.Sprintf("%s has %s", t.Title, r.label(f.Attribute)), nil, err
	}
	return fmt.Sprintf("%s %s %s", t.Title, r.label(f.Attribute), r.shown.value(f.Attribute, f.Value)), nil, err
}

// linkClaim is a fact about a link: "<link>: <role> <thing>, ...", and,
// for a value of the link, "; <attribute>: <value>" or "; has
// <attribute>".
func (r *Reader) linkClaim(ctx context.Context, f interpreter.ProofFact) (string, *Claim, error) {
	claim := &Claim{Test: testLink, Link: f.Relation, LinkLabel: r.label(f.Relation), Ends: []End{}}
	var ends []string
	for _, p := range f.Players {
		thing, _, err := r.named(ctx, p.Thing)
		if err != nil {
			return "", nil, err
		}
		claim.Ends = append(claim.Ends, End{Role: p.Role, Label: r.label(p.Role), Thing: thing})
		if p.Role != "" {
			thing.Title = r.label(p.Role) + " " + thing.Title
		}
		ends = append(ends, thing.Title)
	}
	text := fmt.Sprintf("%s: %s", claim.LinkLabel, strings.Join(ends, ", "))
	if f.Kind != interpreter.ProofLinkValueFact {
		return text, claim, nil
	}
	claim.Test, claim.Attribute, claim.Label = testHas, f.Attribute, r.label(f.Attribute)
	if f.Value != nil && !r.shown.hidden(f.Attribute) {
		claim.Test, claim.Value = testIs, r.shown.value(f.Attribute, f.Value)
	}
	return text + "; " + r.saidValue(claim), claim, nil
}

// ruledOut is a negation of a proof in one form, built from the schema's
// labels: "and no: <what> (for <things>)".
func (r *Reader) ruledOut(ctx context.Context, n ontology.NegativeCondition) (RuledOut, error) {
	claim := r.condition(n.Condition)
	out := RuledOut{Claim: claim, For: []Thing{}}
	var titles []string
	for _, name := range sortedKeys(n.Bindings) {
		id, ok := n.Bindings[name].(string)
		if !ok {
			continue
		}
		thing, held, err := r.named(ctx, id)
		if err != nil {
			return out, err
		}
		if held && !slices.ContainsFunc(out.For, func(t Thing) bool { return t.ID == id }) {
			out.For = append(out.For, thing)
			titles = append(titles, thing.Title)
		}
	}
	what := "a match"
	if claim != nil {
		what = r.said(claim)
	}
	out.Line = "and no: " + what
	if len(titles) > 0 {
		out.Line += " (for " + strings.Join(titles, ", ") + ")"
	}
	return out, nil
}

// The tests a claim makes.
const (
	testType      = "type"
	testLink      = "link"
	testHas       = "has"
	testIs        = "is"
	testMoreThan  = "more_than"
	testLessThan  = "less_than"
	testAtLeast   = "at_least"
	testAtMost    = "at_most"
	testInThePast = "in_the_past"
	testInFuture  = "in_the_future"
	testAnyOf     = "any_of"
	testAllOf     = "all_of"
)

// conditionTests is the test each kind of rule condition makes. A kind
// not here binds or compares the rule's own variables, which the schema
// gives no label.
var conditionTests = map[ontology.ConditionType]string{
	ontology.CondHasType:                    testType,
	ontology.CondPropertyExists:             testHas,
	ontology.CondPropertyEquals:             testIs,
	ontology.CondPropertyGreaterThan:        testMoreThan,
	ontology.CondPropertyLessThan:           testLessThan,
	ontology.CondPropertyAtLeast:            testAtLeast,
	ontology.CondPropertyAtMost:             testAtMost,
	ontology.CondPropertyBeforeNowOffset:    testInThePast,
	ontology.CondPropertyAfterNowOffset:     testInFuture,
	ontology.CondRelatedTo:                  testLink,
	ontology.CondRelatedToPropertyEquals:    testIs,
	ontology.CondRelatedToPropertyExists:    testHas,
	ontology.CondRelationAttributeEquals:    testIs,
	ontology.CondRelationAttributeExists:    testHas,
	ontology.CondRelationAttributeBeforeNow: testInThePast,
	ontology.CondRelationAttributeAfterNow:  testInFuture,
	ontology.CondOr:                         testAnyOf,
	ontology.CondAnd:                        testAllOf,
}

// condition is a rule condition as a claim, or nil when it tests only the
// rule's own variables.
func (r *Reader) condition(c ontology.RuleCondition) *Claim {
	claim := &Claim{Test: conditionTests[c.Type]}
	switch c.Type {
	case ontology.CondOr, ontology.CondAnd:
		for _, part := range c.Children {
			if sub := r.condition(part); sub != nil {
				claim.Of = append(claim.Of, *sub)
			}
		}
		if len(claim.Of) == 0 {
			return nil
		}
		return claim
	case ontology.CondHasType:
		claim.Type, claim.Label = c.ClassID, r.label(c.ClassID)
		return claim
	case ontology.CondRelatedTo:
		claim.Link, claim.LinkLabel = c.PropertyID, r.label(c.PropertyID)
		return claim
	case ontology.CondRelatedToPropertyEquals, ontology.CondRelatedToPropertyExists:
		claim.Attribute = c.TargetPropID
		return &Claim{Test: testLink, Link: c.PropertyID, LinkLabel: r.label(c.PropertyID), With: r.valued(claim, c.Value)}
	case ontology.CondRelationAttributeEquals, ontology.CondRelationAttributeExists,
		ontology.CondRelationAttributeBeforeNow, ontology.CondRelationAttributeAfterNow:
		claim.Link, claim.LinkLabel, claim.Attribute = c.PropertyID, r.label(c.PropertyID), c.RelationAttributeID
	}
	if claim.Test == "" {
		return nil
	}
	if claim.Attribute == "" {
		claim.Attribute = c.PropertyID
	}
	if c.OffsetSeconds != 0 {
		claim.Plus = duration(c.OffsetSeconds)
	}
	return r.valued(claim, c.Value)
}

// valued gives claim its attribute's label and value as the page shows
// that attribute: none of a hidden one, a masked one masked.
func (r *Reader) valued(claim *Claim, v any) *Claim {
	claim.Label = r.label(claim.Attribute)
	if v != nil && !r.shown.hidden(claim.Attribute) {
		claim.Value = r.shown.value(claim.Attribute, v)
	}
	return claim
}

// testWords are the words a test reads as between an attribute's label and
// what it tests.
var testWords = map[string]string{
	testIs: ": ", testMoreThan: " more than ", testLessThan: " less than ", testAtLeast: " at least ", testAtMost: " at most ",
	testInThePast: " in the past", testInFuture: " in the future",
}

// said is a claim in the words of the schema's labels.
func (r *Reader) said(c *Claim) string {
	switch c.Test {
	case testType:
		return c.Label
	case testAnyOf, testAllOf:
		var parts []string
		for _, part := range c.Of {
			parts = append(parts, r.said(&part))
		}
		return strings.Join(parts, map[string]string{testAnyOf: " or ", testAllOf: " and "}[c.Test])
	}
	switch {
	case c.Link == "":
		return r.saidValue(c)
	case c.With != nil && c.With.Test == testHas:
		return c.LinkLabel + ", one with " + c.With.Label
	case c.With != nil:
		return c.LinkLabel + ", one with " + r.saidValue(c.With)
	case c.Attribute != "":
		return c.LinkLabel + "; " + r.saidValue(c)
	}
	return c.LinkLabel
}

// saidValue is what a claim says of its attribute: "has <attribute>" when
// it tests only that there is a value, else "<attribute> <test> <value>",
// a hidden attribute's value said to be hidden.
func (r *Reader) saidValue(c *Claim) string {
	text := c.Label
	if c.Plus != "" {
		text += " plus " + c.Plus
	}
	switch {
	case c.Test == testHas:
		return "has " + c.Label
	case c.Test == testInThePast || c.Test == testInFuture:
		return text + testWords[c.Test]
	case r.shown.hidden(c.Attribute):
		return text + testWords[c.Test] + "a hidden value"
	}
	return text + testWords[c.Test] + c.Value
}

// duration is a rule's offset in the largest whole unit.
func duration(seconds int64) string {
	for _, u := range []struct {
		size      int64
		one, many string
	}{{86400, "day", "days"}, {3600, "hour", "hours"}, {60, "minute", "minutes"}, {1, "second", "seconds"}} {
		if seconds%u.size == 0 {
			if n := seconds / u.size; n != 1 && n != -1 {
				return fmt.Sprintf("%d %s", n, u.many)
			}
			return fmt.Sprintf("%d %s", seconds/u.size, u.one)
		}
	}
	return ""
}
