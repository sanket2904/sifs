package load

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"reflect"
	"regexp"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/link/pack"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/store"
)

// No thing's id holds a key or a value. An organisation's, a number's and
// a bank code's id is its prefix and the first 16 hex characters of the
// SHA-256 of its key, of kind:value or of kind:code, and nothing else (the
// three written down here are those of "BIC COBADEFF",
// "iban:NL91ABNA0417164300" and "bic:COBADEFFXXX"), the same on every load
// and every machine, and each of the committed run's keys, numbers and bank
// codes has an id of its own. No id holds any key or any value of the run,
// or any text the store masks or hides: a value made only of hex digits,
// such as a French RIB key "06", may sit in a digest by chance, and is let
// pass only there, as the digest is checked whole. Mutations: put a key or
// a value in an id, glued to letters or inside the digest, and the id is
// not its prefix and digest, and holds it; count calls in digest, and the
// second load gives other ids.
func TestIDsCarryNoValue(t *testing.T) {
	eq(t, "a German bank's id", OrgID("BIC COBADEFF"), "org:9e577a3773d0d204")
	eq(t, "a Dutch IBAN's id", IdentifierID("iban", "NL91ABNA0417164300"), "identifier:2288ef2e20318bbd")
	eq(t, "a German BIC's id", BankCodeID("bic", "COBADEFFXXX"), "bankcode:45751cfa78b71419")
	run := committedRun(t)
	keys, numbers, codes := map[string]bool{}, map[[2]string]bool{}, map[[2]string]bool{}
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
			if pack.Public(ident.Kind) {
				codes[[2]string{ident.Kind, ident.Value}] = true
			} else {
				numbers[[2]string{ident.Kind, ident.Value}] = true
			}
		}
	}
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		mustLoad(t, ctx, e, run)
		things := func(typeID string) []*ontology.Entity {
			found, err := e.Things(ctx, typeID)
			if err != nil {
				t.Fatal(err)
			}
			return found
		}
		orgs, idents, bankCodes := things(pack.Organisation), things(pack.Identifier), things(pack.BankCode)
		eq(t, "organisations, one per key", len(orgs), len(keys))
		eq(t, "numbers, one per kind and value", len(idents), len(numbers))
		eq(t, "bank codes, one per kind and code", len(bankCodes), len(codes))
		digests := map[string]bool{}
		digestID := func(id, prefix, of string) {
			t.Helper()
			sum := sha256.Sum256([]byte(of))
			if want := prefix + ":" + hex.EncodeToString(sum[:8]); id != want {
				t.Errorf("the id of %q is %s, want %s", of, id, want)
				return
			}
			digests[id] = true
		}
		looked := map[string]bool{}
		for _, o := range orgs {
			key := o.Attributes[pack.Key][0].(string)
			digestID(o.ID, "org", key)
			looked[key] = true
		}
		for _, n := range idents {
			kind, value := n.Attributes[pack.Kind][0].(string), n.Attributes[pack.Value][0].(string)
			digestID(n.ID, "identifier", kind+":"+value)
			looked[value] = true
		}
		for _, c := range bankCodes {
			kind, code := c.Attributes[pack.Kind][0].(string), c.Attributes[pack.Code][0].(string)
			digestID(c.ID, "bankcode", kind+":"+code)
			looked[code] = true
		}
		for key := range keys {
			looked[key] = true
		}
		for n := range numbers {
			looked[n[1]] = true
		}
		for c := range codes {
			looked[c[1]] = true
		}
		box := e.interp.TBox()
		all, err := e.interp.QueryEntities(ctx, store.EntityPattern{})
		if err != nil {
			t.Fatal(err)
		}
		for _, thing := range all {
			for attrID, vs := range thing.Attributes {
				if d := box.GetAttributeType(attrID).Display; d == nil || !d.Mask && !d.Hidden {
					continue
				}
				for _, v := range vs {
					if text, ok := v.(string); ok {
						looked[text] = true
					}
				}
			}
		}
		hexOnly, byChance := regexp.MustCompile(`^[0-9a-f]+$`), 0
		for _, thing := range all {
			id, prefix := thing.ID, thing.ID[:strings.IndexByte(thing.ID, ':')+1]
			for v := range looked {
				if !strings.Contains(id, v) {
					continue
				}
				if digests[id] && hexOnly.MatchString(v) && !strings.Contains(prefix, v) {
					byChance++
					continue
				}
				t.Errorf("the id %s holds %q", id, v)
			}
		}
		if len(digests) != len(orgs)+len(idents)+len(bankCodes) {
			t.Fatalf("%d of %d ids are their prefix and digest", len(digests), len(orgs)+len(idents)+len(bankCodes))
		}
		ids := func(e *Engine) []string {
			all, err := e.interp.QueryEntities(ctx, store.EntityPattern{})
			if err != nil {
				t.Fatal(err)
			}
			var out []string
			for _, thing := range all {
				out = append(out, thing.ID)
			}
			slices.Sort(out)
			return out
		}
		first := ids(e)
		mustLoad(t, ctx, reopen(), run)
		eq(t, "ids after a second load", ids(reopen()), first)
		t.Logf("%d things, %d keys, values and masked or hidden texts looked for in their ids, %d found in a digest by chance", len(first), len(looked), byChance)
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
