package load

import (
	"context"
	"testing"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/pack"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/schema"
)

// mainModule is the files pack as it was before it declared names, labels
// and display hints, copied from pack.Module at efe2d0b, so a test can fill
// a store the way a person's store was filled.
func mainModule() *schema.ModuleDef {
	tier := schema.Annotations(ontology.ValuesAnnotation("certain", "likely"))
	m := schema.NewModule("files", "1")

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

// beforeNames is a document's facts as sifs-load wrote them before the
// pack named anything: as at 6c6f1d6, without the printed names.
func beforeNames(run *extract.Output) func(extract.DocOut) interpreter.RecordFacts {
	return func(doc extract.DocOut) interpreter.RecordFacts {
		facts := asBase(Facts(doc, run.Organisations))
		for _, thing := range facts.Entities {
			delete(thing.Attributes, pack.Printed)
		}
		return facts
	}
}

// A person's store filled before the pack named anything. Opening it with
// the pack finds things by the names the store already holds at once; the
// printed names come with the next load, which rewrites each document whose
// facts were written otherwise (all 41 of the committed run's, as every one
// prints a name or a number), turns none down and leaves the store equal to
// one this loader filled. Mutation: drop the name index rebuild in open,
// and "hdfc bank" finds nothing on opening.
func TestExistingStore(t *testing.T) {
	run := committedRun(t)
	hdfcBank := sortedIDs(OrgID("IFSC HDFC"), OrgID("name:email-fee-payment:hdfcbank"), OrgID("name:hdfc-branch-letter:hdfcbank"), OrgID("name:hdfc-card-statement:hdfcbank"),
		OrgID("name:hdfc-statement-aug:hdfcbank"), OrgID("name:hdfc-statement-jul:hdfcbank"), OrgID("name:hdfc-statement-sep:hdfcbank"), OrgID("name:neft-receipt-tied:hdfcbank"))
	existing(t, mainModule(), run, beforeNames(run), func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		eq(t, "hdfc bank on opening", resolve(t, ctx, e, "HDFC Bank"), hdfcBank)
		eq(t, "kalpit on opening", resolve(t, ctx, e, "kalpit"), kalpit)
		eq(t, "hdfc ltd, printed only, on opening", resolve(t, ctx, e, "HDFC Ltd"), []string(nil))
		eq(t, "first load", mustLoad(t, ctx, e, run), Report{Written: 41, TurnedDown: []TurnDown{}, LeftOut: committedLeftOut})
		eq(t, "hdfc ltd after the load", resolve(t, ctx, e, "HDFC Ltd"), []string{OrgID("name:hdfc-ltd-loan-2019:housingdevelopmentfinancecorporation")})
		eq(t, "the store after the load", held(t, ctx, reopen()), freshly(t, ctx, run))
		eq(t, "second load", mustLoad(t, ctx, reopen(), run), Report{Unchanged: 41, TurnedDown: []TurnDown{}, LeftOut: committedLeftOut})
	})
}
