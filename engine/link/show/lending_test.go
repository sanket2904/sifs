package show_test

import (
	"context"
	"encoding/json"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/schema"
	"github.com/sanket2904/ontology-framework/store/sqlite"
)

// lendingSchema is a small lending library's schema, made up and unrelated
// to the files schema, with rules: members registered at branches, loans
// lent to a member, and reminders sent about a loan. A member's card
// number and the receipt for their fees are masked; a note about them and
// who checked their registration are hidden.
//   - A returned loan no reminder was sent about is free of reminders.
//   - A member registered at a branch in good standing, checked by
//     someone, who paid the fees there with a receipt may borrow from it.
//   - A member with a card number is a card holder; one with none needs a
//     card.
//   - A member whose note says they help on Saturdays is a volunteer.
//   - A member with no suspended registration is in good standing.
func lendingSchema(t *testing.T) *ontology.TBox {
	t.Helper()
	m := schema.NewModule("lending", "1")
	label, names := schema.Label, schema.Alias()
	m.Entity("Member").Label("Member").
		Attribute("fullName", "string", label("Name"), names).
		Attribute("city", "string", label("City")).
		Attribute("cardNumber", "string", label("Card number"), schema.Masked()).
		Attribute("note", "string", label("Note"), schema.Hidden()).
		Display(schema.Title("fullName"), schema.Subtitle("city"), schema.Collection(ontology.CollectionPeople))
	m.Entity("CardHolder").Extends("Member").Label("Card holder")
	m.Entity("Volunteer").Extends("Member").Label("Volunteer")
	m.Entity("NeedsCard").Extends("Member").Label("Needs a card")
	m.Entity("InGoodStanding").Extends("Member").Label("In good standing")
	m.Entity("Branch").Label("Branch").
		Attribute("name", "string", label("Name"), names).
		Attribute("city", "string", label("City")).
		Display(schema.Title("name"), schema.Subtitle("city"), schema.Collection(ontology.CollectionPlaces))
	m.Entity("Loan").Label("Loan").
		Attribute("number", "string", label("Number"), names).
		Attribute("due", "string", label("Due")).
		Attribute("returned", "bool", label("Returned")).
		Display(schema.Title("number"), schema.Subtitle("due"), schema.Collection(ontology.CollectionDocuments))
	m.Entity("ReminderFreeLoan").Extends("Loan").Label("Free of reminders")
	m.Entity("Reminder").Label("Reminder").
		Attribute("number", "string", label("Number"), names).
		Display(schema.Title("number"), schema.Collection(ontology.CollectionDocuments))
	m.Relation("LentTo").Label("Lent to").
		Role("loan", "Loan", schema.One(), label("Loan"), schema.ShownAs("Lent to")).
		Role("member", "Member", label("Member"), schema.ShownAs("Loans"))
	m.Relation("ReminderFor").Label("A reminder was sent").
		Role("reminder", "Reminder", schema.One(), label("Reminder"), schema.ShownAs("About")).
		Role("loan", "Loan", label("Loan"), schema.ShownAs("Reminders"))
	m.Relation("RegisteredAt").Label("Registered at").
		Role("member", "Member", label("Member"), schema.ShownAs("Registered at")).
		Role("branch", "Branch", label("Branch"), schema.ShownAs("Members")).
		Attribute("standing", "string", label("Standing")).
		Attribute("checkedBy", "string", label("Checked by"), schema.Hidden())
	m.Relation("PaidFeesAt").Label("Paid fees at").
		Role("member", "Member", label("Member"), schema.ShownAs("Paid fees at")).
		Role("branch", "Branch", label("Branch"), schema.ShownAs("Fees paid by")).
		Attribute("receipt", "string", label("Receipt"), schema.Masked())
	m.Relation("MayBorrowFrom").Label("May borrow from").
		Role("member", "Member", label("Member"), schema.ShownAs("May borrow from")).
		Role("branch", "Branch", label("Branch"), schema.ShownAs("Lends to"))
	m.Rule("reminder-free").Label("Free of reminders").
		When(schema.EntityIs("", "Loan"), schema.AttrEquals("returned", true), schema.Not(schema.Related("", "ReminderFor", "loan"))).
		Then(schema.AddType("", "ReminderFreeLoan"))
	m.Rule("may-borrow").Label("May borrow").
		When(schema.HasTypeVar("Member", "m"), schema.RelationAttrEqualsVars("RegisteredAt", "standing", "m", "b", "good"),
			schema.RelationAttrExistsVars("RegisteredAt", "checkedBy", "m", "b"), schema.RelationAttrExistsVars("PaidFeesAt", "receipt", "m", "b")).
		Then(schema.AddRelationVars("MayBorrowFrom", "m", "b"))
	m.Rule("card-holder").Label("Card holder").
		When(schema.EntityIs("", "Member"), schema.AttrExists("cardNumber")).
		Then(schema.AddType("", "CardHolder"))
	m.Rule("needs-card").Label("Needs a card").
		When(schema.EntityIs("", "Member"), schema.Not(schema.AttrExists("cardNumber"))).
		Then(schema.AddType("", "NeedsCard"))
	m.Rule("good-standing").Label("In good standing").
		When(schema.EntityIs("", "Member"), schema.Not(schema.RelationAttrEquals("RegisteredAt", "standing", "suspended"))).
		Then(schema.AddType("", "InGoodStanding"))
	m.Rule("volunteer").Label("Volunteer").
		When(schema.EntityIs("", "Member"), schema.AttrEquals("note", aikoNote)).
		Then(schema.AddType("", "Volunteer"))
	box, err := schema.CompileModules(m)
	if err != nil {
		t.Fatal(err)
	}
	return box
}

