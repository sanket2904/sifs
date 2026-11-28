package load

import (
	"context"
	"encoding/json"
	"fmt"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/pack"
	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/schema"
	"github.com/sanket2904/ontology-framework/store"
)

// privacy ranks how much of an attribute's values a person is given: all of
// them (0), the last four characters (1, masked) or none (2, hidden). An
// attribute a schema does not declare is given whole.
func privacy(box *ontology.TBox, attrID string) int {
	at := box.GetAttributeType(attrID)
	switch {
	case at == nil || at.Display == nil:
		return 0
	case at.Display.Hidden:
		return 2
	case at.Display.Mask:
		return 1
	}
	return 0
}

// Every attribute the files pack masked or hid at 6c6f1d6 is masked or
// hidden at least as much by this one. A store holds its old values until
// the next load replaces each record, and a turned-down record keeps them,
// so a name that held private values must never come to show them.
// Mutation: declare Shows' text without the mask, and its old values are
// shown.
func TestOldMasksKept(t *testing.T) {
	base, err := schema.CompileModules(baseModule())
	if err != nil {
		t.Fatal(err)
	}
	now, err := pack.Compile()
	if err != nil {
		t.Fatal(err)
	}
	kept := 0
	for _, id := range sortedKeys(base.AttributeTypes) {
		was, is := privacy(base, id), privacy(now, id)
		if was == 0 {
			continue
		}
		kept++
		if is < was {
			t.Errorf("%s: %d at the base, %d now (0 shown, 1 masked, 2 hidden)", id, was, is)
		}
	}
	if kept == 0 {
		t.Fatal("the base masked or hid nothing")
	}
	t.Logf("%d attributes masked or hidden at the base", kept)
}

// privateTexts is every number of run that is not a public bank code, as
// the run gives its value and its printed text, each also with its spaces
// removed. A text under five characters, the French RIB key "06", is left
// out: a day or a page number holds it by chance. Every account number,
// IBAN, passport number and zone is longer.
func privateTexts(t *testing.T, run *extract.Output) []string {
	t.Helper()
	set := map[string]bool{}
	for _, doc := range run.Documents {
		for _, n := range doc.Identifiers {
			if pack.Public(n.Kind) {
				continue
			}
			for _, s := range []string{n.Value, n.Text} {
				for _, s := range []string{s, strings.ReplaceAll(s, " ", "")} {
					switch {
					case utf8.RuneCountInString(s) >= 5:
						set[s] = true
					case slices.Contains([]string{"account", "iban", "passport_number", "mrz"}, n.Kind):
						t.Fatalf("the %s %q is too short to look for", n.Kind, s)
					}
				}
			}
		}
	}
	return sortedKeys(set)
}

// readAll is everything the reader gives, without reveal, from the store in
// dataDir: its kinds, a find by every text the store holds and the page of
// every thing, as each string field of the answers with where it is.
func readAll(t *testing.T, ctx context.Context, dataDir string) map[string]string {
	t.Helper()
	interp, closer, err := show.Open(ctx, filepath.Join(dataDir, StoreFile))
	if err != nil {
		t.Fatal(err)
	}
	defer closer()
	things, err := interp.QueryEntities(ctx, store.EntityPattern{})
	if err != nil {
		t.Fatal(err)
	}
	r := show.New(interp, false)
	answers := map[string]any{}
	if answers["kinds"], err = r.Kinds(ctx); err != nil {
		t.Fatal(err)
	}
	texts := map[string]bool{}
	for _, thing := range things {
		if answers["page "+thing.ID], err = r.Page(ctx, thing.ID); err != nil {
			t.Fatalf("page %s: %v", thing.ID, err)
		}
		for _, vs := range thing.Attributes {
			for _, v := range vs {
				if s, ok := v.(string); ok {
					texts[s] = true
				}
			}
		}
	}
	for s := range texts {
		if answers["find "+s], err = r.Find(ctx, s); err != nil {
			t.Fatal(err)
		}
	}
	b, err := json.Marshal(answers)
	if err != nil {
		t.Fatal(err)
	}
	var tree any
	if err := json.Unmarshal(b, &tree); err != nil {
		t.Fatal(err)
	}
	fields := map[string]string{}
	var walk func(at string, v any)
	walk = func(at string, v any) {
		switch v := v.(type) {
		case map[string]any:
			for k, c := range v {
				walk(at+"/"+k, c)
			}
		case []any:
			for i, c := range v {
				walk(fmt.Sprintf("%s/%d", at, i), c)
			}
		case string:
			fields[at] = v
		}
	}
	walk("", tree)
	return fields
}

