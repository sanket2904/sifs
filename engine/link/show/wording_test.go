package show_test

import (
	"testing"
	"time"

	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/schema"
)

// gardenRules are the rules of a made-up community garden, each finding
// nothing of one kind of condition for its one plot, by the label of the
// kind it gives the plot.
var gardenRules = []struct {
	label string
	when  schema.ConditionDef
	line  string
}{
	{"Small", schema.AttrGreaterThan("size", int64(40)), "and no: Size more than 40 (for Plot 7)"},
	{"Not tiny", schema.AttrLessThan("size", int64(5)), "and no: Size less than 5 (for Plot 7)"},
	{"Not huge", schema.AttrAtLeast("size", int64(100)), "and no: Size at least 100 (for Plot 7)"},
	{"Not a strip", schema.AttrAtMost("size", int64(1)), "and no: Size at most 1 (for Plot 7)"},
	{"New", schema.AttrElapsed("planted", 30*24*time.Hour), "and no: Planted plus 30 days in the past (for Plot 7)"},
	{"Settled", schema.AttrWithin("planted", time.Hour), "and no: Planted plus 1 hour in the future (for Plot 7)"},
	{"Kept", schema.EntityIs("", "Abandoned"), "and no: Abandoned (for Plot 7)"},
	{"Local", schema.RelatedAttrEquals("TendedBy", "city", "Atlantis"), "and no: Tended by, one with City: Atlantis (for Plot 7)"},
	{"Unlisted", schema.RelatedAttrExists("TendedBy", "phone"), "and no: Tended by, one with Phone (for Plot 7)"},
	{"Open", schema.Or(schema.AttrEquals("size", int64(0)), schema.AttrExists("closedOn")), "and no: Size: 0 or has Closed on (for Plot 7)"},
	{"Running", schema.And(schema.AttrEquals("size", int64(30)), schema.AttrExists("closedOn")), "and no: Size: 30 and has Closed on (for Plot 7)"},
	{"Own gate", schema.AttrEquals("gateCode", "GATE-0000"), "and no: Gate code: •••••0000 (for Plot 7)"},
	{"Quiet", schema.AttrEquals("notes", "keeps bees"), "and no: Notes: a hidden value (for Plot 7)"},
	{"Not closed", schema.AttrElapsed("closedOn", 0), "and no: Closed on in the past (for Plot 7)"},
}

// garden is the garden on the memory store: one plot of 30 square metres
// planted a day ago, tended by one gardener in Kaohsiung, with a masked
// gate code and a hidden note.
func garden(t *testing.T) *interpreter.Interpreter {
	t.Helper()
	m := schema.NewModule("garden", "1")
	label := schema.Label
	m.Entity("Plot").Label("Plot").
		Attribute("name", "string", label("Name"), schema.Alias()).
		Attribute("size", "int64", label("Size")).
		Attribute("planted", "timestamp", label("Planted")).
		Attribute("closedOn", "timestamp", label("Closed on")).
		Attribute("gateCode", "string", label("Gate code"), schema.Masked()).
		Attribute("notes", "string", label("Notes"), schema.Hidden()).
		Display(schema.Title("name"), schema.Collection(ontology.CollectionPlaces))
	m.Entity("Abandoned").Extends("Plot").Label("Abandoned")
	m.Entity("Gardener").Label("Gardener").
		Attribute("fullName", "string", label("Name"), schema.Alias()).
		Attribute("city", "string", label("City")).
		Attribute("phone", "string", label("Phone")).
		Display(schema.Title("fullName"), schema.Collection(ontology.CollectionPeople))
	m.Relation("TendedBy").Label("Tended by").
		Role("plot", "Plot", label("Plot"), schema.ShownAs("Tends")).
		Role("gardener", "Gardener", label("Gardener"), schema.ShownAs("Tended by"))
	for i, r := range gardenRules {
		name := string(rune('a' + i))
		m.Entity("Is" + name).Extends("Plot").Label(r.label)
		m.Rule("rule-"+name).When(schema.EntityIs("", "Plot"), schema.Not(r.when)).Then(schema.AddType("", "Is"+name))
	}
	box, err := schema.CompileModules(m)
	if err != nil {
		t.Fatal(err)
	}
	return memoryStore(t, box, map[string]interpreter.RecordFacts{"plot-7.pdf": {
		Entities: []ontology.ObservedEntity{
			{ID: "plot:7", TypeID: "garden.Plot", Attributes: map[string]any{"garden.name": "Plot 7", "garden.size": int64(30),
				"garden.planted": time.Now().UTC().Add(-24 * time.Hour), "garden.gateCode": "GATE-4417", "garden.notes": "waters on Sundays"}},
			{ID: "gardener:chen", TypeID: "garden.Gardener", Attributes: map[string]any{"garden.fullName": "Chen Wei", "garden.city": "Kaohsiung"}},
		},
		Relations: []ontology.ObservedRelation{{ID: "tends:7", TypeID: "garden.TendedBy", Players: []ontology.RolePlayer{
			{RoleTypeID: "garden.TendedBy:plot", PlayerID: "plot:7", PlayerKind: ontology.PlayerEntity},
			{RoleTypeID: "garden.TendedBy:gardener", PlayerID: "gardener:chen", PlayerKind: ontology.PlayerEntity}}}},
	}}, "garden-files")
}

// What a rule found nothing of reads in one form for every kind of
// condition a rule can test, built from the schema's labels, a masked
// value masked and a hidden one not given. Mutation: drop the words of a
// test, and its line reads wrong.
func TestRuledOutWording(t *testing.T) {
	r := show.New(garden(t), false)
	page := doc(t, r, "plot:7")
	for _, c := range gardenRules {
		ruled := list(also(t, page, c.label)["ruled_out"])
		if len(ruled) != 1 || ruled[0]["line"] != c.line {
			t.Errorf("%s: ruled out %v, want %q", c.label, ruled, c.line)
		}
	}
	revealed := list(also(t, doc(t, show.New(garden(t), true), "plot:7"), "Own gate")["ruled_out"])
	if len(revealed) != 1 || revealed[0]["line"] != "and no: Gate code: GATE-0000 (for Plot 7)" {
		t.Errorf("revealed: %v", revealed)
	}
}
