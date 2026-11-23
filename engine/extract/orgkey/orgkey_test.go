package orgkey

import (
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"testing"
)

func TestCodeKeys(t *testing.T) {
	for _, c := range []struct {
		got  Key
		want string
	}{
		{IFSC("HDFC0SAMPL1"), "IFSC HDFC"},
		{BIC("COBADEFFXXX"), "BIC COBADEFF"},
		{IBAN("DE", "37040044"), "IBAN DE 37040044"},
	} {
		if c.got.String() != c.want || c.got.IsName() {
			t.Errorf("got %q (name %v), want code key %q", c.got, c.got.IsName(), c.want)
		}
	}
}

func TestNameIsTheLongestMatchKey(t *testing.T) {
	for _, c := range []struct {
		texts []string
		want  string
	}{
		{[]string{"HDFC Bank Ltd"}, "name:d:hdfcbank"},
		{[]string{"HDFC", "HDFC Bank"}, "name:d:hdfcbank"},
		{[]string{"HDFC Bank", "HDFC"}, "name:d:hdfcbank"},
		{[]string{"Zeta Bank", "Alfa Bank"}, "name:d:alfabank"},
		{[]string{"Alfa Bank", "Zeta Bank"}, "name:d:alfabank"},
		{[]string{"HDFC Ltd", "Housing Development Finance Corporation Limited"}, "name:d:housingdevelopmentfinancecorporation"},
		{[]string{"Ltd", "SBI"}, "name:d:sbi"},
	} {
		k, ok := Name("d", c.texts...)
		if !ok || k.String() != c.want || !k.IsName() || k.Part() != strings.TrimPrefix(c.want, "name:d:") {
			t.Errorf("Name(%q) = %q, %v; want %q", c.texts, k, ok, c.want)
		}
	}
	if k, ok := Name("d", "The Ltd", "&"); ok {
		t.Errorf("a text with no match key spells no key: %q", k)
	}
	a, _ := Name("a", "Riverside Bank")
	b, _ := Name("b", "Riverside Bank")
	if a == b || a.String() == b.String() {
		t.Errorf("one printed name in two documents has two keys: %q, %q", a, b)
	}
}

