package show_test

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/store"
)

// held is every string value a store holds, by whether its attribute is
// declared masked, hidden or neither, and every thing's id.
type held struct {
	masked, hidden, plain, ids []string
}

func holds(t *testing.T, interp *interpreter.Interpreter) held {
	t.Helper()
	ctx := context.Background()
	box := interp.TBox()
	var h held
	add := func(attrs map[string][]any) {
		for attrID, vs := range attrs {
			d := &ontology.AttributeDisplay{}
			if at := box.GetAttributeType(attrID); at != nil && at.Display != nil {
				d = at.Display
			}
			for _, v := range vs {
				s := fmt.Sprint(v)
				switch {
				case d.Mask:
					h.masked = append(h.masked, s)
				case d.Hidden:
					h.hidden = append(h.hidden, s)
				default:
					h.plain = append(h.plain, s)
				}
			}
		}
	}
	things, err := interp.QueryEntities(ctx, store.EntityPattern{})
	if err != nil {
		t.Fatal(err)
	}
	for _, e := range things {
		h.ids = append(h.ids, e.ID)
		add(e.Attributes)
	}
	links, err := interp.QueryRelations(ctx, store.RelationPattern{})
	if err != nil {
		t.Fatal(err)
	}
	for _, l := range links {
		add(l.Attributes)
	}
	return h
}

// secret are the values of vs a reader can keep from a person: those the
// store does not also hold, whole or inside another value, under an
// attribute it shows. The rest are listed in the log.
func secret(t *testing.T, vs []string, shown []string) []string {
	t.Helper()
	var out, also []string
	for _, v := range vs {
		if v == "" {
			continue
		}
		if slices.ContainsFunc(shown, func(s string) bool { return strings.Contains(s, v) }) {
			also = append(also, v)
			continue
		}
		out = append(out, v)
	}
	slices.Sort(out)
	slices.Sort(also)
	out, also = slices.Compact(out), slices.Compact(also)
	t.Logf("%d values kept from a person; %d also held where they are shown: %q", len(out), len(also), also)
	return out
}

// everything is every read a person can make of the store: its kinds, a
// find by each value it holds, and the page of each thing.
func everything(t *testing.T, r *show.Reader, h held) string {
	t.Helper()
	ctx := context.Background()
	var b strings.Builder
	kinds, err := r.Kinds(ctx)
	if err != nil {
		t.Fatal(err)
	}
	b.Write(encode(t, kinds))
	for _, words := range slices.Concat(h.masked, h.hidden, h.plain) {
		found, err := r.Find(ctx, words)
		if err != nil {
			t.Fatal(err)
		}
		b.Write(encode(t, found))
	}
	for _, id := range h.ids {
		page, err := r.Page(ctx, id)
		if err != nil {
			t.Fatal(err)
		}
		b.Write(encode(t, page))
	}
	return b.String()
}

// Mutation: give a title's first value raw in shown.first, and account
// numbers are given to a person.
func TestMaskedAndHidden(t *testing.T) {
	for _, s := range []struct {
		name string
		open func(*testing.T) *interpreter.Interpreter
		raw  string
	}{
		{"files", filesOnMemory, "DE89370400440532013000"},
		{"shop", shopOnMemory, anaTax},
	} {
		t.Run(s.name, func(t *testing.T) {
			interp := s.open(t)
			h := holds(t, interp)
			masked := secret(t, h.masked, slices.Concat(h.plain, h.hidden))
			hidden := secret(t, h.hidden, slices.Concat(h.plain, h.masked, h.ids))
			if !slices.Contains(masked, s.raw) {
				t.Fatalf("%s is not among the masked values", s.raw)
			}
			out := everything(t, show.New(interp, false), h)
			for _, v := range slices.Concat(masked, hidden) {
				if strings.Contains(out, v) {
					t.Errorf("%q is given to a person", v)
				}
			}
			revealed := everything(t, show.New(interp, true), h)
			if !strings.Contains(revealed, s.raw) {
				t.Errorf("revealed, %q is not given", s.raw)
			}
			for _, v := range hidden {
				if strings.Contains(revealed, v) {
					t.Errorf("revealed, hidden %q is given", v)
				}
			}
		})
	}
}
