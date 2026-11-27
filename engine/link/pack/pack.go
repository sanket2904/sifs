// Package pack is the schema the engine is opened with for a person's files:
// the things and links format 3 (sample/FORMAT.md) states, each claim with
// its tier and reasons. It holds no rules and no derived facts.
package pack

import (
	"fmt"

	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/schema"
)

const module = "files"

// Things.
const (
	Document     = module + ".Document"
	Organisation = module + ".Organisation"
	Identifier   = module + ".Identifier"
	Statement    = module + ".Statement"
	Passport     = module + ".Passport"
	Holder       = module + ".Holder"
)

// Links.
const (
	Mentions         = module + ".Mentions"
	Shows            = module + ".Shows"
	HeldAt           = module + ".HeldAt"
	SameOrg          = module + ".SameOrg"
	StatementOf      = module + ".StatementOf"
	StatementAccount = module + ".StatementAccount"
	IssuedBy         = module + ".IssuedBy"
	PassportOf       = module + ".PassportOf"
	HolderOf         = module + ".HolderOf"
	PassportNumber   = module + ".PassportNumber"
	PassportMRZ      = module + ".PassportMRZ"
)

// Things and Links list every type the pack declares.
var (
	Things = []string{Document, Organisation, Identifier, Statement, Passport, Holder}
	Links  = []string{Mentions, Shows, HeldAt, SameOrg, StatementOf, StatementAccount, IssuedBy, PassportOf, HolderOf, PassportNumber, PassportMRZ}
)

// Attributes, by the name the loader writes them under.
const (
	Key         = module + ".key"
	Name        = module + ".name"
	Printed     = module + ".printed"
	DirectoryID = module + ".directory"
	Kind        = module + ".kind"
	Value       = module + ".value"
	Valid       = module + ".valid"
	Page        = module + ".page"
	Start       = module + ".start"
	End         = module + ".end"
	Text        = module + ".text"
	SettledBy   = module + ".settledBy"
	Tier        = module + ".tier"
	Reasons     = module + ".reasons"
	Via         = module + ".via"
	From        = module + ".from"
	To          = module + ".to"
	PeriodTier  = module + ".periodTier"
	Surname     = module + ".surname"
	Given       = module + ".given"
	Issued      = module + ".issued"
	Expiry      = module + ".expiry"
	ExpiryTier  = module + ".expiryTier"
	ExpiryPage  = module + ".expiryPage"
	ExpiryStart = module + ".expiryStart"
	ExpiryEnd   = module + ".expiryEnd"
	ExpiryText  = module + ".expiryText"
	Current     = module + ".current"
	CurrentTier = module + ".currentTier"
)

