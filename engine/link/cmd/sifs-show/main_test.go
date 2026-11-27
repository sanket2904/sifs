package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"maps"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/link/load"
)

func sifsShow(t *testing.T, args ...string) (int, string, string) {
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

// answer is a document the tool printed, without its timing.
func answer(t *testing.T, stdout string) map[string]any {
	t.Helper()
	if strings.Count(stdout, "\n") != 1 || !strings.HasSuffix(stdout, "\n") {
		t.Fatalf("want one JSON document on one line, got %q", stdout)
	}
	var doc map[string]any
	if err := json.Unmarshal([]byte(stdout), &doc); err != nil {
		t.Fatal(err)
	}
	if _, ok := doc["query_ms"].(float64); !ok {
		t.Fatalf("no query_ms in %s", stdout)
	}
	delete(doc, "query_ms")
	return doc
}

var reads = [][]string{
	{"--kinds"},
	{"--find", "Commerzbank"},
	{"--find", "Eriksson"},
	{"--find", "NL91ABNA0417164300"},
	{"--find", "Zanzibar"},
	{"--page", "org:BIC COBADEFF"},
	{"--page", "identifier:iban:DE89370400440532013000"},
	{"--page", "holder:passport-bad-mrz", "--reveal"},
	{"--page", "statement:natwest-statement"},
	{"--find", "DE89370400440532013000", "--reveal"},
}

// Mutation: open the store with load.Open in do, which publishes the files
// pack, and the store's bytes change.
func TestReadsWriteNothing(t *testing.T) {
	dir := loaded(t)
	before := state(t, dir)
	if names := slices.Sorted(maps.Keys(before)); !slices.Equal(names, []string{load.StoreFile}) {
		t.Fatalf("the closed store left %v", names)
	}
	for i := range 20 {
		args := append([]string{"--data-dir", dir}, reads[i%len(reads)]...)
		if code, _, stderr := sifsShow(t, args...); code != 0 {
			t.Fatalf("%v: exit %d: %s", args, code, stderr)
		}
	}
	if after := state(t, dir); !maps.Equal(before, after) {
		t.Fatalf("20 reads changed the data dir: %d files before, %d after", len(before), len(after))
	}
}

var timing = regexp.MustCompile(`,"query_ms":[0-9]+}\n$`)

// Mutation: drop the sort of a page's link groups, and two reads of the
// same store differ.
func TestSameBytes(t *testing.T) {
	dir := loaded(t)
	for _, read := range reads {
		args := append([]string{"--data-dir", dir}, read...)
		_, first, _ := sifsShow(t, args...)
		for range 3 {
			_, again, _ := sifsShow(t, args...)
			if a, b := timing.ReplaceAllString(first, "}"), timing.ReplaceAllString(again, "}"); a != b || a == first {
				t.Fatalf("%v gave different bytes on the same store", read)
			}
		}
	}
}

func TestMaskedUnlessRevealed(t *testing.T) {
	dir := loaded(t)
	const iban = "DE89370400440532013000"
	_, masked, _ := sifsShow(t, "--data-dir", dir, "--find", iban)
	if strings.Contains(masked, iban) || !strings.Contains(masked, "3000") {
		t.Fatalf("the IBAN is not masked: %s", masked)
	}
	_, raw, _ := sifsShow(t, "--data-dir", dir, "--find", iban, "--reveal")
	if !strings.Contains(raw, iban) {
		t.Fatalf("--reveal does not give the IBAN: %s", raw)
	}
}

// Mutation: decide by the value of --find instead of whether it was given,
// and an empty find is a usage error.
func TestEmptyFind(t *testing.T) {
	dir := loaded(t)
	code, stdout, stderr := sifsShow(t, "--data-dir", dir, "--find", "")
	if code != 0 || stderr != "" {
		t.Fatalf("exit %d: %s", code, stderr)
	}
	if groups, ok := answer(t, stdout)["groups"].([]any); !ok || len(groups) != 0 {
		t.Fatalf("want empty groups, got %s", stdout)
	}
}

func TestCannotRun(t *testing.T) {
	dir := loaded(t)
	missing := filepath.Join(t.TempDir(), "nothing-here")
	for _, c := range []struct {
		args []string
		says string
	}{
		{nil, "usage"},
		{[]string{"--kinds"}, "usage"},
		{[]string{"--data-dir", dir}, "usage"},
		{[]string{"--data-dir", dir, "--kinds", "--find", "x"}, "usage"},
		{[]string{"--data-dir", dir, "--find", "x", "--page", "y"}, "usage"},
		{[]string{"--data-dir", dir, "--kinds", "extra"}, "usage"},
		{[]string{"--data-dir", dir, "--colour"}, "flag provided but not defined"},
		{[]string{"--data-dir", missing, "--kinds"}, "no store"},
		{[]string{"--data-dir", dir, "--page", "org:NO SUCH BANK"}, "no such thing"},
	} {
		code, stdout, stderr := sifsShow(t, c.args...)
		if code != 2 || stdout != "" || !strings.Contains(stderr, c.says) {
			t.Errorf("%v: exit %d, stdout %q, stderr %q; want exit 2, no stdout, %q", c.args, code, stdout, stderr, c.says)
		}
	}
	if _, err := os.Stat(missing); !os.IsNotExist(err) {
		t.Fatalf("a read of a missing store made %s", missing)
	}
}