func TestMatchKey(t *testing.T) {
	for _, c := range []struct{ in, want string }{
		{"Example Bank Ltd", "examplebank"},
		{"EXAMPLE BANK", "examplebank"},
		{"examplebank", "examplebank"},
		{"The Bank of Example plc", "bankofexample"},
		{"見本 ABC 銀行", "見本abc銀行"},
		{"株式会社見本ＡＢＣ銀行", "見本abc銀行"},
		{"桜銀行株式会社Ltd", "桜銀行"},
		{"Alpha & Beta Bank", "alphabetabank"},
	} {
		if got := MatchKey(c.in); got != c.want {
			t.Errorf("MatchKey(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}

// TestKeysAgreeWithTheScorer spells, with this package and with the
// scorer's own functions (sample/tools/lib/key.mjs and tiers.mjs), the match
// key of every name the sample's labels and directory hold, and the
// document key of every labelled organisation in every document from the
// texts of its mentions there, as the reference output spells it.
func TestKeysAgreeWithTheScorer(t *testing.T) {
	sample, err := filepath.Abs("../../../sample")
	if err != nil {
		t.Fatal(err)
	}
	node, err := exec.LookPath("node")
	if err != nil {
		t.Skip("no node on PATH")
	}
	if _, err := os.Stat(filepath.Join(sample, "tools/lib/tiers.mjs")); err != nil {
		t.Skip("no scorer beside the module")
	}
	var labels struct {
		Organisations map[string]struct {
			Name  string   `json:"name"`
			Names []string `json:"names"`
		} `json:"organisations"`
		Documents []struct {
			ID   string `json:"id"`
			Orgs []struct {
				Text     string `json:"text"`
				Resolved *struct {
					Org string `json:"org"`
				} `json:"resolved_by_document"`
			} `json:"orgs"`
			NotOrgs []struct {
				Text string `json:"text"`
			} `json:"not_orgs"`
		} `json:"documents"`
	}
	readJSON(t, filepath.Join(sample, "labels.json"), &labels)
	var dir struct {
		Entries []struct {
			Names []struct {
				Text string `json:"text"`
			} `json:"names"`
		} `json:"entries"`
	}
	readJSON(t, filepath.Join(sample, "directory/banks.json"), &dir)

	type group struct {
		Doc   string   `json:"doc"`
		Texts []string `json:"texts"`
	}
	var in struct {
		Names  []string `json:"names"`
		Groups []group  `json:"groups"`
	}
	in.Names = []string{"桜銀行株式会社Ltd", "株式会社見本ＡＢＣ銀行"}
	for _, o := range labels.Organisations {
		in.Names = append(in.Names, o.Name)
		in.Names = append(in.Names, o.Names...)
	}
	for _, d := range labels.Documents {
		byOrg := map[string][]string{}
		for _, m := range d.Orgs {
			in.Names = append(in.Names, m.Text)
			if m.Resolved != nil && m.Resolved.Org != "" {
				byOrg[m.Resolved.Org] = append(byOrg[m.Resolved.Org], m.Text)
			}
		}
		for _, m := range d.NotOrgs {
			in.Names = append(in.Names, m.Text)
		}
		orgs := make([]string, 0, len(byOrg))
		for o := range byOrg {
			orgs = append(orgs, o)
		}
		sort.Strings(orgs)
		for _, o := range orgs {
			in.Groups = append(in.Groups, group{d.ID, byOrg[o]})
		}
	}
	for _, e := range dir.Entries {
		for _, n := range e.Names {
			in.Names = append(in.Names, n.Text)
		}
	}
	if len(in.Names) < 1000 || len(in.Groups) < 50 {
		t.Fatalf("only %d names and %d groups", len(in.Names), len(in.Groups))
	}

	script := filepath.Join(t.TempDir(), "keys.mjs")
	src := `import { matchKey } from ` + quote("file://"+sample+"/tools/lib/key.mjs") + `;
import { nameKey } from ` + quote("file://"+sample+"/tools/lib/tiers.mjs") + `;
import { readFileSync } from 'node:fs';
const { names, groups } = JSON.parse(readFileSync(0, 'utf8'));
console.log(JSON.stringify({ names: names.map(matchKey), groups: groups.map((g) => nameKey(g.doc, g.texts)) }));
`
	if err := os.WriteFile(script, []byte(src), 0o644); err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(in)
	cmd := exec.Command(node, script)
	cmd.Stdin = strings.NewReader(string(raw))
	got, err := cmd.Output()
	if err != nil {
		t.Fatalf("node: %v", err)
	}
	var theirs struct {
		Names  []string `json:"names"`
		Groups []string `json:"groups"`
	}
	if err := json.Unmarshal(got, &theirs); err != nil {
		t.Fatal(err)
	}
	for i, n := range in.Names {
		if ours := MatchKey(n); ours != theirs.Names[i] {
			t.Errorf("match key of %q: ours %q, the scorer's %q", n, ours, theirs.Names[i])
		}
	}
	for i, g := range in.Groups {
		ours, _ := Name(g.Doc, g.Texts...)
		if ours.String() != theirs.Groups[i] {
			t.Errorf("key of %q in %s: ours %q, the scorer's %q", g.Texts, g.Doc, ours, theirs.Groups[i])
		}
	}
	t.Logf("%d names and %d document keys agree with the scorer", len(in.Names), len(in.Groups))
}

func readJSON(t *testing.T, path string, v any) {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Skipf("no %s beside the module", path)
	}
	if err := json.Unmarshal(raw, v); err != nil {
		t.Fatal(err)
	}
}

func quote(s string) string { raw, _ := json.Marshal(s); return string(raw) }