const (
	aikoCard    = "JP-OSK-4471-2209"
	aikoReceipt = "RCPT-OSK-88213"
	aikoChecker = "Kenji Watanabe"
	aikoNote    = "helps on Saturdays"
)

// lendingRecords are made-up library documents, each one source record.
func lendingRecords() map[string]interpreter.RecordFacts {
	entity := func(id, typeID string, attrs map[string]any) ontology.ObservedEntity {
		return ontology.ObservedEntity{ID: id, TypeID: "lending." + typeID, Attributes: attrs}
	}
	link := func(id, typeID string, attrs map[string]any, players ...[2]string) ontology.ObservedRelation {
		r := ontology.ObservedRelation{ID: id, TypeID: "lending." + typeID, Attributes: attrs}
		for _, p := range players {
			r.Players = append(r.Players, ontology.RolePlayer{RoleTypeID: r.TypeID + ":" + p[0], PlayerID: p[1], PlayerKind: ontology.PlayerEntity})
		}
		return r
	}
	aiko := entity("member:aiko", "Member", map[string]any{
		"lending.fullName": "Aiko Tanaka", "lending.city": "Osaka", "lending.cardNumber": aikoCard, "lending.note": aikoNote})
	lucas := entity("member:lucas", "Member", map[string]any{"lending.fullName": "Lucas Moreau", "lending.city": "Lyon"})
	osaka := entity("branch:osaka", "Branch", map[string]any{"lending.name": "Osaka Central", "lending.city": "Osaka"})
	lyon := entity("branch:lyon", "Branch", map[string]any{"lending.name": "Lyon Part-Dieu", "lending.city": "Lyon"})
	loan := func(id, number, due string, returned bool) ontology.ObservedEntity {
		return entity(id, "Loan", map[string]any{"lending.number": number, "lending.due": due, "lending.returned": returned})
	}
	return map[string]interpreter.RecordFacts{
		"registration-osaka.pdf": {
			Entities: []ontology.ObservedEntity{aiko, osaka},
			Relations: []ontology.ObservedRelation{
				link("reg:aiko:osaka", "RegisteredAt", map[string]any{"lending.standing": "good", "lending.checkedBy": aikoChecker}, [2]string{"member", "member:aiko"}, [2]string{"branch", "branch:osaka"}),
			},
		},
		"fees-osaka-2026.pdf": {
			Entities: []ontology.ObservedEntity{aiko, osaka},
			Relations: []ontology.ObservedRelation{
				link("fees:aiko:osaka", "PaidFeesAt", map[string]any{"lending.receipt": aikoReceipt}, [2]string{"member", "member:aiko"}, [2]string{"branch", "branch:osaka"}),
			},
		},
		"registration-lyon.pdf": {
			Entities: []ontology.ObservedEntity{lucas, lyon},
			Relations: []ontology.ObservedRelation{
				link("reg:lucas:lyon", "RegisteredAt", map[string]any{"lending.standing": "suspended", "lending.checkedBy": "Ingrid Larsen"}, [2]string{"member", "member:lucas"}, [2]string{"branch", "branch:lyon"}),
			},
		},
		"loan-slip-0107.pdf": {
			Entities: []ontology.ObservedEntity{aiko, loan("loan:0107", "L-0107", "2026-10-20", true)},
			Relations: []ontology.ObservedRelation{
				link("lent:0107", "LentTo", nil, [2]string{"loan", "loan:0107"}, [2]string{"member", "member:aiko"}),
			},
		},
		"loan-slip-0212.pdf": {
			Entities: []ontology.ObservedEntity{lucas, loan("loan:0212", "L-0212", "2026-11-03", true)},
			Relations: []ontology.ObservedRelation{
				link("lent:0212", "LentTo", nil, [2]string{"loan", "loan:0212"}, [2]string{"member", "member:lucas"}),
			},
		},
		"reminder-0212.pdf": {
			Entities: []ontology.ObservedEntity{entity("reminder:0212", "Reminder", map[string]any{"lending.number": "R-0212"})},
			Relations: []ontology.ObservedRelation{
				link("rem:0212", "ReminderFor", nil, [2]string{"reminder", "reminder:0212"}, [2]string{"loan", "loan:0212"}),
			},
		},
	}
}

