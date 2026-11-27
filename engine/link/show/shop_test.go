package show_test

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/schema"
	"github.com/sanket2904/ontology-framework/store/sqlite"
)

// shopSchema is a small bakery's schema, made up and unrelated to the
// files schema: customers, invoices billed to them, and the products on
// each invoice with a quantity. A customer's tax number is masked and a
// note about them is hidden; an invoice of 1000 or more is also a large
// invoice, by a rule.
func shopSchema(t *testing.T) *ontology.TBox {
	t.Helper()
	m := schema.NewModule("bakery", "1")
	label, names := schema.Label, schema.Alias()
	m.Entity("Customer").Label("Customer").
		Attribute("fullName", "string", label("Name"), names).
		Attribute("city", "string", label("City")).
		Attribute("taxNumber", "string", label("Tax number"), names, schema.Masked()).
		Attribute("note", "string", label("Note"), schema.Hidden()).
		Display(schema.Title("fullName"), schema.Subtitle("city"), schema.Collection(ontology.CollectionPeople))
	m.Entity("Invoice").Label("Invoice").
		Attribute("number", "string", label("Number"), names).
		Attribute("issued", "string", label("Issued")).
		Attribute("total", "int64", label("Total")).
		Display(schema.Title("number"), schema.Subtitle("issued"), schema.Collection(ontology.CollectionDocuments))
	m.Entity("LargeInvoice").Extends("Invoice").Label("Large invoice")
	m.Entity("Product").Label("Product").
		Attribute("title", "string", label("Name"), names).
		Attribute("sku", "string", label("Stock code")).
		Display(schema.Title("title"), schema.Subtitle("sku"), schema.Collection(ontology.CollectionThings))
	m.Relation("BilledTo").Label("Billed to").
		Role("invoice", "Invoice", schema.One(), label("Invoice"), schema.ShownAs("Billed to")).
		Role("customer", "Customer", schema.One(), label("Customer"), schema.ShownAs("Invoices"))
	m.Relation("LineOf").Label("Line of").
		Role("product", "Product", schema.One(), label("Product"), schema.ShownAs("On invoices")).
		Role("invoice", "Invoice", schema.One(), label("Invoice"), schema.ShownAs("Lines")).
		Attribute("quantity", "int64", label("Quantity"))
	m.Rule("large-invoice").
		When(schema.EntityIs("", "Invoice"), schema.AttrAtLeast("total", int64(1000))).
		Then(schema.AddType("", "LargeInvoice"))
	box, err := schema.CompileModules(m)
	if err != nil {
		t.Fatal(err)
	}
	return box
}

const (
	anaTax   = "PT509876543210"
	kwameTax = "GH0044718829"
)

