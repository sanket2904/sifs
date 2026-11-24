package main

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A made-up run of two documents; the second gives a tier format 3 does not
// have, which the files pack refuses.
const sampleRun = `{"format": 3, "extractor": "test", "directory": "off", "text_set": "text",
 "organisations": {"IBAN XX 0001": {"name": null, "directory": null}},
 "documents": [
  {"id": "good", "orgs": [], "identifiers": [{"id": "i1", "page": 1, "start": 0, "end": 4, "text": "XX01", "kind": "iban", "value": "XX01", "valid": true, "tier": "certain"}],
   "links": [{"identifier": "i1", "org": "IBAN XX 0001", "reasons": ["iban-bank-code"], "tier": "certain"}], "same_org": []},
  {"id": "bad", "orgs": [], "identifiers": [{"id": "i1", "page": 1, "start": 0, "end": 4, "text": "XX02", "kind": "iban", "value": "XX02", "valid": true, "tier": "certain"}],
   "links": [{"identifier": "i1", "org": "IBAN XX 0001", "reasons": ["iban-bank-code"], "tier": "maybe"}], "same_org": []}],
 "answers": {"latest_statement": [], "current_passport": []}}`

// Mutation: return 0 from do when a record was turned down, and the exit
// code says nothing went wrong.
func TestTurnedDownIsPrintedAndFails(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "run.json")
	if err := os.WriteFile(path, []byte(sampleRun), 0o644); err != nil {
		t.Fatal(err)
	}
	var stdout, stderr bytes.Buffer
	code := sifsLoad(t, []string{"--run", path, "--data-dir", filepath.Join(dir, "data")}, &stdout, &stderr)
	if code != 1 {
		t.Fatalf("exit %d, want 1; stderr %s", code, stderr.String())
	}
	var out loaded
	if err := json.Unmarshal(stdout.Bytes(), &out); err != nil {
		t.Fatalf("%v: %s", err, stdout.String())
	}
	if len(out.TurnedDown) != 1 || out.TurnedDown[0].Document != "bad" || !strings.Contains(out.TurnedDown[0].Message, "maybe") {
		t.Fatalf("turned down: %+v", out.TurnedDown)
	}
	if out.Documents.Written != 1 || out.Things["Document"] != 1 {
		t.Fatalf("the good document did not load: %s", stdout.String())
	}

	stdout.Reset()
	if code := sifsLoad(t, []string{"--data-dir", filepath.Join(dir, "data"), "--counts"}, &stdout, &stderr); code != 0 {
		t.Fatalf("--counts exit %d: %s", code, stderr.String())
	}
	if !strings.Contains(stdout.String(), `"Document":1`) {
		t.Fatalf("--counts: %s", stdout.String())
	}
}

func sifsLoad(t *testing.T, args []string, stdout, stderr *bytes.Buffer) int {
	t.Helper()
	code := runMain(args, stdout, stderr)
	t.Logf("sifs-load %s: exit %d: %s%s", strings.Join(args, " "), code, stdout.String(), stderr.String())
	return code
}

// A made-up run with a mention no organisation settles and one answer:
// both are left out by design, counted, and the exit code is 0.
const leftOutRun = `{"format": 3, "extractor": "test", "directory": "off", "text_set": "text",
 "organisations": {"IBAN XX 0001": {"name": null, "directory": null}},
 "documents": [
  {"id": "good", "orgs": [{"page": 1, "start": 10, "end": 22, "text": "Example Club", "org": null, "settled_by": null}],
   "identifiers": [{"id": "i1", "page": 1, "start": 0, "end": 4, "text": "XX01", "kind": "iban", "value": "XX01", "valid": true, "tier": "certain"}],
   "links": [{"identifier": "i1", "org": "IBAN XX 0001", "reasons": ["iban-bank-code"], "tier": "certain"}], "same_org": []}],
 "answers": {"latest_statement": [{"account": "XX01", "document": "good"}], "current_passport": []}}`

// Mutation: leave LeftOut out of the line do prints, and left_out reads 0.
func TestLeftOutIsPrintedAndPasses(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "run.json")
	if err := os.WriteFile(path, []byte(leftOutRun), 0o644); err != nil {
		t.Fatal(err)
	}
	var stdout, stderr bytes.Buffer
	if code := sifsLoad(t, []string{"--run", path, "--data-dir", filepath.Join(dir, "data")}, &stdout, &stderr); code != 0 {
		t.Fatalf("exit %d, want 0; stderr %s", code, stderr.String())
	}
	if want := `"left_out":{"unused_organisations":0,"mentions_without_organisation":1,"answers":1}`; !strings.Contains(stdout.String(), want) {
		t.Fatalf("no %s in %s", want, stdout.String())
	}
	var out loaded
	if err := json.Unmarshal(stdout.Bytes(), &out); err != nil {
		t.Fatalf("%v: %s", err, stdout.String())
	}
	if out.Documents.Written != 1 || out.Links["HeldAt"] != 1 || out.Links["Shows"] != 1 || out.Links["Mentions"] != 0 {
		t.Fatalf("the rest of the run did not load: %s", stdout.String())
	}
}