func lendingOnMemory(t *testing.T) *interpreter.Interpreter {
	return memoryStore(t, lendingSchema(t), lendingRecords(), "lending-files")
}

func lendingOnSQLite(t *testing.T) *interpreter.Interpreter {
	t.Helper()
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "lending.db")
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
	writeRecords(t, writer, lendingSchema(t), lendingRecords(), "lending-files")
	st.Close()
	interp, closer, err := show.Open(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closer() })
	return interp
}

func lendingStores() map[string]func(*testing.T) *interpreter.Interpreter {
	return map[string]func(*testing.T) *interpreter.Interpreter{"memory": lendingOnMemory, "sqlite": lendingOnSQLite}
}

// doc is a page as the JSON a person is given.
func doc(t *testing.T, r *show.Reader, id string) map[string]any {
	t.Helper()
	p, err := r.Page(context.Background(), id)
	if err != nil {
		t.Fatal(err)
	}
	var out map[string]any
	if err := json.Unmarshal(encode(t, p), &out); err != nil {
		t.Fatal(err)
	}
	return out
}

func list(v any) []map[string]any {
	var out []map[string]any
	vs, _ := v.([]any)
	for _, v := range vs {
		if m, ok := v.(map[string]any); ok {
			out = append(out, m)
		}
	}
	return out
}

// row is the row of the page's group labelled group whose other end is
// titled title.
func row(t *testing.T, page map[string]any, group, title string) map[string]any {
	t.Helper()
	for _, g := range list(page["links"]) {
		if g["label"] != group {
			continue
		}
		for _, r := range list(g["rows"]) {
			for _, e := range list(r["ends"]) {
				if thing, _ := e["thing"].(map[string]any); thing["title"] == title {
					return r
				}
			}
		}
	}
	t.Fatalf("%s: no row %q to %q", page["id"], group, title)
	return nil
}

// also is the proof of the type labelled label the page has by a rule.
func also(t *testing.T, page map[string]any, label string) map[string]any {
	t.Helper()
	for _, a := range list(page["also"]) {
		if a["label"] == label {
			proof, _ := a["proof"].(map[string]any)
			return proof
		}
	}
	t.Fatalf("%s: not also %q", page["id"], label)
	return nil
}

// premises are a proof's premises, each as its fact and the documents it
// came from.
func premises(proof map[string]any) []string {
	var out []string
	for _, p := range list(proof["because"]) {
		var from []string
		for _, s := range p["sources"].([]any) {
			from = append(from, s.(string))
		}
		out = append(out, p["fact"].(string)+" | "+strings.Join(from, ", "))
	}
	return out
}

