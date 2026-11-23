package orgkey

import (
	"encoding/json"
	"flag"
	"math/rand/v2"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"unicode"

	"golang.org/x/text/unicode/norm"
)

var keySeed = flag.Uint64("keyseed", 20261123, "seed of the names TestGeneratedKeysAgreeWithTheScorer makes")

// Words of eleven scripts, made up or common nouns, written composed. The
// generator adds the legal forms the key drops, dotted legal forms, 株式会社,
// digits of several scripts and separators.
var scriptWords = map[string][]string{
	"latin":      {"Valério", "Ribeirão", "Ñandú", "Øresund", "Łódź", "Straße", "Ærø", "Crème", "Göteborg", "Šibenik", "Đông", "Bank", "Banco", "Banque", "Trust", "ﬁnance"},
	"turkish":    {"İzmir", "Iğdır", "ŞEKER", "İŞ", "ıslak", "Katılım", "Bankası"},
	"greek":      {"Λίμνη", "Τράπεζα", "ΟΔΟΣ", "Ελλάς", "Πίστις", "ΣΙΣΥΦΟΣ", "ΐδα", "ΚΑΙ"},
	"cyrillic":   {"Банк", "Йошкар", "Ёлка", "Сбер", "Їжак", "ЩИТ"},
	"korean":     {"바다", "은행", "한빛", "국민", "뱅크"},
	"japanese":   {"ギンガ", "ぎんこう", "パン", "ヴァ", "白浜", "銀行", "ポート"},
	"chinese":    {"工商", "滙豐", "銀行", "興業"},
	"arabic":     {"بنك", "نجم", "النيل", "مصرف", "ضفاف", "لا"},
	"hebrew":     {"בנק", "שָׁלוֹם", "יִשְׂרָאֵל"},
	"devanagari": {"बैंक", "क़िरण", "तट", "ज़ेड", "फ़र्म"},
	"thai":       {"ธนาคาร", "ลำธาร", "ทอง", "คำ"},
	"vietnamese": {"Ngân", "hàng", "Bến", "Mây", "Việt", "Ở"},
	"georgian":   {"ბანკი", "თბილისი"},
}

var (
	extraWords = []string{"the", "THE", "Ltd", "LTD", "Limited", "LIMITED", "plc", "PLC", "Inc", "AG", "SA", "NV", "sa", "nv",
		"N.V.", "S.A.", "A.G.", "P.L.C.", "Ltd.", "Inc.", "株式会社", "㈱", "24", "٢٤", "२४", "２４", "²", "Ⅻ"}
	separators = []string{" ", "  ", "-", "'", "’", "·", "　", "&", ",", ".", "/", " ", "\t", "・", ""}
)

// compatOf maps a decomposed text of one to three scalar values to the
// characters whose compatibility decomposition it is, from the blocks that
// hold such characters: half- and full-width forms, presentation forms,
// ligatures, letterlike and enclosed characters, compatibility jamo, and the
// like.
func compatOf() map[string][]rune {
	blocks := [][2]rune{{0x00A0, 0x024F}, {0x02B0, 0x02FF}, {0x0370, 0x03FF}, {0x0587, 0x0587}, {0x0E33, 0x0E33}, {0x1D2C, 0x1DBF},
		{0x2000, 0x2FDF}, {0x3000, 0x33FF}, {0xF900, 0xFAFF}, {0xFB00, 0xFDFF}, {0xFE30, 0xFFEF}, {0x1D400, 0x1D7FF}, {0x1F100, 0x1F1FF}}
	out := map[string][]rune{}
	for _, b := range blocks {
		for r := b[0]; r <= b[1]; r++ {
			s := string(r)
			d := norm.NFKD.String(s)
			if d == norm.NFD.String(s) || len([]rune(d)) > 3 {
				continue
			}
			out[d] = append(out[d], r)
		}
	}
	return out
}

// compatForm writes a decomposed text with some of its characters, or runs
// of up to three, replaced by a compatibility character that decomposes to
// them.
func compatForm(rng *rand.Rand, pre map[string][]rune, s string) string {
	rs := []rune(norm.NFD.String(s))
	var b strings.Builder
	for i := 0; i < len(rs); {
		done := false
		for n := 3; n >= 1 && !done; n-- {
			if i+n > len(rs) {
				continue
			}
			if cs := pre[string(rs[i:i+n])]; len(cs) > 0 && rng.IntN(10) < 7 {
				b.WriteRune(cs[rng.IntN(len(cs))])
				i += n
				done = true
			}
		}
		if !done {
			b.WriteRune(rs[i])
			i++
		}
	}
	return b.String()
}

