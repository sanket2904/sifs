package load_test

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/card"
	"github.com/sanket2904/sifs/engine/link/load"
	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/store"
)

var words = []string{"HDFC", "Commerzbank", "ABN AMRO", "COBADEFFXXX", "NL91ABNA0417164300", "ERIKSSON", "Mizuho"}

// files is the SHA-256 of every file in dir, by name.
func files(t *testing.T, dir string) map[string]string {
	t.Helper()
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	out := map[string]string{}
	for _, entry := range entries {
		b, err := os.ReadFile(filepath.Join(dir, entry.Name()))
		if err != nil {
			t.Fatal(err)
		}
		sum := sha256.Sum256(b)
		out[entry.Name()] = hex.EncodeToString(sum[:])
	}
	return out
}

func asJSON(t *testing.T, v any) string {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

// cards is what sifs-card answers from dataDir: the cards for each of
// words, and when each passport expires.
func cards(t *testing.T, dataDir string) []string {
	t.Helper()
	ctx := context.Background()
	e, err := load.OpenExisting(ctx, dataDir)
	if err != nil {
		t.Fatal(err)
	}
	defer e.Close()
	var out []string
	for _, w := range words {
		ans, err := card.Find(ctx, e, w)
		if err != nil {
			t.Fatal(err)
		}
		out = append(out, asJSON(t, ans))
	}
	ans, err := card.Ask(ctx, e, "when does my passport expire", "2026-11-27")
	if err != nil {
		t.Fatal(err)
	}
	return append(out, asJSON(t, ans))
}

// reads is what sifs-show answers from dataDir: its kinds, a find by each
// of words, and the page of each of ids, or of every thing it holds when
// ids is nil.
type reads struct {
	kinds show.Kinds
	finds []string
	pages map[string]show.Page
}

func shows(t *testing.T, dataDir string, ids []string) reads {
	t.Helper()
	ctx := context.Background()
	interp, closer, err := show.Open(ctx, filepath.Join(dataDir, load.StoreFile))
	if err != nil {
		t.Fatal(err)
	}
	defer closer()
	if ids == nil {
		things, err := interp.QueryEntities(ctx, store.EntityPattern{})
		if err != nil {
			t.Fatal(err)
		}
		for _, thing := range things {
			ids = append(ids, thing.ID)
		}
	}
	r := show.New(interp, false)
	out := reads{pages: map[string]show.Page{}}
	if out.kinds, err = r.Kinds(ctx); err != nil {
		t.Fatal(err)
	}
	for _, w := range words {
		found, err := r.Find(ctx, w)
		if err != nil {
			t.Fatal(err)
		}
		out.finds = append(out.finds, asJSON(t, found))
	}
	for _, id := range ids {
		if out.pages[id], err = r.Page(ctx, id); err != nil {
			t.Fatalf("page %s: %v", id, err)
		}
	}
	return out
}

func valueOf(p show.Page, label string) []string {
	for _, v := range p.Values {
		if v.Label == label {
			return v.Values
		}
	}
	return nil
}

// A data dir filled at 6c6f1d6, read before sifs-load runs on it again.
// Reading it writes no byte of it. The card answers as on a fresh store;
// the reader answers from the schema and values the store holds, a bank's
// code still a masked number, a statement titled by its end day as the
// store spells it. After the next load both answer as on a fresh store.
// Mutations: publish the files pack in OpenExisting, and the store's file
// changes; read the word cannot-tell as an expiry's date in the card, and
// the passport's answer says likely-only.
func TestReadAnOlderStore(t *testing.T) {
	run, err := load.ReadRun("../../../sample/runs/go-m1a-on-text.json")
	if err != nil {
		t.Fatal(err)
	}
	fresh, older := t.TempDir(), t.TempDir()
	loadInto(t, fresh, run)
	load.FillAtBase(t, older, run)
	before := files(t, older)
	if len(before) == 0 {
		t.Fatal("the older data dir holds no file")
	}

	freshCards := cards(t, fresh)
	if got := cards(t, older); !slices.Equal(got, freshCards) {
		t.Errorf("the card on the older store:\n%s\nwant as on a fresh store:\n%s", strings.Join(got, "\n"), strings.Join(freshCards, "\n"))
	}
	iban := load.IdentifierID("iban", "NL91ABNA0417164300")
	old := shows(t, older, []string{"statement:abn-amro-statement", iban})
	counts := map[string]int{}
	for _, k := range old.kinds.Kinds {
		counts[k.Label] = k.Count
	}
	if counts["Number or code"] != 56 || counts["Bank code"] != 0 {
		t.Errorf("the older store's kinds: %v", counts)
	}
	stmt := old.pages["statement:abn-amro-statement"]
	if stmt.Title != "2026-08-31" || !slices.Equal(valueOf(stmt, "From"), []string{"2026-08-01"}) {
		t.Errorf("the older store's statement: title %q, from %q", stmt.Title, valueOf(stmt, "From"))
	}
	if p := old.pages[iban]; p.Title != "••••••••••••••4300" || strings.Contains(asJSON(t, p), "NL91ABNA0417164300") {
		t.Errorf("the older store's IBAN is given as %q", p.Title)
	}
	if after := files(t, older); !maps.Equal(after, before) {
		t.Fatalf("reading the older store changed it: %v, then %v", before, after)
	}

	loadInto(t, older, run)
	if got := cards(t, older); !slices.Equal(got, freshCards) {
		t.Errorf("the card after the load differs from a fresh store's")
	}
	want, got := shows(t, fresh, nil), shows(t, older, nil)
	if asJSON(t, got.kinds) != asJSON(t, want.kinds) || !slices.Equal(got.finds, want.finds) || len(got.pages) != len(want.pages) {
		t.Errorf("after the load the reader's kinds, finds or things differ from a fresh store's")
	}
	for id, p := range want.pages {
		if asJSON(t, got.pages[id]) != asJSON(t, p) {
			t.Errorf("after the load the page of %s differs from a fresh store's", id)
		}
	}
}

func loadInto(t *testing.T, dataDir string, run *extract.Output) {
	t.Helper()
	ctx := context.Background()
	e, err := load.Open(ctx, dataDir)
	if err != nil {
		t.Fatal(err)
	}
	defer e.Close()
	rep, err := e.Load(ctx, run)
	if err != nil || len(rep.TurnedDown) > 0 {
		t.Fatalf("load: %v %+v", err, rep.TurnedDown)
	}
}