// A link a rule derived, which no document states, is shown with the
// proof the engine gives, on the pages of both its ends: the rule, and
// each premise in the words of the schema with its things by their titles
// and the documents it came from. A link a document states is shown as
// before, with its documents and no proof.
func TestDerivedLinkSaysWhy(t *testing.T) {
	want := []string{
		"Aiko Tanaka is Member | fees-osaka-2026.pdf, loan-slip-0107.pdf, registration-osaka.pdf",
		"Registered at: Member Aiko Tanaka, Branch Osaka Central; Standing: good | registration-osaka.pdf",
		"Registered at: Member Aiko Tanaka, Branch Osaka Central; has Checked by | registration-osaka.pdf",
		"Paid fees at: Member Aiko Tanaka, Branch Osaka Central; Receipt: ••••••••••8213 | fees-osaka-2026.pdf",
	}
	for name, open := range lendingStores() {
		t.Run(name, func(t *testing.T) {
			r := show.New(open(t), false)
			aiko, osaka := doc(t, r, "member:aiko"), doc(t, r, "branch:osaka")
			for _, derived := range []map[string]any{row(t, aiko, "May borrow from", "Osaka Central"), row(t, osaka, "Lends to", "Aiko Tanaka")} {
				proof, _ := derived["proof"].(map[string]any)
				if proof == nil {
					t.Fatalf("a derived link row has no proof: %v", derived)
				}
				if proof["fact"] != "May borrow from: Member Aiko Tanaka, Branch Osaka Central" || proof["status"] != "derived" || proof["rule"] != "lending.may-borrow" {
					t.Errorf("proof %q, %q, %q", proof["fact"], proof["status"], proof["rule"])
				}
				if got := premises(proof); !slices.Equal(got, want) {
					t.Errorf("premises\n%s\nwant\n%s", strings.Join(got, "\n"), strings.Join(want, "\n"))
				}
			}
			stated := row(t, aiko, "Registered at", "Osaka Central")
			if _, ok := stated["proof"]; ok || !slices.Equal(stated["sources"].([]any), []any{"registration-osaka.pdf"}) {
				t.Errorf("a stated row: %v", stated)
			}
		})
	}
}

// A proof says what its rule found nothing of, in the schema's labels
// with the things it looked for by their titles, as text and as data: a
// link, a link with a value, and an attribute with a value.
func TestProofShowsWhatItRuledOut(t *testing.T) {
	for name, open := range lendingStores() {
		t.Run(name, func(t *testing.T) {
			r := show.New(open(t), false)
			for _, c := range []struct{ id, also, line string }{
				{"member:aiko", "In good standing", "and no: Registered at; Standing: suspended (for Aiko Tanaka)"},
				{"member:lucas", "Needs a card", "and no: has Card number (for Lucas Moreau)"},
			} {
				ruled := list(also(t, doc(t, r, c.id), c.also)["ruled_out"])
				if len(ruled) != 1 || ruled[0]["line"] != c.line {
					t.Errorf("%s %s: ruled out %v, want %q", c.id, c.also, ruled, c.line)
				}
			}
			proof := also(t, doc(t, r, "loan:0107"), "Free of reminders")
			ruled := list(proof["ruled_out"])
			if len(ruled) != 1 {
				t.Fatalf("ruled out %v", proof["ruled_out"])
			}
			if ruled[0]["line"] != "and no: A reminder was sent (for L-0107)" {
				t.Errorf("text %q", ruled[0]["line"])
			}
			claim, _ := ruled[0]["claim"].(map[string]any)
			things := list(ruled[0]["for"])
			if claim["test"] != "link" || claim["link"] != "lending.ReminderFor" || claim["link_label"] != "A reminder was sent" || len(things) != 1 || things[0]["id"] != "loan:0107" || things[0]["title"] != "L-0107" {
				t.Errorf("data %v for %v", claim, things)
			}
			if got := list(doc(t, r, "loan:0212")["also"]); len(got) != 0 {
				t.Errorf("a loan with a reminder: also %v", got)
			}
		})
	}
}

