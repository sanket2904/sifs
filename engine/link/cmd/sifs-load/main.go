// Command sifs-load puts a format 3 run into the engine's store in a data
// dir, or prints that store's counts.
//
//	sifs-load --run <format 3 file> --data-dir <dir>
//	sifs-load --data-dir <dir> --counts
//
// It prints one line of JSON. It exits 1 when the engine turned a record
// down (the others still load) and 2 when it could not run.
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"os"
	"time"

	"github.com/sanket2904/sifs/engine/link/load"
)

type loaded struct {
	Documents  documents       `json:"documents"`
	TurnedDown []load.TurnDown `json:"turned_down"`
	load.Counts
	LoadMS int64 `json:"load_ms"`
}

type documents struct {
	Written   int `json:"written"`
	Unchanged int `json:"unchanged"`
	Removed   int `json:"removed"`
}

func main() { os.Exit(runMain(os.Args[1:], os.Stdout, os.Stderr)) }

func runMain(args []string, stdout, stderr io.Writer) int {
	flags := flag.NewFlagSet("sifs-load", flag.ContinueOnError)
	flags.SetOutput(stderr)
	runPath := flags.String("run", "", "the format 3 run to load")
	dataDir := flags.String("data-dir", "", "the data dir that holds "+load.StoreFile)
	counts := flags.Bool("counts", false, "print the store's counts and write nothing")
	if err := flags.Parse(args); err != nil {
		return 2
	}
	if *dataDir == "" || (*runPath == "") == !*counts {
		fmt.Fprintln(stderr, "usage: sifs-load --run <format 3 file> --data-dir <dir> | sifs-load --data-dir <dir> --counts")
		return 2
	}
	out, code, err := do(context.Background(), *runPath, *dataDir, *counts)
	if err != nil {
		fmt.Fprintln(stderr, "sifs-load:", err)
		return 2
	}
	if err := json.NewEncoder(stdout).Encode(out); err != nil {
		fmt.Fprintln(stderr, "sifs-load:", err)
		return 2
	}
	return code
}

func do(ctx context.Context, runPath, dataDir string, countsOnly bool) (any, int, error) {
	if countsOnly {
		e, err := load.OpenExisting(ctx, dataDir)
		if err != nil {
			return nil, 0, err
		}
		defer e.Close()
		c, err := e.Count(ctx)
		return c, 0, err
	}
	run, err := load.ReadRun(runPath)
	if err != nil {
		return nil, 0, err
	}
	start := time.Now()
	e, err := load.Open(ctx, dataDir)
	if err != nil {
		return nil, 0, err
	}
	defer e.Close()
	rep, err := e.Load(ctx, run)
	if err != nil {
		return nil, 0, err
	}
	elapsed := time.Since(start)
	c, err := e.Count(ctx)
	if err != nil {
		return nil, 0, err
	}
	out := loaded{Documents: documents{rep.Written, rep.Unchanged, rep.Removed}, TurnedDown: rep.TurnedDown, Counts: c, LoadMS: elapsed.Milliseconds()}
	if len(rep.TurnedDown) > 0 {
		return out, 1, nil
	}
	return out, 0, nil
}
