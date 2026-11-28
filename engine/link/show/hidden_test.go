package show_test

import (
	"encoding/json"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/schema"
)

// harbour is a made-up harbour on the memory store whose rules rest on
// hidden attributes in every way a why can show one: a boat's keeper has a
// value, equals a constant, is not there, is not a constant, and stands in
// an any-of; a mooring's permit equals a constant or not; a berth's warden
// is not a constant. A keeper's country is shown, and its label shares a
// word with the keeper's.
func harbour(t *testing.T) *interpreter.Interpreter {
	t.Helper()
	m := schema.NewModule("harbour", "1")
	label := schema.Label
	m.Entity("Boat").Label("Boat").
		Attribute("name", "string", label("Name"), schema.Alias()).
		Attribute("length", "int64", label("Length")).
		Attribute("keeper", "string", label("Keeper"), schema.Hidden()).
		Attribute("keeperCountry", "string", label("Keeper's country")).
		Display(schema.Title("name"), schema.Collection(ontology.CollectionThings))
	m.Entity("Berth").Label("Berth").
		Attribute("name", "string", label("Name"), schema.Alias()).
		Attribute("warden", "string", label("Warden"), schema.Hidden()).
		Display(schema.Title("name"), schema.Collection(ontology.CollectionPlaces))
	m.Relation("MooredAt").Label("Moored at").
		Role("boat", "Boat", label("Boat"), schema.ShownAs("Moored at")).
		Role("berth", "Berth", label("Berth"), schema.ShownAs("Boats")).
		Attribute("fee", "int64", label("Fee")).
		Attribute("permit", "string", label("Permit"), schema.Hidden())
	m.Relation("ClearedAt").Label("Cleared at").
		Role("boat", "Boat", label("Boat"), schema.ShownAs("Cleared at")).
		Role("berth", "Berth", label("Berth"), schema.ShownAs("Cleared boats"))
	kind := func(name, kindLabel string, when ...schema.ConditionDef) {
		m.Entity(name).Extends("Boat").Label(kindLabel)
		m.Rule(strings.ToLower(name)).When(append([]schema.ConditionDef{schema.EntityIs("", "Boat")}, when...)...).Then(schema.AddType("", name))
	}
	kind("Kept", "Kept", schema.AttrExists("keeper"))
	kind("KnownKeeper", "Known keeper", schema.AttrEquals("keeper", "Ingrid Solberg"))
	kind("Unkept", "Unkept", schema.Not(schema.AttrExists("keeper")))
	kind("NotOlus", "Not Olu's", schema.Not(schema.AttrEquals("keeper", "Olu Bankole")))
	kind("NoBlankPermit", "No blank permit", schema.Not(schema.RelationAttrEquals("MooredAt", "permit", "PRM-0000")))
	kind("Unwatched", "Not watched by Rivera", schema.Not(schema.RelatedAttrEquals("MooredAt", "warden", "Tomás Rivera")))
	kind("Unmeasured", "Unmeasured", schema.Not(schema.Or(schema.AttrEquals("length", int64(0)), schema.AttrExists("keeper"))))
	kind("FromAbroad", "From abroad", schema.AttrExists("keeperCountry"))
	kind("NoCountry", "No country", schema.Not(schema.AttrExists("keeperCountry")))
	m.Rule("cleared").When(schema.HasTypeVar("Boat", "b"),
		schema.RelationAttrEqualsVars("MooredAt", "permit", "b", "q", "PRM-2207"),
		schema.RelationAttrExistsVars("MooredAt", "fee", "b", "q")).
		Then(schema.AddRelationVars("ClearedAt", "b", "q"))
	box, err := schema.CompileModules(m)
	if err != nil {
		t.Fatal(err)
	}
	return memoryStore(t, box, map[string]interpreter.RecordFacts{
		"mooring-quay-4.pdf": {
			Entities: []ontology.ObservedEntity{
				{ID: "boat:sjostjerna", TypeID: "harbour.Boat", Attributes: map[string]any{"harbour.name": "Sjøstjerna", "harbour.length": int64(12),
					"harbour.keeper": "Ingrid Solberg", "harbour.keeperCountry": "Norway"}},
				{ID: "berth:quay4", TypeID: "harbour.Berth", Attributes: map[string]any{"harbour.name": "Mombasa Quay 4", "harbour.warden": "Tomás Rivera"}},
			},
			Relations: []ontology.ObservedRelation{{ID: "moored:sjostjerna", TypeID: "harbour.MooredAt",
				Attributes: map[string]any{"harbour.fee": int64(120), "harbour.permit": "PRM-2207"},
				Players: []ontology.RolePlayer{
					{RoleTypeID: "harbour.MooredAt:boat", PlayerID: "boat:sjostjerna", PlayerKind: ontology.PlayerEntity},
					{RoleTypeID: "harbour.MooredAt:berth", PlayerID: "berth:quay4", PlayerKind: ontology.PlayerEntity}}}},
		},
		"registry-santos.pdf": {
			Entities: []ontology.ObservedEntity{{ID: "boat:estrela", TypeID: "harbour.Boat", Attributes: map[string]any{"harbour.name": "Estrela do Mar", "harbour.length": int64(9)}}},
		},
	}, "harbour-files")
}

