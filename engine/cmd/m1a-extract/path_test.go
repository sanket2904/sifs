package main

import (
	"encoding/json"
	"maps"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"testing"
)

// tempting is a folder whose parts read like a passport, a bank, an IBAN and
// a statement's date.
const tempting = "/Volumes/Archive/passport/Commerzbank/DE89370400440532013000/statement 2031-04-15/"

// sampleWithFiles copies the committed text set with each document's file
// given by file, and returns the run the extractor makes of it.
func sampleWithFiles(t *testing.T, file func(string) string) map[string]any {
	t.Helper()
	from := "../../../sample/text"
	text := filepath.Join(t.TempDir(), "text")
	if err := os.Mkdir(text, 0o755); err != nil {
		t.Fatal(err)
	}
	var index struct {
		Documents []string `json:"documents"`
	}
	raw, err := os.ReadFile(filepath.Join(from, "index.json"))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(raw, &index); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(text, "index.json"), raw, 0o644); err != nil {
		t.Fatal(err)
	}
	for _, id := range index.Documents {
		var doc map[string]any
		raw, err := os.ReadFile(filepath.Join(from, id+".json"))
		if err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(raw, &doc); err != nil {
			t.Fatal(err)
		}
		doc["file"] = file(doc["file"].(string))
		writeJSON(t, filepath.Join(text, id+".json"), doc)
	}
	out := filepath.Join(t.TempDir(), "out.json")
	if err := run(text, "../../../sample/directory/banks.json", "../../../sample/iban-registry.json", out, "go-m1a"); err != nil {
		t.Fatal(err)
	}
	var o map[string]any
	raw, err = os.ReadFile(out)
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(raw, &o); err != nil {
		t.Fatal(err)
	}
	return o
}

// takeFiles removes each document's file from a run and returns them by id.
func takeFiles(t *testing.T, o map[string]any) map[string]string {
	t.Helper()
	files := map[string]string{}
	for _, d := range o["documents"].([]any) {
		doc := d.(map[string]any)
		files[doc["id"].(string)] = doc["file"].(string)
		delete(doc, "file")
	}
	return files
}

// The extractor reads a document by its text alone. The committed sample's
// documents, from the Netherlands, Germany, Britain, France, Japan, Canada
// and India, read once with each file a name and once with each file in a
// folder named like a passport, a bank, an IBAN and a date, give runs equal
// in every field but the file.
func TestTheFolderChangesNothingButTheFile(t *testing.T) {
	named := sampleWithFiles(t, func(name string) string { return name })
	foldered := sampleWithFiles(t, func(name string) string { return tempting + name })
	names, paths := takeFiles(t, named), takeFiles(t, foldered)
	if len(names) < 40 {
		t.Fatalf("the run has %d documents, want the whole sample", len(names))
	}
	for id, name := range names {
		if paths[id] != tempting+name {
			t.Errorf("%s: the run gives the file %q, want %q", id, paths[id], tempting+name)
		}
	}
	for _, key := range slices.Sorted(maps.Keys(named)) {
		if key == "documents" {
			continue
		}
		if !reflect.DeepEqual(named[key], foldered[key]) {
			t.Errorf("the run's %s differs when files are in a folder", key)
		}
	}
	a, b := named["documents"].([]any), foldered["documents"].([]any)
	if len(a) != len(b) {
		t.Fatalf("%d documents with names, %d in a folder", len(a), len(b))
	}
	for i := range a {
		da, db := a[i].(map[string]any), b[i].(map[string]any)
		for key := range da {
			if !reflect.DeepEqual(da[key], db[key]) {
				t.Errorf("document %v: its %s differs when its file is in a folder", da["id"], key)
			}
		}
	}
	if !reflect.DeepEqual(named, foldered) {
		t.Error("the runs differ in more than the file")
	}
}
