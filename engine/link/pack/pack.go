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
func Module() *schema.ModuleDef {
	tier := schema.Annotations(ontology.ValuesAnnotation("certain", "likely"))
	m := schema.NewModule(module, "1")

	m.Entity("Document")
	m.Entity("Organisation").
		Attribute("key", "string", schema.Required()).
		Attribute("name", "string").
		Attribute("directory", "string")
	m.Entity("Identifier").
		Attribute("kind", "string", schema.Required()).
		Attribute("value", "string", schema.Required())
	m.Entity("Statement").
		Attribute("from", "string").
		Attribute("to", "string").
		Attribute("periodTier", "string", tier)
	m.Entity("Passport").
		Attribute("issued", "string").
		Attribute("expiry", "string").
		Attribute("expiryTier", "string", tier).
		Attribute("expiryPage", "int64").
		Attribute("expiryStart", "int64").
		Attribute("expiryEnd", "int64").
		Attribute("expiryText", "string").
		Attribute("current", "bool").
		Attribute("currentTier", "string", tier)
	m.Entity("Holder").
		Attribute("surname", "string").
		Attribute("given", "string")

	m.Relation("Mentions").
		Role("document", "Document", schema.One()).
		Role("organisation", "Organisation", schema.One()).
		Attribute("page", "int64", schema.Required()).
		Attribute("start", "int64", schema.Required()).
		Attribute("end", "int64", schema.Required()).
		Attribute("text", "string", schema.Required()).
		Attribute("settledBy", "string")
	m.Relation("Shows").
		Role("document", "Document", schema.One()).
		Role("identifier", "Identifier", schema.One()).
		Attribute("page", "int64", schema.Required()).
		Attribute("start", "int64", schema.Required()).
		Attribute("end", "int64", schema.Required()).
		Attribute("text", "string", schema.Required()).
		Attribute("valid", "bool").
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("HeldAt").
		Role("identifier", "Identifier", schema.One()).
		Role("organisation", "Organisation", schema.One()).
		Role("document", "Document", schema.One()).
		Attribute("reasons", "string", schema.Repeated()).
		Attribute("via", "string", schema.Repeated()).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("SameOrg").
		Role("one", "Organisation", schema.One()).
		Role("other", "Organisation", schema.One()).
		Role("document", "Document", schema.One()).
		Attribute("reasons", "string", schema.Repeated()).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("StatementOf").
		Role("statement", "Statement", schema.One()).
		Role("document", "Document", schema.One())
	m.Relation("StatementAccount").
		Role("statement", "Statement", schema.One()).
		Role("account", "Identifier", schema.One()).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("IssuedBy").
		Role("statement", "Statement", schema.One()).
		Role("issuer", "Organisation", schema.One()).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("PassportOf").
		Role("passport", "Passport", schema.One()).
		Role("document", "Document", schema.One())
	m.Relation("HolderOf").
		Role("holder", "Holder", schema.One()).
		Role("passport", "Passport", schema.One()).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("PassportNumber").
		Role("passport", "Passport", schema.One()).
		Role("identifier", "Identifier", schema.One()).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("PassportMRZ").
		Role("passport", "Passport", schema.One()).
		Role("identifier", "Identifier", schema.One())
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