// A premise that tests a link's attribute names the attribute and its
// value; one that asks only that an attribute has a value says "has" and
// its label; no page prints Go's nil.
func TestValuePremisesRead(t *testing.T) {
	for name, open := range lendingStores() {
		t.Run(name, func(t *testing.T) {
			r := show.New(open(t), false)
			aiko := doc(t, r, "member:aiko")
			holder := also(t, aiko, "Card holder")
			if got := premises(holder); len(got) != 2 || !strings.HasPrefix(got[1], "Aiko Tanaka has Card number | ") {
				t.Errorf("has a value: %q", got)
			}
			has, _ := list(holder["because"])[1]["claim"].(map[string]any)
			if has["test"] != "has" || has["attribute"] != "lending.cardNumber" || has["label"] != "Card number" || has["value"] != nil {
				t.Errorf("has a value, as data: %v", has)
			}
			proof, _ := row(t, aiko, "May borrow from", "Osaka Central")["proof"].(map[string]any)
			var standing map[string]any
			for _, p := range list(proof["because"]) {
				if c, _ := p["claim"].(map[string]any); c["attribute"] == "lending.standing" {
					standing = c
				}
			}
			if standing["test"] != "is" || standing["link"] != "lending.RegisteredAt" || standing["label"] != "Standing" || standing["value"] != "good" || len(list(standing["ends"])) != 2 {
				t.Errorf("a link's value, as data: %v", standing)
			}
			for _, id := range []string{"member:aiko", "member:lucas", "branch:osaka", "branch:lyon", "loan:0107", "loan:0212", "reminder:0212"} {
				if b := encode(t, doc(t, r, id)); strings.Contains(string(b), "nil>") {
					t.Errorf("%s prints nil", id)
				}
			}
		})
	}
}

// lendingPages are the pages of every thing in the lending library, as one
// document.
func lendingPages(t *testing.T, r *show.Reader) []byte {
	t.Helper()
	out := map[string]show.Page{}
	for _, id := range []string{"member:aiko", "member:lucas", "branch:osaka", "branch:lyon", "loan:0107", "loan:0212", "reminder:0212"} {
		p, err := r.Page(context.Background(), id)
		if err != nil {
			t.Fatal(err)
		}
		out[id] = p
	}
	return encode(t, out)
}

// Every page of the lending library, with and without reveal, is the same
// on both stores and as its golden.
func TestLendingGoldens(t *testing.T) {
	for name, open := range lendingStores() {
		t.Run(name, func(t *testing.T) {
			interp := open(t)
			golden(t, "lending.json", lendingPages(t, show.New(interp, false)))
			golden(t, "lending-revealed.json", lendingPages(t, show.New(interp, true)))
		})
	}
}

// A proof gives a value only as the page gives that attribute: a masked
// value masked unless revealed, in the text and the data of a premise; a
// hidden value never, while the premise still says the attribute has one.
// Every read of the library is searched field by field as for the other
// schemas.
func TestProofKeepsMaskedAndHidden(t *testing.T) {
	for name, open := range lendingStores() {
		t.Run(name, func(t *testing.T) {
			interp := open(t)
			h := holds(t, interp)
			for _, v := range []string{aikoCard, aikoReceipt} {
				if !slices.Contains(h.masked, any(v)) {
					t.Fatalf("%s is not among the masked values", v)
				}
			}
			for _, v := range []string{aikoChecker, aikoNote} {
				if !slices.Contains(h.hidden, any(v)) {
					t.Fatalf("%s is not among the hidden values", v)
				}
			}
			for _, reveal := range []bool{false, true} {
				r := show.New(interp, reveal)
				reportLeaks(t, h, everything(t, r, h), reveal)
				receipt := "••••••••••8213"
				if reveal {
					receipt = aikoReceipt
				}
				proof, _ := row(t, doc(t, r, "member:aiko"), "May borrow from", "Osaka Central")["proof"].(map[string]any)
				var fees map[string]any
				for _, p := range list(proof["because"]) {
					if c, _ := p["claim"].(map[string]any); c["link"] == "lending.PaidFeesAt" {
						fees = p
					}
				}
				if claim, _ := fees["claim"].(map[string]any); fees["fact"] != "Paid fees at: Member Aiko Tanaka, Branch Osaka Central; Receipt: "+receipt || claim["value"] != receipt {
					t.Errorf("reveal %v: the receipt premise %v", reveal, fees)
				}
				for _, page := range []string{"member:aiko", "branch:osaka"} {
					b := string(encode(t, doc(t, r, page)))
					if !strings.Contains(b, "; has Checked by") || page == "member:aiko" && !strings.Contains(b, "Aiko Tanaka has Note") {
						t.Errorf("reveal %v: %s does not say a hidden attribute has a value", reveal, page)
					}
					for _, v := range []string{aikoChecker, aikoNote} {
						if strings.Contains(b, v) {
							t.Errorf("reveal %v: %s gives hidden %q", reveal, page, v)
						}
					}
				}
			}
		})
	}
}
