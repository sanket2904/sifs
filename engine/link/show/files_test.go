package show_test

import (
	"bytes"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"testing"

	"github.com/sanket2904/sifs/engine/link/load"
	"github.com/sanket2904/sifs/engine/link/pack"
	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/store"
)

const runFile = "../../../sample/runs/go-m1a-on-text.json"

// memoryStore is an engine on its memory store holding records, written
// under box.
func memoryStore(t *testing.T, box *ontology.TBox, records map[string]interpreter.RecordFacts, source string) *interpreter.Interpreter {
	t.Helper()
	ctx := context.Background()
	interp, err := interpreter.New(ctx, store.NewMemorySchemaStore(), store.NewMemoryStore(), interpreter.WithAnonymousWrites(), interpreter.WithUnverifiedSourceClaims())
	if err != nil {
		t.Fatal(err)
	}
	writeRecords(t, interp, box, records, source)
	return interp
}

// writeRecords publishes box and writes each record whole.
func writeRecords(t *testing.T, interp *interpreter.Interpreter, box *ontology.TBox, records map[string]interpreter.RecordFacts, source string) {
	t.Helper()
	ctx := context.Background()
	_, version, err := interp.SchemaSnapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if err := interp.PublishSchema(ctx, box, version); err != nil {
		t.Fatal(err)
	}
	for _, id := range sortedIDs(records) {
		rec := interpreter.SourceRecord{SourceSystem: source, SourceInstanceID: "local", RecordID: id}
		if _, err := interp.ReplaceRecord(ctx, rec, records[id]); err != nil {
			t.Fatalf("record %s: %v", id, err)
		}
	}
}

var update = flag.Bool("update", false, "write the goldens from this run")

// golden compares got with testdata/name, or writes it there with -update.
func golden(t *testing.T, name string, got []byte) {
	t.Helper()
	path := filepath.Join("testdata", name)
	if *update {
		if err := os.WriteFile(path, got, 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != string(want) {
		t.Fatalf("%s differs from what the store gives:\n%s", path, firstDiff(string(want), string(got)))
	}
}

func firstDiff(want, got string) string {
	w, g := strings.Split(want, "\n"), strings.Split(got, "\n")
	for i := range min(len(w), len(g)) {
		if w[i] != g[i] {
			return fmt.Sprintf("line %d\n  want %s\n  got  %s", i+1, w[i], g[i])
		}
	}
	return fmt.Sprintf("want %d lines, got %d", len(w), len(g))
}

// filesOnMemory is the committed run on the memory store, each document
// written as the loader writes it.
func filesOnMemory(t *testing.T) *interpreter.Interpreter {
	t.Helper()
	run, err := load.ReadRun(runFile)
	if err != nil {
		t.Fatal(err)
	}
	box, err := pack.Compile()
	if err != nil {
		t.Fatal(err)
	}
	records := map[string]interpreter.RecordFacts{}
	for _, doc := range run.Documents {
		records[doc.ID] = load.Facts(doc, run.Organisations)
	}
	return memoryStore(t, box, records, load.SourceSystem)
}

var (
	filesDirOnce sync.Once
	filesDir     string
	filesDirErr  error
)

// filesOnSQLite is a data dir the loader filled with the committed run,
// made once for the package's tests and only read after.
func filesOnSQLite(t *testing.T) string {
	t.Helper()
	filesDirOnce.Do(func() {
		dir, err := os.MkdirTemp("", "sifs-show-")
		if err != nil {
			filesDirErr = err
			return
		}
		filesDir = dir
		filesDirErr = loadRun(dir)
	})
	if filesDirErr != nil {
		t.Fatal(filesDirErr)
	}
	return filesDir
}

func loadRun(dir string) error {
	ctx := context.Background()
	run, err := load.ReadRun(runFile)
	if err != nil {
		return err
	}
	e, err := load.Open(ctx, dir)
	if err != nil {
		return err
	}
	if _, err := e.Load(ctx, run); err != nil {
		e.Close()
		return err
	}
	return e.Close()
}

func openSQLite(t *testing.T, dir string) *interpreter.Interpreter {
	t.Helper()
	interp, closer, err := show.Open(context.Background(), filepath.Join(dir, load.StoreFile))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { closer() })
	return interp
}

func TestMain(m *testing.M) {
	code := m.Run()
	if filesDir != "" {
		os.RemoveAll(filesDir)
	}
	os.Exit(code)
}

func sortedIDs[V any](m map[string]V) []string {
	ids := make([]string, 0, len(m))
	for id := range m {
		ids = append(ids, id)
	}
	slices.Sort(ids)
	return ids
}

func encode(t *testing.T, v any) []byte {
	t.Helper()
	var b bytes.Buffer
	enc := json.NewEncoder(&b)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", "  ")
	if err := enc.Encode(v); err != nil {
		t.Fatal(err)
	}
	return b.Bytes()
}

// The words and things the goldens are drawn from, across the sample.
var (
	filesWords = map[string]string{
		"a word of several kinds":      "smbk",
		"a German bank's printed name": "Commerzbank",
		"a holder's surname":           "Eriksson",
		"a full IBAN":                  "DE89370400440532013000",
		"a word that is nowhere":       "Zanzibar",
	}
	filesPages = map[string]string{
		"a German bank":     "org:BIC COBADEFF",
		"a Dutch IBAN":      "identifier:iban:NL91ABNA0417164300",
		"a Dutch statement": "statement:abn-amro-statement",
		"a passport":        "passport:passport-bad-mrz",
		"its holder":        "holder:passport-bad-mrz",
	}
)

// filesReads are the goldens' three reads on one store, each as one
// document.
func filesReads(t *testing.T, r *show.Reader) map[string][]byte {
	t.Helper()
	ctx := context.Background()
	kinds, err := r.Kinds(ctx)
	if err != nil {
		t.Fatal(err)
	}
	finds := map[string]show.Found{}
	for name, words := range filesWords {
		if finds[name], err = r.Find(ctx, words); err != nil {
			t.Fatal(err)
		}
	}
	pages := map[string]show.Page{}
	for name, id := range filesPages {
		if pages[name], err = r.Page(ctx, id); err != nil {
			t.Fatal(err)
		}
	}
	return map[string][]byte{"files-kinds.json": encode(t, kinds), "files-find.json": encode(t, finds), "files-pages.json": encode(t, pages)}
}

// Mutation: drop the sort of a page's link rows, and the pages differ from
// the golden.
func TestFilesGoldens(t *testing.T) {
	stores := []struct {
		name string
		open func(*testing.T) *interpreter.Interpreter
	}{
		{"memory", filesOnMemory},
		{"sqlite", func(t *testing.T) *interpreter.Interpreter { return openSQLite(t, filesOnSQLite(t)) }},
	}
	for _, s := range stores {
		t.Run(s.name, func(t *testing.T) {
			r := show.New(s.open(t), false)
			got := filesReads(t, r)
			for _, name := range sortedIDs(got) {
				golden(t, name, got[name])
			}
			again := filesReads(t, r)
			for name := range got {
				if !bytes.Equal(again[name], got[name]) {
					t.Fatalf("%s: the same store read twice gave different bytes", name)
				}
			}
		})
	}
}
