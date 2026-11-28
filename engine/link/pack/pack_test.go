package pack_test

import (
	"context"
	"reflect"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/link/pack"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/store"
)

// published is an engine with the files pack published, as sifs-load opens
// it.
func published(t *testing.T) (context.Context, *interpreter.Interpreter) {
	t.Helper()
	ctx := context.Background()
	interp, err := interpreter.New(ctx, store.NewMemorySchemaStore(), store.NewMemoryStore())
	if err != nil {
		t.Fatal(err)
	}
	box, err := pack.Compile()
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
	return ctx, interp
}

func eq[T any](t *testing.T, what string, got, want T) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		t.Errorf("%s:\n got %v\nwant %v", what, got, want)
	}
}

// How each kind of thing is shown and found. A document has no attribute
// in format 3, so nothing can title it; a statement is titled by its
// period.
var things = map[string]struct {
	collection      ontology.Collection
	title, subtitle []string
	names           []string
}{
	pack.Document:     {collection: ontology.CollectionDocuments},
	pack.Organisation: {ontology.CollectionOrganisations, []string{pack.Name, pack.Printed, pack.Institution}, []string{pack.Printed}, []string{pack.Name, pack.Printed, pack.Institution}},
	pack.Identifier:   {ontology.CollectionAccounts, []string{pack.Value}, []string{pack.Kind}, []string{pack.Value}},
	pack.BankCode:     {ontology.CollectionOrganisations, []string{pack.Code}, []string{pack.Kind}, []string{pack.Code}},
	pack.Statement:    {ontology.CollectionDocuments, []string{pack.Period}, nil, nil},
	pack.Passport:     {ontology.CollectionDocuments, []string{pack.ExpiryText}, nil, nil},
	pack.Holder:       {ontology.CollectionPeople, []string{pack.Surname}, []string{pack.Given}, []string{pack.Surname, pack.Given}},
}

var (
	masked = []string{pack.NumberText, pack.Value}
	hidden = []string{pack.DirectoryID, pack.End, pack.ExpiryEnd, pack.ExpiryPage, pack.ExpiryStart, pack.Key, pack.SettledBy, pack.Start}
)

// The type cards the engine gives for each kind of thing carry the pack's
// label, title, subtitle, collection and names, a label for every property
// and a "shown as" for every link. Mutation: drop Alias from the holder's
// given names, and Holder's names lose it.
func TestThingTypes(t *testing.T) {
	ctx, interp := published(t)
	eq(t, "thing types", len(things), len(pack.Things))
	for _, typeID := range pack.Things {
		want, ok := things[typeID]
		if !ok {
			t.Errorf("%s: no expectation", typeID)
			continue
		}
		card, err := interp.DescribeObjectType(ctx, typeID)
		if err != nil {
			t.Fatal(err)
		}
		if card.Label == "" {
			t.Errorf("%s has no label", typeID)
		}
		if card.Display == nil {
			t.Errorf("%s has no display", typeID)
			continue
		}
		eq(t, typeID+" collection", card.Display.Collection, want.collection)
		eq(t, typeID+" title", card.Display.Title, want.title)
		eq(t, typeID+" subtitle", card.Display.Subtitle, want.subtitle)
		var names []string
		for _, p := range card.Properties {
			if p.Label == "" {
				t.Errorf("%s property %s has no label", typeID, p.AttributeTypeID)
			}
			if p.Alias {
				names = append(names, p.AttributeTypeID)
			}
		}
		eq(t, typeID+" names", names, want.names)
		for _, l := range card.Links {
			if l.Display == nil || l.Display.Label == "" {
				t.Errorf("%s link %s has no shown as", typeID, l.RoleTypeID)
			}
		}
	}
}

// Every link type, role and attribute of the pack has a label, every role
// a "shown as", and exactly the sensitive values are masked and the
// internal ones hidden. Mutation: drop Masked from the identifier's value,
// and the masked set loses it.
func TestSchemaLabelsAndMasks(t *testing.T) {
	ctx, interp := published(t)
	box, _, err := interp.SchemaSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	for _, typeID := range pack.Links {
		rel := box.GetRelationType(typeID)
		if rel == nil {
			t.Fatalf("no link type %s", typeID)
		}
		if rel.Label == "" {
			t.Errorf("link type %s has no label", typeID)
		}
		for _, r := range rel.Relates {
			role := box.GetRoleType(r.RoleTypeID)
			if role.Label == "" {
				t.Errorf("role %s has no label", role.ID)
			}
			if role.Display == nil || role.Display.Label == "" {
				t.Errorf("role %s has no shown as", role.ID)
			}
		}
	}
	var gotMasked, gotHidden []string
	for id, at := range box.AttributeTypes {
		if !strings.HasPrefix(id, "files.") {
			continue
		}
		if at.Label == "" {
			t.Errorf("attribute %s has no label", id)
		}
		if at.Display != nil && at.Display.Mask {
			gotMasked = append(gotMasked, id)
		}
		if at.Display != nil && at.Display.Hidden {
			gotHidden = append(gotHidden, id)
		}
	}
	slices.Sort(gotMasked)
	slices.Sort(gotHidden)
	eq(t, "masked", gotMasked, masked)
	eq(t, "hidden", gotHidden, hidden)
}

// A statement's period and a passport's dates are the engine's timestamps,
// shown as dates, so a rule can compare them. Mutation: declare the
// statement's end as a string, and it is not a date.
func TestDates(t *testing.T) {
	ctx, interp := published(t)
	box, _, err := interp.SchemaSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	for _, id := range []string{pack.From, pack.To, pack.Issued, pack.Expiry} {
		at := box.GetAttributeType(id)
		if at.ValueType != ontology.ValueDateTime || at.Display == nil || at.Display.Kind != ontology.DisplayDate {
			t.Errorf("%s is a %v shown as %+v, want a timestamp shown as a date", id, at.ValueType, at.Display)
		}
	}
}

// Exactly the kinds that name a bank or a branch are public, and a kind
// the list does not know is a person's own. Mutation: make Public true for
// every kind, and the made-up kind is public.
func TestPublicKinds(t *testing.T) {
	for kind, want := range map[string]bool{
		"bic": true, "ifsc": true, "gb_sort_code": true, "fr_bank_code": true, "fr_branch_code": true,
		"iban": false, "account": false, "card": false, "mrz": false, "passport_number": false, "in_pan": false, "fr_rib_key": false,
		"xx_member_number": false,
	} {
		eq(t, kind+" is public", pack.Public(kind), want)
	}
}
