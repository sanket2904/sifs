package show_test

import (
	"context"
	"testing"

	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/interpreter"
)

// The id the reader gives for a thing without reveal opens that thing's
// page, and no two things are given one id: for every thing the store
// holds, and every thing a find by any value it holds returns, on both
// schemas and both stores. Mutation: mask the id the reader gives, and the
// ids it gives open no page.
func TestIDsOpenTheirPage(t *testing.T) {
	ctx := context.Background()
	for _, s := range []struct {
		name string
		open func(*testing.T) *interpreter.Interpreter
	}{
		{"files on memory", filesOnMemory},
		{"files on sqlite", func(t *testing.T) *interpreter.Interpreter { return openSQLite(t, filesOnSQLite(t)) }},
		{"shop on memory", shopOnMemory},
		{"shop on sqlite", shopOnSQLite},
	} {
		t.Run(s.name, func(t *testing.T) {
			interp := s.open(t)
			h := holds(t, interp)
			r, revealed := show.New(interp, false), show.New(interp, true)
			given := map[string]string{}
			for _, id := range sortedIDs(h.things) {
				page, err := r.Page(ctx, id)
				if err != nil {
					t.Fatal(err)
				}
				if other, ok := given[page.ID]; ok {
					t.Errorf("%s and %s are both given the id %s", other, id, page.ID)
				}
				given[page.ID] = id
				want, err := revealed.Page(ctx, id)
				if err != nil {
					t.Fatal(err)
				}
				if got, err := revealed.Page(ctx, page.ID); err != nil || got.Thing != want.Thing {
					t.Errorf("the id given for %s, %s, opens %+v, %v", id, page.ID, got.Thing, err)
				}
			}
			found := 0
			for _, words := range h.all {
				res, err := r.Find(ctx, words)
				if err != nil {
					t.Fatal(err)
				}
				for _, g := range res.Groups {
					for _, m := range g.Things {
						found++
						if _, ok := given[m.ID]; !ok {
							t.Errorf("find %q gives %s, an id no page gives", words, m.ID)
						}
						if page, err := r.Page(ctx, m.ID); err != nil || page.Thing != m.Thing {
							t.Errorf("find %q gives %+v, whose id opens %+v, %v", words, m.Thing, page.Thing, err)
						}
					}
				}
			}
			if found == 0 {
				t.Fatal("no find found anything")
			}
			t.Logf("%d things, %d things found by a find", len(h.things), found)
		})
	}
}