// Module declares the files schema. Dates stay the strings format 3 gives,
// because a passport's expiry may be `cannot-tell`.
//
// Names. An organisation is found by its card's name and by every text a
// document prints for it, an identifier by its value, a holder by either
// name. A document, a statement and a passport have no name of their own in
// format 3; they are reached through the things they link to.
func Module() *schema.ModuleDef {
	tier := schema.Annotations(ontology.ValuesAnnotation("certain", "likely"))
	m := schema.NewModule(module, "1")
	label, hidden, masked, names, shownAs := schema.Label, schema.Hidden(), schema.Masked(), schema.Alias(), schema.ShownAs
	one := schema.One()

	m.Entity("Document").Label("Document").
		Display(schema.Collection(ontology.CollectionDocuments))
	m.Entity("Organisation").Label("Organisation").
		Attribute("key", "string", schema.Required(), label("Key"), hidden).
		Attribute("name", "string", label("Name"), names).
		Attribute("printed", "string", schema.Repeated(), label("Printed as"), names).
		Attribute("directory", "string", label("Directory entry"), hidden).
		Display(schema.Title("name", "key"), schema.Subtitle("printed"), schema.Collection(ontology.CollectionOrganisations))
	m.Entity("Identifier").Label("Number or code").
		Attribute("kind", "string", schema.Required(), label("Kind")).
		Attribute("value", "string", schema.Required(), label("Value"), names, masked).
		Display(schema.Title("value"), schema.Subtitle("kind"), schema.Collection(ontology.CollectionAccounts))
	m.Entity("Statement").Label("Statement").
		Attribute("from", "string", label("From")).
		Attribute("to", "string", label("To")).
		Attribute("periodTier", "string", tier, label("Period tier")).
		Display(schema.Title("to"), schema.Subtitle("from"), schema.Collection(ontology.CollectionDocuments))
	m.Entity("Passport").Label("Passport").
		Attribute("issued", "string", label("Issued")).
		Attribute("expiry", "string", label("Expires")).
		Attribute("expiryTier", "string", tier, label("Expiry tier")).
		Attribute("expiryPage", "int64", label("Expiry page"), hidden).
		Attribute("expiryStart", "int64", label("Expiry start"), hidden).
		Attribute("expiryEnd", "int64", label("Expiry end"), hidden).
		Attribute("expiryText", "string", label("Expiry as printed")).
		Attribute("current", "bool", label("Current")).
		Attribute("currentTier", "string", tier, label("Current tier")).
		Display(schema.Title("expiry"), schema.Subtitle("issued"), schema.Collection(ontology.CollectionDocuments))
	m.Entity("Holder").Label("Passport holder").
		Attribute("surname", "string", label("Surname"), names).
		Attribute("given", "string", label("Given names"), names).
		Display(schema.Title("surname"), schema.Subtitle("given"), schema.Collection(ontology.CollectionPeople))

	m.Relation("Mentions").Label("Mentions").
		Role("document", "Document", one, label("Document"), shownAs("Mentions")).
		Role("organisation", "Organisation", one, label("Organisation"), shownAs("Mentioned in")).
		Attribute("page", "int64", schema.Required(), label("Page")).
		Attribute("start", "int64", schema.Required(), label("Start"), hidden).
		Attribute("end", "int64", schema.Required(), label("End"), hidden).
		Attribute("text", "string", schema.Required(), label("Printed text"), masked).
		Attribute("settledBy", "string", label("Settled by"), hidden)
	m.Relation("Shows").Label("Shows").
		Role("document", "Document", one, label("Document"), shownAs("Shows")).
		Role("identifier", "Identifier", one, label("Number or code"), shownAs("Printed in")).
		Attribute("page", "int64", schema.Required()).
		Attribute("start", "int64", schema.Required()).
		Attribute("end", "int64", schema.Required()).
		Attribute("text", "string", schema.Required()).
		Attribute("valid", "bool", label("Check passes")).
		Attribute("tier", "string", schema.Required(), tier, label("Tier"))
	m.Relation("HeldAt").Label("Held at").
		Role("identifier", "Identifier", one, label("Number or code"), shownAs("Held at")).
		Role("organisation", "Organisation", one, label("Organisation"), shownAs("Holds")).
		Role("document", "Document", one, label("Document"), shownAs("Says who holds")).
		Attribute("reasons", "string", schema.Repeated(), label("Reasons")).
		Attribute("via", "string", schema.Repeated(), label("Via")).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("SameOrg").Label("Same organisation").
		Role("one", "Organisation", one, label("One"), shownAs("Same organisation as")).
		Role("other", "Organisation", one, label("Other"), shownAs("Same organisation as")).
		Role("document", "Document", one, label("Document"), shownAs("Says same organisation")).
		Attribute("reasons", "string", schema.Repeated()).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("StatementOf").Label("Statement in").
		Role("statement", "Statement", one, label("Statement"), shownAs("In document")).
		Role("document", "Document", one, label("Document"), shownAs("Statement"))
	m.Relation("StatementAccount").Label("Statement account").
		Role("statement", "Statement", one, label("Statement"), shownAs("Account")).
		Role("account", "Identifier", one, label("Account"), shownAs("Statements")).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("IssuedBy").Label("Issued by").
		Role("statement", "Statement", one, label("Statement"), shownAs("Issued by")).
		Role("issuer", "Organisation", one, label("Issuer"), shownAs("Statements issued")).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("PassportOf").Label("Passport in").
		Role("passport", "Passport", one, label("Passport"), shownAs("In document")).
		Role("document", "Document", one, label("Document"), shownAs("Passport"))
	m.Relation("HolderOf").Label("Holder of").
		Role("holder", "Holder", one, label("Holder"), shownAs("Passport")).
		Role("passport", "Passport", one, label("Passport"), shownAs("Holder")).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("PassportNumber").Label("Passport number").
		Role("passport", "Passport", one, label("Passport"), shownAs("Number")).
		Role("identifier", "Identifier", one, label("Number"), shownAs("Number of passport")).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("PassportMRZ").Label("Machine-readable zone").
		Role("passport", "Passport", one, label("Passport"), shownAs("Machine-readable zone")).
		Role("identifier", "Identifier", one, label("Zone"), shownAs("Zone of passport"))
	return m
}

// Compile returns the schema the engine is opened with.
func Compile() (*ontology.TBox, error) {
	box, err := schema.CompileModules(Module())
	if err != nil {
		return nil, fmt.Errorf("compile the files pack: %w", err)
	}
	return box, nil
}
