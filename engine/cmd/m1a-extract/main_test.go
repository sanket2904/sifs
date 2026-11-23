package main

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// oddPages are inputs no real document looks like. Every one must give a
// run file, and the scorer's format check must accept it.
var oddPages = map[string][]string{
	"no-pages":       nil,
	"empty-page":     {""},
	"empty-pages":    {"", "", ""},
	"only-newlines":  {"\n\n\n\n"},
	"only-digits":    {"0472019538647201953864720195386123456789\n4242 4242 4242 4242\n1234 5678"},
	"only-colons":    {":\n::\n:::: ::\n：：\nBank:\nBank: ,\nBank: :\nAccount: :"},
	"devanagari-num": {"अवधि: ०४ मार्च २०२६ से २६ मार्च २०२६\nखाता संख्या: ३०१२३४५६७८९"},
	"full-width-num": {"期間: ２０２６年８月１日～２０２６年８月３１日\n口座番号: ７６５４３２１"},
	"arabic-indic":   {"Period: ٠١ Aug ٢٠٢٦ to ٣١ Aug ٢٠٢٦\nAccount: ٨٨٠٠١٢٣٤٠١"},
	"mixed-scripts":  {"Example Bank 見本銀行 नमूना बैंक Банк Ejemplo\nStatement 明細 विवरण\nPeriod: 01 Aug 2026 to 31 Aug 2026\nAccount: 4720195386 IFSC: ABCD0123456"},
	"long-line":      {strings.Repeat("Example Bank Account: 123456789 IFSC: ABCD0123456 ", 1000)},
	"long-word":      {strings.Repeat("A", 200000)},
	"zone-shapes":    {strings.Repeat("<", 44) + "\n" + strings.Repeat("<", 44) + "\nP" + strings.Repeat("A", 43) + "\n" + strings.Repeat("9", 44)},
	"passport-junk":  {"PASSPORT\nPassport No.: AB1234567\nSurname: \nGiven names: \nDate of expiry: 31 Feb 2030"},
	"crlf":           {"Example Bank\r\nStatement\r\nAccount: 123456789\r\nPeriod: 01 Aug 2026 to 31 Aug 2026\r\n"},
	"emoji-marks":    {"🏦 Bank 🏦\ń́ Bank\n& Bank\nBank &"},
}

func TestOddInputs(t *testing.T) {
	dir := t.TempDir()
	text := filepath.Join(dir, "text")
	if err := os.Mkdir(text, 0o755); err != nil {
		t.Fatal(err)
	}
	var ids []string
	for id, pages := range oddPages {
		ids = append(ids, id)
		doc := map[string]any{"id": id, "lang": "en", "pages": []map[string]any{}}
		for _, p := range pages {
			doc["pages"] = append(doc["pages"].([]map[string]any), map[string]any{"text": p, "scanned": id == "only-digits"})
		}
		writeJSON(t, filepath.Join(text, id+".json"), doc)
	}
	writeJSON(t, filepath.Join(text, "index.json"), map[string]any{"format": 1, "text_set": "text", "reference_date": "2026-11-22", "documents": ids})

	// each document alone too: a run where nothing is found must still be a run
	for _, set := range append([][]string{ids}, split(ids)...) {
		writeJSON(t, filepath.Join(text, "index.json"), map[string]any{"format": 1, "text_set": "text", "reference_date": "2026-11-22", "documents": set})
		out := filepath.Join(dir, "out.json")
		if err := run(text, "", "../../../sample/iban-registry.json", out, "go-m1a"); err != nil {
			t.Fatalf("%v: %v", set, err)
		}
		var o struct {
			Documents []json.RawMessage `json:"documents"`
		}
		raw, _ := os.ReadFile(out)
		if err := json.Unmarshal(raw, &o); err != nil || o.Documents == nil {
			t.Fatalf("%v: documents must be a list: %s", set, raw[:min(len(raw), 200)])
		}
		scorerAccepts(t, dir, out)
	}
}

func split(ids []string) [][]string {
	var out [][]string
	for _, id := range ids {
		out = append(out, []string{id})
	}
	return out
}

func writeJSON(t *testing.T, path string, v any) {
	t.Helper()
	raw, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, raw, 0o644); err != nil {
		t.Fatal(err)
	}
}

// scorerAccepts runs the scorer's own format check on the run file, with
// labels that hold no finding, when Node and the sample's tools are there.
func scorerAccepts(t *testing.T, sample, out string) {
	t.Helper()
	tools, err := filepath.Abs("../../../sample/tools")
	if err != nil {
		t.Fatal(err)
	}
	node, err := exec.LookPath("node")
	if err != nil {
		t.Skip("no node on PATH")
	}
	if _, err := os.Stat(filepath.Join(tools, "score.mjs")); err != nil {
		t.Skip("no scorer beside the module")
	}
	check := filepath.Join(sample, "check.mjs")
	src := `import { score, FormatError } from ` + quote("file://"+tools+"/score.mjs") + `;
import { readTextSet } from ` + quote("file://"+tools+"/text.mjs") + `;
import { readFileSync } from 'node:fs';
const [sample, out] = process.argv.slice(2);
const set = readTextSet('text', new URL('file://' + sample + '/'));
const empty = (id) => ({ id, orgs: [], not_orgs: [], identifiers: [], not_identifiers: [], links: [], not_links: [] });
const labels = { organisations: {}, documents: [...set.docs.keys()].map(empty), answers: { latest_statement: [], current_passport: [] } };
try {
	score(labels, set, JSON.parse(readFileSync(out, 'utf8')), { entries: [] });
} catch (e) {
	if (e instanceof FormatError || e instanceof SyntaxError) { console.error(e.message); process.exit(2); }
	throw e;
}
`
	if err := os.WriteFile(check, []byte(src), 0o644); err != nil {
		t.Fatal(err)
	}
	if msg, err := exec.Command(node, check, sample, out).CombinedOutput(); err != nil {
		t.Errorf("the scorer rejects the run file: %v\n%s", err, msg)
	}
}

func quote(s string) string { raw, _ := json.Marshal(s); return string(raw) }
