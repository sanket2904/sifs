package load

import (
	"context"
	"fmt"
	"reflect"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/link/pack"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/store"
)

// An organisation's id and a number's id hold neither its key nor its
// value. Each is the same on every load and every machine (the two written
// down here are the first 16 hex characters of the SHA-256 of "BIC
// COBADEFF" and of "iban:NL91ABNA0417164300"), and each of the committed
// run's keys and numbers has an id of its own. Mutations: put the key back
// in OrgID, or the value back in IdentifierID, and the stored ids hold
// them; count calls in digest, and the ids differ from the ones written
// down here.
func TestIDsCarryNoValue(t *testing.T) {
	eq(t, "a German bank's id", OrgID("BIC COBADEFF"), "org:9e577a3773d0d204")
	eq(t, "a Dutch IBAN's id", IdentifierID("iban", "NL91ABNA0417164300"), "identifier:2288ef2e20318bbd")
	run := committedRun(t)
	keys, numbers := map[string]bool{}, map[[2]string]bool{}
	for _, doc := range run.Documents {
		for _, m := range doc.Orgs {
			if m.Org != nil {
				keys[*m.Org] = true
			}
		}
		for _, l := range doc.Links {
			keys[l.Org] = true
		}
		for _, s := range doc.SameOrg {
			keys[s.Keys[0]], keys[s.Keys[1]] = true, true
		}
		if s := doc.Statement; s != nil && s.Issuer != "" {
			keys[s.Issuer] = true
		}
		for _, ident := range doc.Identifiers {
			numbers[[2]string{ident.Kind, ident.Value}] = true
		}
	}
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		mustLoad(t, ctx, e, run)
		orgs, err := e.Things(ctx, pack.Organisation)
		if err != nil {
			t.Fatal(err)
		}
		idents, err := e.Things(ctx, pack.Identifier)
		if err != nil {
			t.Fatal(err)
		}
		eq(t, "organisations, one per key", len(orgs), len(keys))
		eq(t, "numbers, one per kind and value", len(idents), len(numbers))
		for _, o := range orgs {
			key := o.Attributes[pack.Key][0].(string)
			if o.ID != OrgID(key) || strings.Contains(o.ID, key) {
				t.Errorf("the organisation with key %q has the id %s", key, o.ID)
			}
		}
		for _, n := range idents {
			kind, value := n.Attributes[pack.Kind][0].(string), n.Attributes[pack.Value][0].(string)
			if n.ID != IdentifierID(kind, value) || strings.Contains(n.ID, value) {
				t.Errorf("the %s %q has the id %s", kind, value, n.ID)
			}
		}
	})
}

// asBefore is a document's facts with the ids sifs-load gave before an id
// held no value: "org:" and the key, "identifier:", the kind, ":" and the
// value, in the things, in the links that name them and in a link's via.
func asBefore(facts interpreter.RecordFacts) interpreter.RecordFacts {
	var pairs []string
	for _, e := range facts.Entities {
		switch e.TypeID {
		case pack.Organisation:
			pairs = append(pairs, e.ID, "org:"+e.Attributes[pack.Key].(string))
		case pack.Identifier:
			pairs = append(pairs, e.ID, "identifier:"+e.Attributes[pack.Kind].(string)+":"+e.Attributes[pack.Value].(string))
		}
	}
	old := strings.NewReplacer(pairs...)
	for i := range facts.Entities {
		facts.Entities[i].ID = old.Replace(facts.Entities[i].ID)
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
	}
	return facts
}

// held is everything a store holds of the files pack, by id: each thing's
// type and values, each link's type, ends and values, each repeated value
// as a set.
func held(t *testing.T, ctx context.Context, e *Engine) map[string]string {
	t.Helper()
	out := map[string]string{}
	values := func(attrs map[string][]any) string {
		var parts []string
		for id, vs := range attrs {
			var s []string
			for _, v := range vs {
				s = append(s, fmt.Sprint(v))
			}
			slices.Sort(s)
			parts = append(parts, id+"="+strings.Join(s, "|"))
		}
		slices.Sort(parts)
		return strings.Join(parts, " ")
	}
	things, err := e.interp.QueryEntities(ctx, store.EntityPattern{})
	if err != nil {
		t.Fatal(err)
	}
	for _, thing := range things {
		out[thing.ID] = thing.TypeID + " " + values(thing.Attributes)
	}
	links, err := e.interp.QueryRelations(ctx, store.RelationPattern{})
	if err != nil {
		t.Fatal(err)
	}
	for _, l := range links {
		var ends []string
		for _, p := range l.Players {
			ends = append(ends, p.RoleTypeID+"="+p.PlayerID)
		}
		slices.Sort(ends)
		out[l.ID] = l.TypeID + " " + strings.Join(ends, " ") + " " + values(l.Attributes)
	}
	return out
}

// A person's store filled when an id held its key or value. The next load
// rewrites every document that names an organisation or a number, turns
// none down, and leaves the store equal to one this loader filled: no
// thing or link stays under an old id. A second load changes nothing.
func TestStoreWithValueIDs(t *testing.T) {
	run := committedRun(t)
	fresh := func(t *testing.T, ctx context.Context) map[string]string {
		e, err := OpenMemory(ctx)
		if err != nil {
			t.Fatal(err)
		}
		mustLoad(t, ctx, e, run)
		return held(t, ctx, e)
	}
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		changed := 0
		for _, doc := range run.Documents {
			facts := Facts(doc, run.Organisations)
			before := asBefore(Facts(doc, run.Organisations))
			if !reflect.DeepEqual(facts, before) {
				changed++
			}
			if _, err := e.interp.ReplaceRecord(ctx, record(doc.ID), before); err != nil {
				t.Fatal(err)
			}
		}
		if reflect.DeepEqual(held(t, ctx, e), fresh(t, ctx)) {
			t.Fatal("filled as before, the store already equals a fresh one")
		}
		eq(t, "first load", mustLoad(t, ctx, reopen(), run), Report{Written: changed, Unchanged: len(run.Documents) - changed, TurnedDown: []TurnDown{}, LeftOut: committedLeftOut})
		eq(t, "the store after the load", held(t, ctx, reopen()), fresh(t, ctx))
		eq(t, "second load", mustLoad(t, ctx, reopen(), run), Report{Unchanged: len(run.Documents), TurnedDown: []TurnDown{}, LeftOut: committedLeftOut})
		t.Logf("%d of %d documents rewritten", changed, len(run.Documents))
	})
}