// proofLines are the fact and ruled-out lines of every proof in a read, and
// the data beside them as JSON.
func proofLines(read []byte) (lines []string, data []string) {
	var doc any
	if json.Unmarshal(read, &doc) != nil {
		return nil, nil
	}
	var walk func(v any, inProof bool)
	walk = func(v any, inProof bool) {
		switch t := v.(type) {
		case []any:
			for _, sub := range t {
				walk(sub, inProof)
			}
		case map[string]any:
			for k, sub := range t {
				switch {
				case inProof && (k == "fact" || k == "line"):
					lines = append(lines, text(sub))
				case inProof && (k == "claim" || k == "for"):
					b, _ := json.Marshal(sub)
					data = append(data, string(b))
				default:
					walk(sub, inProof || k == "proof")
				}
			}
		}
	}
	walk(doc, false)
	return lines, data
}

// A why that rests on a hidden attribute says that it rests on a value that
// is not shown, and never names the attribute: no line of any read, with
// or without reveal, holds its label, no data its id or label, and nothing
// its value. An attribute that is shown keeps its label, whatever words it
// shares with a hidden one. Mutation: name the hidden attribute's label
// again in a line, and it is found.
func TestHiddenIsNotNamed(t *testing.T) {
	interp := harbour(t)
	h := holds(t, interp)
	want := []string{
		"Sjøstjerna has " + wordNotShown,
		"Moored at: Boat Sjøstjerna, Berth Mombasa Quay 4; has " + wordNotShown,
		"Moored at: Boat Sjøstjerna, Berth Mombasa Quay 4; Fee: 120",
		"Sjøstjerna has Keeper's country",
		"and no: " + wordNotShown + " (for Estrela do Mar)",
		"and no: Moored at; " + wordNotShown + " (for Sjøstjerna)",
		"and no: Moored at, one with " + wordNotShown + " (for Estrela do Mar)",
		"and no: Length: 0 or " + wordNotShown + " (for Estrela do Mar)",
		"and no: has Keeper's country (for Estrela do Mar)",
	}
	hidden := map[string]string{"harbour.keeper": "Keeper", "harbour.permit": "Permit", "harbour.warden": "Warden"}
	secrets := []string{"Ingrid Solberg", "PRM-2207", "Tomás Rivera", "Olu Bankole", "PRM-0000"}
	for _, reveal := range []bool{false, true} {
		var lines []string
		for _, read := range everything(t, show.New(interp, reveal), h) {
			for _, s := range secrets {
				if strings.Contains(string(read), s) {
					t.Errorf("reveal %v: a read gives hidden %q", reveal, s)
				}
			}
			got, data := proofLines(read)
			lines = append(lines, got...)
			for _, d := range data {
				for id, label := range hidden {
					if strings.Contains(d, `"`+id+`"`) || strings.Contains(d, `"label":"`+label+`"`) {
						t.Errorf("reveal %v: proof data names hidden %s: %s", reveal, id, d)
					}
				}
			}
		}
		for _, line := range lines {
			for _, label := range hidden {
				if holdsWhole(strings.ReplaceAll(line, "Keeper's country", ""), label) {
					t.Errorf("reveal %v: %q names a hidden attribute", reveal, line)
				}
			}
		}
		for _, w := range want {
			if !slices.Contains(lines, w) {
				t.Errorf("reveal %v: no line %q among\n%s", reveal, w, strings.Join(lines, "\n"))
			}
		}
	}
}