// noPrivate fails for each field of what the reader gives from dataDir that
// holds one of private.
func noPrivate(t *testing.T, ctx context.Context, when, dataDir string, private []string) {
	t.Helper()
	fields := readAll(t, ctx, dataDir)
	for _, at := range sortedKeys(fields) {
		for _, p := range private {
			if strings.Contains(fields[at], p) {
				t.Errorf("%s: %s gives %q", when, at, fields[at])
			}
		}
	}
	t.Logf("%s: %d fields looked through for %d private texts", when, len(fields), len(private))
}

// A person's store filled at 6c6f1d6, upgraded by sifs-load: the files
// pack is published first and each document's record is replaced after.
// At no point does the reader give an account number, an IBAN, a passport
// number or a zone raw: with the pack published and no record replaced,
// with one document turned down and the others replaced, and with every
// record replaced. The turned-down document is named, and the other 40
// documents are as a fresh store holds them. Mutation: declare Shows' text
// shown, and an old row's IBAN is given raw at the first point.
func TestUpgradeShowsNoPrivateNumber(t *testing.T) {
	ctx := context.Background()
	run := committedRun(t)
	private := privateTexts(t, run)
	const refused = "commerzbank-kontoauszug"
	docs, others := slices.Clone(run.Documents), &extract.Output{}
	*others = *run
	others.Documents = nil
	var refusedDoc extract.DocOut
	for i := range docs {
		if docs[i].ID != refused {
			others.Documents = append(others.Documents, docs[i])
			continue
		}
		refusedDoc = docs[i]
		if len(docs[i].Links) == 0 {
			t.Fatalf("%s has no link to turn down", refused)
		}
		docs[i].Links = slices.Clone(docs[i].Links)
		docs[i].Links[0].Tier = "invalid-tier"
	}
	bad := *run
	bad.Documents = docs

	dir := t.TempDir()
	FillAtBase(t, dir, run)
	load := func(run *extract.Output) Report {
		t.Helper()
		e, err := Open(ctx, dir)
		if err != nil {
			t.Fatal(err)
		}
		defer e.Close()
		if run == nil {
			return Report{}
		}
		return mustLoad(t, ctx, e, run)
	}
	stored := func() map[string]string {
		t.Helper()
		e, err := OpenExisting(ctx, dir)
		if err != nil {
			t.Fatal(err)
		}
		defer e.Close()
		return held(t, ctx, e)
	}

	load(nil)
	noPrivate(t, ctx, "the pack published, no record replaced", dir, private)

	rep := load(&bad)
	if len(rep.TurnedDown) != 1 || rep.TurnedDown[0].Document != refused || rep.Written+rep.Unchanged != len(run.Documents)-1 {
		t.Fatalf("the load with %s turned down: %+v", refused, rep)
	}
	noPrivate(t, ctx, "one document turned down", dir, private)
	named := map[string]bool{}
	for _, thing := range Facts(refusedDoc, run.Organisations).Entities {
		named[thing.ID] = true
	}
	fresh, mid, compared := freshly(t, ctx, others), stored(), 0
	for _, id := range sortedKeys(fresh) {
		if named[id] {
			continue
		}
		compared++
		if mid[id] != fresh[id] {
			t.Errorf("with %s turned down, %s is\n %s\nwant\n %s", refused, id, mid[id], fresh[id])
		}
	}
	t.Logf("with %s turned down, %d things and links of the other %d documents as a fresh store holds them", refused, compared, len(others.Documents))

	eq(t, "the load after", load(run).TurnedDown, []TurnDown{})
	noPrivate(t, ctx, "every record replaced", dir, private)
	eq(t, "the store with every record replaced", stored(), freshly(t, ctx, run))
}
