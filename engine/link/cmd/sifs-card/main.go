// Command sifs-card answers from the engine's store in a data dir, which
// sifs-load filled: the organisation cards for a typed word or phrase, or
// one kind of question.
//
//	sifs-card --data-dir <dir> --json "<words>"
//	sifs-card --data-dir <dir> --ask "<question>" [--reference-date YYYY-MM-DD]
//
// It prints one JSON document and writes nothing to the store. It exits 0
// with an answer, also when nothing matches or the question is not
// answered yet, and 2 when it could not run.
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"
	"time"

	"github.com/sanket2904/sifs/engine/link/card"
	"github.com/sanket2904/sifs/engine/link/load"
)

func main() { os.Exit(runMain(os.Args[1:], os.Stdout, os.Stderr)) }

type found struct {
	card.Answer
	QueryMS int64 `json:"query_ms"`
}

type asked struct {
	card.Asked
	QueryMS int64 `json:"query_ms"`
}

func runMain(args []string, stdout, stderr io.Writer) int {
	flags := flag.NewFlagSet("sifs-card", flag.ContinueOnError)
	flags.SetOutput(stderr)
	dataDir := flags.String("data-dir", "", "the data dir that holds "+load.StoreFile)
	words := flags.String("json", "", "the word or phrase to give cards for")
	question := flags.String("ask", "", "the question to answer")
	refDate := flags.String("reference-date", time.Now().Format(time.DateOnly), "the date that decides which expiries are past")
	if err := flags.Parse(args); err != nil {
		return 2
	}
	if *dataDir == "" || (*words == "") == (*question == "") || flags.NArg() > 0 {
		fmt.Fprintln(stderr, `usage: sifs-card --data-dir <dir> --json "<words>" | sifs-card --data-dir <dir> --ask "<question>" [--reference-date YYYY-MM-DD]`)
		return 2
	}
	if _, err := time.Parse(time.DateOnly, *refDate); err != nil {
		fmt.Fprintln(stderr, "sifs-card: --reference-date:", err)
		return 2
	}
	out, err := do(context.Background(), *dataDir, *words, *question, *refDate)
	if err != nil {
		fmt.Fprintln(stderr, "sifs-card:", err)
		return 2
	}
	if err := json.NewEncoder(stdout).Encode(out); err != nil {
		fmt.Fprintln(stderr, "sifs-card:", err)
		return 2
	}
	return 0
}

func do(ctx context.Context, dataDir, words, question, refDate string) (any, error) {
	e, err := load.OpenExisting(ctx, dataDir)
	if err != nil {
		return nil, err
	}
	defer e.Close()
	start := time.Now()
	if words != "" {
		ans, err := card.Find(ctx, e, words)
		return found{ans, time.Since(start).Milliseconds()}, err
	}
	ans, err := card.Ask(ctx, e, question, refDate)
	return asked{ans, time.Since(start).Milliseconds()}, err
}
