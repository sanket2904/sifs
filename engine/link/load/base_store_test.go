package load

import (
	"context"
	"maps"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/pack"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/schema"
	"github.com/sanket2904/ontology-framework/store"
	"github.com/sanket2904/ontology-framework/store/sqlite"
)

// baseModule is the files pack as it was before bank codes, dates and
// institution codes, copied from pack.Module at 6c6f1d6, so a test can fill
// a store the way a person's store was filled.
func baseModule() *schema.ModuleDef {
	tier := schema.Annotations(ontology.ValuesAnnotation("certain", "likely"))
	m := schema.NewModule("files", "1")
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

// asBase is a document's facts as sifs-load wrote them at 6c6f1d6: a bank
// code is a number with the id, type, links and printed text of one; an
// organisation has no institution code; a statement has no period; a date
// is the string format 3 gives, and an expiry the run cannot tell is the word `cannot-tell`.
func asBase(facts interpreter.RecordFacts) interpreter.RecordFacts {
	var pairs []string
	for _, e := range facts.Entities {
		if e.TypeID == pack.BankCode {
			pairs = append(pairs, e.ID, IdentifierID(e.Attributes[pack.Kind].(string), e.Attributes[pack.Code].(string)))
		}
	}
	old := strings.NewReplacer(pairs...)
	day := func(attrs map[string]any, attr string) {
		if t, ok := attrs[attr].(time.Time); ok {
			attrs[attr] = t.Format(time.DateOnly)
		}
	}
	for i := range facts.Entities {
		e := &facts.Entities[i]
		switch e.TypeID {
		case pack.BankCode:
			e.ID, e.TypeID = old.Replace(e.ID), pack.Identifier
			e.Attributes = map[string]any{pack.Kind: e.Attributes[pack.Kind], pack.Value: e.Attributes[pack.Code]}
		case pack.Organisation:
			delete(e.Attributes, pack.Institution)
		case pack.Statement:
			delete(e.Attributes, pack.Period)
			day(e.Attributes, pack.From)
			day(e.Attributes, pack.To)
		case pack.Passport:
			day(e.Attributes, pack.Issued)
			day(e.Attributes, pack.Expiry)
			if _, given := e.Attributes[pack.Expiry]; !given && e.Attributes[pack.ExpiryTier] != nil {
				e.Attributes[pack.Expiry] = "cannot-tell"
			}
		}
	}
	types := map[string]struct{ typeID, kind, role string }{
		pack.ShowsCode: {pack.Shows, "shows", "identifier"},
		pack.CodeOf:    {pack.HeldAt, "heldAt", "identifier"},
	}
	for i := range facts.Relations {
		r := &facts.Relations[i]
		r.ID = old.Replace(r.ID)
		for j := range r.Players {
			r.Players[j].PlayerID = old.Replace(r.Players[j].PlayerID)
		}
		if via, ok := r.Attributes[pack.Via].([]any); ok {
			for j, v := range via {
				via[j] = old.Replace(v.(string))
			}
		}
		if text, ok := r.Attributes[pack.PrintedText]; ok {
			r.Attributes[pack.Text] = text
			delete(r.Attributes, pack.PrintedText)
		}
		to, ok := types[r.TypeID]
		if !ok {
			continue
		}
		r.ID = to.kind + r.ID[strings.IndexByte(r.ID, ':'):]
		for j := range r.Players {
			p := &r.Players[j]
			role := p.RoleTypeID[strings.IndexByte(p.RoleTypeID, ':')+1:]
			if role == "code" {
				role = to.role
			}
			p.RoleTypeID = to.typeID + ":" + role
		}
		r.TypeID = to.typeID
	}
	return facts
}

// fillAt fills a store the way sifs-load did under module: that module
// published, and each document's facts as base gives them.
func fillAt(t *testing.T, ctx context.Context, schemas store.SchemaStore, objects store.Store, module *schema.ModuleDef, run *extract.Output, base func(extract.DocOut) interpreter.RecordFacts) {
	t.Helper()
	box, err := schema.CompileModules(module)
	if err != nil {
		t.Fatal(err)
	}
	interp, err := interpreter.New(ctx, schemas, objects, interpreter.WithAnonymousWrites(), interpreter.WithUnverifiedSourceClaims())
	if err != nil {
		t.Fatal(err)
	}
	_, version, err := interp.SchemaSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if err := interp.PublishSchema(ctx, box, version); err != nil {
		t.Fatal(err)
	}
	for _, doc := range run.Documents {
		if _, err := interp.ReplaceRecord(ctx, record(doc.ID), base(doc)); err != nil {
			t.Fatalf("%s: %v", doc.ID, err)
		}
	}
}

// existing runs check on a store filled under module, on the memory store
// and on SQLite; reopen opens the engine on it with the files pack, as
// sifs-load does.
func existing(t *testing.T, module *schema.ModuleDef, run *extract.Output, base func(extract.DocOut) interpreter.RecordFacts, check func(t *testing.T, ctx context.Context, reopen func() *Engine)) {
	ctx := context.Background()
	t.Run("memory", func(t *testing.T) {
		schemas, objects := store.NewMemorySchemaStore(), store.NewMemoryStore()
		fillAt(t, ctx, schemas, objects, module, run, base)
		check(t, ctx, func() *Engine {
			e, err := open(ctx, schemas, objects, func() error { return nil })
			if err != nil {
				t.Fatal(err)
			}
			if err := e.publish(ctx); err != nil {
				t.Fatal(err)
			}
			return e
		})
	})
	t.Run("sqlite", func(t *testing.T) {
		dir := t.TempDir()
		st, err := sqlite.Open(filepath.Join(dir, StoreFile))
		if err != nil {
			t.Fatal(err)
		}
		schemas, err := st.SchemaStore(ctx)
		if err != nil {
			t.Fatal(err)
		}
		fillAt(t, ctx, schemas, st, module, run, base)
		if err := st.Close(); err != nil {
			t.Fatal(err)
		}
		var e *Engine
		t.Cleanup(func() { e.Close() })
		check(t, ctx, func() *Engine {
			if e != nil {
				if err := e.Close(); err != nil {
					t.Fatal(err)
				}
			}
			if e, err = Open(ctx, dir); err != nil {
				t.Fatal(err)
			}
			return e
		})
	})
}

// freshly is what a store this loader fills with run holds.
func freshly(t *testing.T, ctx context.Context, run *extract.Output) map[string]string {
	t.Helper()
	e, err := OpenMemory(ctx)
	if err != nil {
		t.Fatal(err)
	}
	mustLoad(t, ctx, e, run)
	return held(t, ctx, e)
}

// A person's store filled at 6c6f1d6. The next load rewrites every
// document that has a bank code, a date, a passport or an institution
// code's key, turns none down, and leaves the store equal to one this
// loader filled: no thing or link stays under an old id or with an old
// value. A second load changes nothing. Mutations: skip a document the
// store already holds in Load, and old bank code ids stay; write an
// expiry the run cannot tell as the word, and the store differs.
func TestBaseStore(t *testing.T) {
	run := committedRun(t)
	changed := 0
	for _, doc := range run.Documents {
		if !reflect.DeepEqual(Facts(doc, run.Organisations), asBase(Facts(doc, run.Organisations))) {
			changed++
		}
	}
	existing(t, baseModule(), run, func(doc extract.DocOut) interpreter.RecordFacts { return asBase(Facts(doc, run.Organisations)) },
		func(t *testing.T, ctx context.Context, reopen func() *Engine) {
			fresh := freshly(t, ctx, run)
			e := reopen()
			if before := held(t, ctx, e); len(before) == 0 || maps.Equal(before, fresh) {
				t.Fatal("filled at the base, the store is empty or already equals a fresh one")
			}
			eq(t, "first load", mustLoad(t, ctx, e, run), Report{Written: changed, Unchanged: len(run.Documents) - changed, TurnedDown: []TurnDown{}, LeftOut: committedLeftOut})
			eq(t, "the store after the load", held(t, ctx, reopen()), fresh)
			eq(t, "second load", mustLoad(t, ctx, reopen(), run), Report{Unchanged: len(run.Documents), TurnedDown: []TurnDown{}, LeftOut: committedLeftOut})
			t.Logf("%d of %d documents rewritten", changed, len(run.Documents))
		})
}
