package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/link/load"
)

func sifsCard(t *testing.T, args ...string) (int, string, string) {
	t.Helper()
	var stdout, stderr bytes.Buffer
	code := runMain(args, &stdout, &stderr)
	return code, stdout.String(), stderr.String()
}

// loaded is a data dir holding the committed run, its store closed.
func loaded(t *testing.T) string {
	t.Helper()
	run, err := load.ReadRun("../../../../sample/runs/go-m1a-on-text.json")
	if err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	ctx := context.Background()
	e, err := load.Open(ctx, dir)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := e.Load(ctx, run); err != nil {
		t.Fatal(err)
	}
	if err := e.Close(); err != nil {
		t.Fatal(err)
	}
	return dir
}

// state is every file in the data dir with its hash. The store is closed,
// so a write would show in engine.db or leave a file beside it.
func state(t *testing.T, dir string) map[string][32]byte {
	t.Helper()
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	out := map[string][32]byte{}
	for _, entry := range entries {
		b, err := os.ReadFile(filepath.Join(dir, entry.Name()))
		if err != nil {
			t.Fatal(err)
		}
		out[entry.Name()] = sha256.Sum256(b)
	}
	return out
}

// Mutation: open the store with load.Open in do, which publishes the files
// pack, and the store's bytes change.
func TestQueriesWriteNothing(t *testing.T) {
	dir := loaded(t)
	before := state(t, dir)
	if names := slices.Sorted(maps.Keys(before)); !slices.Equal(names, []string{load.StoreFile}) {
		t.Fatalf("the closed store left %v", names)
	}
	queries := [][]string{
		{"--json", "HDFC"}, {"--json", "Zebra Savings"}, {"--json", "HDFC Bank"}, {"--json", "ICICI"},
		{"--ask", "when does my passport expire", "--reference-date", "2026-11-27"}, {"--ask", "what is my balance"},
	}
	for i := range 20 {
		q := queries[i%len(queries)]
		if code, _, stderr := sifsCard(t, append([]string{"--data-dir", dir}, q...)...); code != 0 {
			t.Fatalf("%v: exit %d: %s", q, code, stderr)
		}
	}
	after := state(t, dir)
	if len(after) != len(before) || after[load.StoreFile] != before[load.StoreFile] {
		t.Errorf("20 queries changed the data dir: %d files, store hash %x, was %x", len(after), after[load.StoreFile], before[load.StoreFile])
	}
}

// Mutation: open with load.Open, which creates the store, and the missing
// store answers.
func TestMissingStoreFailsAndCreatesNothing(t *testing.T) {
	empty := t.TempDir()
	missing := filepath.Join(t.TempDir(), "none")
	for _, dir := range []string{empty, missing} {
		if code, _, stderr := sifsCard(t, "--data-dir", dir, "--json", "HDFC"); code != 2 || !strings.Contains(stderr, "no store") {
			t.Errorf("%s: exit %d: %s", dir, code, stderr)
		}
	}
	if entries, _ := os.ReadDir(empty); len(entries) != 0 {
		t.Errorf("the empty data dir now holds %v", entries)
	}
	if _, err := os.Stat(missing); !os.IsNotExist(err) {
		t.Errorf("the missing data dir was made: %v", err)
	}
}

func TestUsage(t *testing.T) {
	for _, args := range [][]string{
		{"--json", "HDFC"},
		{"--data-dir", "x"},
		{"--data-dir", "x", "--json", "HDFC", "--ask", "when does my passport expire"},
		{"--data-dir", "x", "--ask", "passport expiry", "--reference-date", "27-11-2026"},
	} {
		if code, _, _ := sifsCard(t, args...); code != 2 {
			t.Errorf("%v: exit %d, want 2", args, code)
		}
	}
}

// One JSON document with query_ms; nothing matched and an unanswered
// question both exit 0.
func TestAnswersExitZero(t *testing.T) {
	dir := loaded(t)
	code, stdout, stderr := sifsCard(t, "--data-dir", dir, "--json", "Zebra Savings")
	var found map[string]any
	if code != 0 || json.Unmarshal([]byte(stdout), &found) != nil {
		t.Fatalf("exit %d: %s%s", code, stdout, stderr)
	}
	if cards, ok := found["cards"].([]any); !ok || len(cards) != 0 || found["query_ms"] == nil {
		t.Errorf("no match: %s", stdout)
	}
	code, stdout, _ = sifsCard(t, "--data-dir", dir, "--ask", "what is my balance")
	if code != 0 || !strings.Contains(stdout, `"answer":"not-answered-yet"`) {
		t.Errorf("exit %d: %s", code, stdout)
	}
}