func generatedNames(rng *rand.Rand, n int) []string {
	scripts := make([]string, 0, len(scriptWords))
	for s := range scriptWords {
		scripts = append(scripts, s)
	}
	sortStrings(scripts)
	pick := func(xs []string) string { return xs[rng.IntN(len(xs))] }
	out := make([]string, n)
	for i := range out {
		var b strings.Builder
		for w := 0; w < 1+rng.IntN(5); w++ {
			if w > 0 {
				b.WriteString(pick(separators))
			}
			word := pick(scriptWords[scripts[rng.IntN(len(scripts))]])
			if rng.IntN(4) == 0 {
				word = pick(extraWords)
			}
			switch rng.IntN(4) {
			case 0:
				word = Upper(word)
			case 1:
				word = strings.ToLower(word)
			}
			b.WriteString(word)
		}
		out[i] = b.String()
	}
	return out
}

func sortStrings(xs []string) {
	for i := 1; i < len(xs); i++ {
		for j := i; j > 0 && xs[j] < xs[j-1]; j-- {
			xs[j], xs[j-1] = xs[j-1], xs[j]
		}
	}
}

// TestGeneratedKeysAgreeWithTheScorer spells, with this package and with
// the scorer's own functions, the match key and the upper case of 5,000
// generated names over eleven scripts, each written composed, decomposed
// and with compatibility characters, and the document key of the three
// forms together. Any disagreement fails. Run with -keyseed to draw other
// names.
func TestGeneratedKeysAgreeWithTheScorer(t *testing.T) {
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
	t.Logf("seed %d (-keyseed), Unicode %s in Go, normaliser tables %s", *keySeed, unicode.Version, norm.Version)
	rng := rand.New(rand.NewPCG(*keySeed, 0))
	pre := compatOf()
	type group struct {
		Doc   string   `json:"doc"`
		Texts []string `json:"texts"`
	}
	var in struct {
		Names  []string `json:"names"`
		Groups []group  `json:"groups"`
	}
	for _, n := range generatedNames(rng, 5000) {
		forms := []string{norm.NFC.String(n), norm.NFD.String(n), compatForm(rng, pre, n)}
		in.Names = append(in.Names, forms...)
		in.Groups = append(in.Groups, group{"d", forms})
	}

	script := filepath.Join(t.TempDir(), "keys.mjs")
	src := `import { matchKey } from ` + quote("file://"+sample+"/tools/lib/key.mjs") + `;
import { nameKey } from ` + quote("file://"+sample+"/tools/lib/tiers.mjs") + `;
import { readFileSync } from 'node:fs';
const { names, groups } = JSON.parse(readFileSync(0, 'utf8'));
console.log(JSON.stringify({
	keys: names.map(matchKey),
	upper: names.map((n) => n.normalize('NFKC').toUpperCase()),
	groups: groups.map((g) => nameKey(g.doc, g.texts))
}));
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
		Keys   []string `json:"keys"`
		Upper  []string `json:"upper"`
		Groups []string `json:"groups"`
	}
	if err := json.Unmarshal(got, &theirs); err != nil {
		t.Fatal(err)
	}
	bad := 0
	for i, n := range in.Names {
		if ours := MatchKey(n); ours != theirs.Keys[i] {
			bad++
			t.Errorf("match key of %+q: ours %+q, the scorer's %+q", n, ours, theirs.Keys[i])
		}
		if ours := Upper(Fold(n)); ours != theirs.Upper[i] {
			bad++
			t.Errorf("upper case of %+q: ours %+q, the scorer's %+q", n, ours, theirs.Upper[i])
		}
	}
	for i, g := range in.Groups {
		ours, ok := Name(g.Doc, g.Texts...)
		if want := theirs.Groups[i]; ok && ours.String() != want || !ok && want != "name:d:" {
			bad++
			t.Errorf("key of %+q: ours %q (%v), the scorer's %q", g.Texts, ours, ok, want)
		}
	}
	if bad > 0 {
		t.Fatalf("seed %d: %d disagreements with the scorer", *keySeed, bad)
	}
	scripts := map[string]bool{}
	for _, n := range in.Names {
		for _, r := range n {
			for name, s := range unicode.Scripts {
				if name != "Common" && name != "Inherited" && unicode.Is(s, r) {
					scripts[name] = true
				}
			}
		}
	}
	if len(in.Names) < 15000 || len(scripts) < 10 {
		t.Fatalf("only %d names over %d scripts", len(in.Names), len(scripts))
	}
	t.Logf("seed %d: %d names (%d generated, each in three forms) over %d scripts agree with the scorer", *keySeed, len(in.Names), len(in.Groups), len(scripts))
}