// shopRecords are two made-up invoices, each one source record.
func shopRecords() map[string]interpreter.RecordFacts {
	ana := ontology.ObservedEntity{ID: "customer:ana", TypeID: "bakery.Customer", Attributes: map[string]any{
		"bakery.fullName": "Ana Ferreira", "bakery.city": "Lisbon", "bakery.taxNumber": anaTax, "bakery.note": "prefers rye"}}
	kwame := ontology.ObservedEntity{ID: "customer:kwame", TypeID: "bakery.Customer", Attributes: map[string]any{
		"bakery.fullName": "Kwame Mensah", "bakery.city": "Accra", "bakery.taxNumber": kwameTax}}
	loaf := ontology.ObservedEntity{ID: "product:loaf", TypeID: "bakery.Product", Attributes: map[string]any{
		"bakery.title": "Sourdough loaf", "bakery.sku": "SD-01"}}
	tart := ontology.ObservedEntity{ID: "product:tart", TypeID: "bakery.Product", Attributes: map[string]any{
		"bakery.title": "Kwarktaart", "bakery.sku": "KT-12"}}
	invoice := func(id, number, issued string, total int64) ontology.ObservedEntity {
		return ontology.ObservedEntity{ID: id, TypeID: "bakery.Invoice", Attributes: map[string]any{
			"bakery.number": number, "bakery.issued": issued, "bakery.total": total}}
	}
	link := func(id, typeID string, attrs map[string]any, players ...[2]string) ontology.ObservedRelation {
		r := ontology.ObservedRelation{ID: id, TypeID: typeID, Attributes: attrs}
		for _, p := range players {
			r.Players = append(r.Players, ontology.RolePlayer{RoleTypeID: typeID + ":" + p[0], PlayerID: p[1], PlayerKind: ontology.PlayerEntity})
		}
		return r
	}
	return map[string]interpreter.RecordFacts{
		"invoice-0041.pdf": {
			Entities: []ontology.ObservedEntity{ana, loaf, tart, invoice("invoice:0041", "INV-0041", "2026-10-02", 1250)},
			Relations: []ontology.ObservedRelation{
				link("billed:0041", "bakery.BilledTo", nil, [2]string{"invoice", "invoice:0041"}, [2]string{"customer", "customer:ana"}),
				link("line:0041:loaf", "bakery.LineOf", map[string]any{"bakery.quantity": int64(40)}, [2]string{"product", "product:loaf"}, [2]string{"invoice", "invoice:0041"}),
				link("line:0041:tart", "bakery.LineOf", map[string]any{"bakery.quantity": int64(25)}, [2]string{"product", "product:tart"}, [2]string{"invoice", "invoice:0041"}),
			},
		},
		"invoice-0042.pdf": {
			Entities: []ontology.ObservedEntity{kwame, loaf, invoice("invoice:0042", "INV-0042", "2026-10-09", 85)},
			Relations: []ontology.ObservedRelation{
				link("billed:0042", "bakery.BilledTo", nil, [2]string{"invoice", "invoice:0042"}, [2]string{"customer", "customer:kwame"}),
				link("line:0042:loaf", "bakery.LineOf", map[string]any{"bakery.quantity": int64(3)}, [2]string{"product", "product:loaf"}, [2]string{"invoice", "invoice:0042"}),
			},
		},
	}
}

func shopOnMemory(t *testing.T) *interpreter.Interpreter {
	return memoryStore(t, shopSchema(t), shopRecords(), "bakery-files")
}

// shopOnSQLite is the same shop written into a SQLite store, then opened
// again to read.
func shopOnSQLite(t *testing.T) *interpreter.Interpreter {
	t.Helper()
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "shop.db")
	st, err := sqlite.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	schemas, err := st.SchemaStore(ctx)
	if err != nil {
		t.Fatal(err)
	}
	writer, err := interpreter.New(ctx, schemas, st, interpreter.WithAnonymousWrites(), interpreter.WithUnverifiedSourceClaims())
	if err != nil {
		t.Fatal(err)
	}
	writeRecords(t, writer, shopSchema(t), shopRecords(), "bakery-files")
	st.Close()
	interp, closer, err := show.Open(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closer() })
	return interp
}

// shopReads are the three reads on the shop, as one document.
func shopReads(t *testing.T, r *show.Reader) []byte {
	t.Helper()
	ctx := context.Background()
	out := map[string]any{}
	kinds, err := r.Kinds(ctx)
	if err != nil {
		t.Fatal(err)
	}
	out["kinds"] = kinds
	for name, words := range map[string]string{"a name": "Ana", "a tax number": anaTax, "invoice numbers": "INV-004", "a word of two kinds": "kwa", "nothing": "croissant"} {
		found, err := r.Find(ctx, words)
		if err != nil {
			t.Fatal(err)
		}
		out["find "+name] = found
	}
	for _, id := range []string{"customer:ana", "invoice:0041", "invoice:0042", "product:loaf"} {
		page, err := r.Page(ctx, id)
		if err != nil {
			t.Fatal(err)
		}
		out["page "+id] = page
	}
	return encode(t, out)
}

// The same reads give the shop's kinds, finds and pages from its schema
// alone. Mutation: ignore a declared label, and the shop differs from the
// golden.
func TestSecondSchema(t *testing.T) {
	for name, open := range map[string]func(*testing.T) *interpreter.Interpreter{"memory": shopOnMemory, "sqlite": shopOnSQLite} {
		t.Run(name, func(t *testing.T) {
			got := shopReads(t, show.New(open(t), false))
			golden(t, "shop.json", got)
			if again := shopReads(t, show.New(open(t), false)); string(again) != string(got) {
				t.Fatal("the same shop read twice gave different bytes")
			}
		})
	}
}
