package load

import (
	"context"
	"slices"
	"testing"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/pack"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/schema"
)

// packAtMain is the files pack as sifs-load published it before the pack
// had rules: this pack without its rules and its derived links.
func packAtMain() *schema.ModuleDef {
	m := pack.Module()
	m.Rules = nil
	m.Relations = slices.DeleteFunc(m.Relations, func(r schema.RelationDef) bool { return slices.Contains(pack.Derived, "files."+r.Name) })
	return m
}

// The committed run gives the same derived rows whether it is loaded into
// an empty store, into one filled before the pack had rules, or into one
// this loader filled already; a second load leaves every document
// unchanged and the derived rows as they were.
// Mutation: drop the step at the end of a load that works the rules out,
// and the store filled before the rules has none of their rows.
func TestDerivedFromEveryStart(t *testing.T) {
	run := committedRun(t)
	ctx := context.Background()
	empty, err := OpenMemory(ctx)
	if err != nil {
		t.Fatal(err)
	}
	mustLoad(t, ctx, empty, run)
	want := derived(t, ctx, empty)
	if len(want) != 15 {
		t.Fatalf("an empty store loaded: %d derived rows, want 13 latest and 2 later: %v", len(want), want)
	}
	unchanged := Report{Unchanged: len(run.Documents), TurnedDown: []TurnDown{}, LeftOut: committedLeftOut}
	t.Run("filled before the rules", func(t *testing.T) {
		existing(t, packAtMain(), run, func(doc extract.DocOut) interpreter.RecordFacts { return Facts(doc, run.Organisations) }, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
			eq(t, "first load", mustLoad(t, ctx, reopen(), run), unchanged)
			eq(t, "derived after the first load", derived(t, ctx, reopen()), want)
			eq(t, "second load", mustLoad(t, ctx, reopen(), run), unchanged)
			eq(t, "derived after the second load", derived(t, ctx, reopen()), want)
		})
	})
	t.Run("filled by this loader", func(t *testing.T) {
		stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
			loaded(t, ctx, reopen(), run)
			eq(t, "derived after the first load", derived(t, ctx, reopen()), want)
			eq(t, "second load", mustLoad(t, ctx, reopen(), run), unchanged)
			eq(t, "derived after the second load", derived(t, ctx, reopen()), want)
		})
	})
}
