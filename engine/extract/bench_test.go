package extract

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

// BenchmarkRun times one run over the sample's clean text set, directory off
// and on, after the files are read.
func BenchmarkRun(b *testing.B) {
	root := "../../sample"
	var index struct{ Documents []string }
	raw, err := os.ReadFile(filepath.Join(root, "text", "index.json"))
	if err != nil {
		b.Skip("no sample beside the module")
	}
	if err := json.Unmarshal(raw, &index); err != nil {
		b.Fatal(err)
	}
	docs := make([]Document, len(index.Documents))
	for i, id := range index.Documents {
		raw, err := os.ReadFile(filepath.Join(root, "text", id+".json"))
		if err != nil {
			b.Fatal(err)
		}
		if err := json.Unmarshal(raw, &docs[i]); err != nil {
			b.Fatal(err)
		}
	}
	dir, err := LoadDirectory(filepath.Join(root, "directory", "banks.json"))
	if err != nil {
		b.Fatal(err)
	}
	b.Run("off", func(b *testing.B) {
		for range b.N {
			Run(docs, nil, ref)
		}
	})
	b.Run("on", func(b *testing.B) {
		for range b.N {
			Run(docs, dir, ref)
		}
	})
}
