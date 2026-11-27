// Command sifs-show shows what the engine's store in a data dir knows, for
// any kind of thing: the kinds of thing it holds, what a typed word or
// phrase finds, or the page of one thing.
//
//	sifs-show --data-dir <dir> --kinds [--reveal]
//	sifs-show --data-dir <dir> --find "<words>" [--reveal]
//	sifs-show --data-dir <dir> --page <thing id> [--reveal]
//
// It prints one JSON document and writes nothing to the store. Masked
// values are masked unless --reveal is given. It exits 0 with an answer,
// also when nothing is found, and 2 when it could not run.
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"time"

	"github.com/sanket2904/sifs/engine/link/load"
	"github.com/sanket2904/sifs/engine/link/show"
)

func main() { os.Exit(runMain(os.Args[1:], os.Stdout, os.Stderr)) }

const usage = `usage: sifs-show --data-dir <dir> (--kinds | --find "<words>" | --page <thing id>) [--reveal]`

type kinds struct {
	show.Kinds
	QueryMS int64 `json:"query_ms"`
}

type found struct {
	show.Found
	QueryMS int64 `json:"query_ms"`
}

type page struct {
	show.Page
	QueryMS int64 `json:"query_ms"`
}

func runMain(args []string, stdout, stderr io.Writer) int {
	flags := flag.NewFlagSet("sifs-show", flag.ContinueOnError)
	flags.SetOutput(stderr)
	dataDir := flags.String("data-dir", "", "the data dir that holds "+load.StoreFile)
	flags.Bool("kinds", false, "list the kinds of thing the store holds")
	words := flags.String("find", "", "the word or phrase to find things by")
	id := flags.String("page", "", "the id of the thing to show")
	reveal := flags.Bool("reveal", false, "give masked values raw")
	if err := flags.Parse(args); err != nil {
		return 2
	}
	set := map[string]bool{}
	flags.Visit(func(f *flag.Flag) { set[f.Name] = true })
	if *dataDir == "" || flags.NArg() > 0 || count(set, "kinds", "find", "page") != 1 {
		fmt.Fprintln(stderr, usage)
		return 2
	}
	out, err := do(context.Background(), *dataDir, *reveal, set, *words, *id)
	if err != nil {
		fmt.Fprintln(stderr, "sifs-show:", err)
		return 2
	}
	stdout.Write(out)
	return 0
}

func count(set map[string]bool, names ...string) int {
	n := 0
	for _, name := range names {
		if set[name] {
			n++
		}
	}
	return n
}

func do(ctx context.Context, dataDir string, reveal bool, set map[string]bool, words, id string) ([]byte, error) {
	interp, closer, err := show.Open(ctx, filepath.Join(dataDir, load.StoreFile))
	if err != nil {
		return nil, err
	}
	defer closer()
	r := show.New(interp, reveal)
	start := time.Now()
	var answer any
	switch {
	case set["kinds"]:
		var k show.Kinds
		k, err = r.Kinds(ctx)
		answer = kinds{k, time.Since(start).Milliseconds()}
	case set["find"]:
		var f show.Found
		f, err = r.Find(ctx, words)
		answer = found{f, time.Since(start).Milliseconds()}
	default:
		var p show.Page
		p, err = r.Page(ctx, id)
		answer = page{p, time.Since(start).Milliseconds()}
	}
	if err != nil {
		return nil, err
	}
	var b bytes.Buffer
	enc := json.NewEncoder(&b)
	enc.SetEscapeHTML(false)
	err = enc.Encode(answer)
	return b.Bytes(), err
}
