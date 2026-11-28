package pack

import "github.com/sanket2904/ontology-framework/schema"

// Derived links: the loader writes none of them; each comes from a rule
// in rules. A derived fact reads as no more certain than what it rests on.
const (
	LatestStatement         = module + ".LatestStatement"
	ProbablyLatestStatement = module + ".ProbablyLatestStatement"
	LaterStatement          = module + ".LaterStatement"
	EndNotKnown             = module + ".EndNotKnown"
	SameEnd                 = module + ".SameEnd"
)

// Derived lists every derived link the pack declares.
var Derived = []string{LatestStatement, ProbablyLatestStatement, LaterStatement, EndNotKnown, SameEnd}

// rules declares the facts the files say without saying them outright.
//
// The latest statement of an account is a statement of it that has an end
// and no statement of it that ends later. It is a link between the
// statement and the account, since a statement may cover several accounts.
// The rule is written in two steps: first "a later statement exists" for a
// statement and an account, then the latest as a statement without one. A
// statement of the account whose end cannot be read may be the later one,
// so then no statement of it is called the latest. Two statements that end
// the same day are both the latest, and each says so.
func rules(m *schema.ModuleDef) {
	one, label, shownAs := schema.One(), schema.Label, schema.ShownAs
	pair := func(name, linkLabel, fromStatement, fromAccount string) {
		m.Relation(name).Label(linkLabel).
			Role("statement", "Statement", one, label("Statement"), shownAs(fromStatement)).
			Role("account", "Identifier", one, label("Account"), shownAs(fromAccount))
	}
	pair("LaterStatement", "A later statement exists", "Not the latest statement of", "Older statements")
	m.Rule("later-statement").When(
		schema.RelatedVars("StatementAccount", "s", "a"),
		schema.RelatedVars("StatementAccount", "later", "a"),
		schema.BindAttr("to", "s", "end"),
		schema.BindAttr("to", "later", "laterEnd"),
		schema.VarsCompare("laterEnd", ">", "end"),
	).Then(schema.AddRelationVars("LaterStatement", "s", "a"))

	pair("EndNotKnown", "Another statement's end is not known", "Cannot tell if it is the latest statement of", "Cannot tell if it is the latest statement")
	m.Rule("end-not-known").When(
		schema.RelatedVars("StatementAccount", "s", "a"),
		schema.RelatedVars("StatementAccount", "other", "a"),
		schema.VarsNotEqual("s", "other"),
		schema.Not(schema.BindAttr("to", "other", "otherEnd")),
	).Then(schema.AddRelationVars("EndNotKnown", "s", "a"))

	pair("SameEnd", "Ends the same day as another statement of this account", "Ends the same day as another statement of", "Statements that end the same day")
	m.Rule("same-end").When(
		schema.RelatedVars("StatementAccount", "s", "a"),
		schema.RelatedVars("StatementAccount", "other", "a"),
		schema.VarsNotEqual("s", "other"),
		schema.BindAttr("to", "s", "end"),
		schema.BindAttr("to", "other", "otherEnd"),
		schema.VarsCompare("end", "==", "otherEnd"),
	).Then(schema.AddRelationVars("SameEnd", "s", "a"))

	pair("LatestStatement", "Latest statement", "Latest statement of", "Latest statement")
	pair("ProbablyLatestStatement", "Probably the latest statement", "Probably the latest statement of", "Probably the latest statement")
	latest := func(name, linkType string, sure ...schema.ConditionDef) {
		m.Rule(name).When(append(append([]schema.ConditionDef{schema.RelatedVars("StatementAccount", "s", "a")}, sure...),
			schema.AttrExists("to"),
			schema.NotRelatedVars("LaterStatement", "s", "a"),
			schema.NotRelatedVars("EndNotKnown", "s", "a"),
		)...).Then(schema.AddRelationVars(linkType, "s", "a"))
	}
	latest("latest-statement", "LatestStatement",
		schema.AttrEqualsVar("periodTier", "s", "certain"),
		schema.RelationAttrEqualsVars("StatementAccount", "tier", "s", "a", "certain"))
	latest("probably-latest-statement", "ProbablyLatestStatement", schema.Or(
		schema.AttrEqualsVar("periodTier", "s", "likely"),
		schema.RelationAttrEqualsVars("StatementAccount", "tier", "s", "a", "likely")))
}
