// Command m1a-extract reads a text set and writes the run file that
// sample/FORMAT.md describes.
//
//	m1a-extract -text sample/text [-directory sample/directory/banks.json] -out run.json
//
// The directory is on exactly when -directory is given; a run with it off
// never opens the file.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"

	"github.com/sanket2904/sifs/engine/extract"
)

func main() {
	text := flag.String("text", "", "text set folder (holds index.json)")
	dirPath := flag.String("directory", "", "bank directory file; omit for a run with the directory off")
	out := flag.String("out", "", "output file")
	name := flag.String("extractor", "go-m1a", "extractor name written to the output")
	flag.Parse()
	if *text == "" || *out == "" {
		flag.Usage()
		os.Exit(2)
	}
	if err := run(*text, *dirPath, *out, *name); err != nil {
		fmt.Fprintln(os.Stderr, "m1a-extract:", err)
		os.Exit(1)
	}
}

func run(text, dirPath, out, name string) error {
	var index struct {
		TextSet       string   `json:"text_set"`
		ReferenceDate string   `json:"reference_date"`
		Documents     []string `json:"documents"`
	}
	if err := readJSON(filepath.Join(text, "index.json"), &index); err != nil {
		return err
	}
	ref, ok := extract.ParseDate(index.ReferenceDate)
	if !ok {
		return fmt.Errorf("index.json: reference_date %q is no date", index.ReferenceDate)
	}
	docs := make([]extract.Document, len(index.Documents))
	for i, id := range index.Documents {
		if err := readJSON(filepath.Join(text, id+".json"), &docs[i]); err != nil {
			return err
		}
	}
	var dir *extract.Directory
	if dirPath != "" {
		var err error
		if dir, err = extract.LoadDirectory(dirPath); err != nil {
			return err
		}
	}
	o := extract.Run(docs, dir, ref)
	o.Extractor, o.TextSet = name, index.TextSet
	raw, err := json.MarshalIndent(o, "", " ")
	if err != nil {
		return err
	}
	return os.WriteFile(out, append(raw, '\n'), 0o644)
}

func readJSON(path string, v any) error {
	raw, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	if err := json.Unmarshal(raw, v); err != nil {
		return fmt.Errorf("%s: %w", path, err)
	}
	return nil
}
