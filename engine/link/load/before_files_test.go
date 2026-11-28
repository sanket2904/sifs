package load_test

import (
	"context"
	"maps"
	"path/filepath"
	"slices"
	"testing"

	"github.com/sanket2904/sifs/engine/link/load"
	"github.com/sanket2904/sifs/engine/link/pack"
	"github.com/sanket2904/sifs/engine/link/show"
)

// A data dir filled at 2afc7bb, before a document had a file name, read
// before sifs-load runs on it again. Reading it writes no byte of it, each
// document reads as its id, and typing a file name finds no document. After
// the next load the reader answers as on a fresh store. Mutation: publish
// the files pack in OpenExisting, and the store's file changes.
func TestReadAStoreFilledBeforeFileNames(t *testing.T) {
	run, err := load.ReadRun("../../../sample/runs/go-m1a-on-text.json")
	if err != nil {
		t.Fatal(err)
	}
	fresh, older := t.TempDir(), t.TempDir()
	loadInto(t, fresh, run)
	load.FillBeforeFiles(t, older, run)
	before := files(t, older)

	var ids []string
	for _, doc := range run.Documents {
		ids = append(ids, load.DocumentPrefix+doc.ID)
	}
	old := shows(t, older, ids)
	for _, doc := range run.Documents {
		p := old.pages[load.DocumentPrefix+doc.ID]
		if p.Title != load.DocumentPrefix+doc.ID || p.Subtitle != "" || valueOf(p, "File name") != nil {
			t.Errorf("the older store's %s reads as %q %q %v", doc.ID, p.Title, p.Subtitle, valueOf(p, "File name"))
		}
	}
	ctx := context.Background()
	interp, closer, err := show.Open(ctx, filepath.Join(older, load.StoreFile))
	if err != nil {
		t.Fatal(err)
	}
	r := show.New(interp, false)
	for _, words := range []string{"kalpit-statement-a.pdf", "kontoauszug-oktober.pdf", "torihiki-meisai.pdf"} {
		found, err := r.Find(ctx, words)
		if err != nil {
			t.Fatal(err)
		}
		for _, g := range found.Groups {
			if g.Type == pack.Document {
				t.Errorf("on the older store %q finds documents %v", words, g.Things)
			}
		}
	}
	closer()
	if after := files(t, older); !maps.Equal(after, before) {
		t.Fatalf("reading the older store changed it: %v, then %v", before, after)
	}

	loadInto(t, older, run)
	want, got := shows(t, fresh, nil), shows(t, older, nil)
	if asJSON(t, got.kinds) != asJSON(t, want.kinds) || !slices.Equal(got.finds, want.finds) || len(got.pages) != len(want.pages) {
		t.Errorf("after the load the reader's kinds, finds or things differ from a fresh store's")
	}
	for id, p := range want.pages {
		if asJSON(t, got.pages[id]) != asJSON(t, p) {
			t.Errorf("after the load the page of %s differs from a fresh store's", id)
		}
	}
	if p := got.pages[load.DocumentPrefix+"hdfc-statement-sep"]; p.Title != "kalpit-statement-a.pdf" {
		t.Errorf("after the load hdfc-statement-sep reads as %q", p.Title)
	}
}
