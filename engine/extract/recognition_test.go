package extract

import (
	"encoding/json"
	"fmt"
	"math/big"
	"os"
	"sort"
	"testing"
)

func TestLegalFormTailRequiresWholeToken(t *testing.T) {
	for _, c := range []struct {
		line string
		want string
	}{
		{"紫凪銀行 株式会社 AG81 支店", "紫凪銀行 株式会社"},
		{"紫凪銀行 Ltd2026 支店", "紫凪銀行"},
		{"紫凪銀行 株式会社 AG支店", "紫凪銀行 株式会社"},
		{"紫凪銀行 株式会社 AG 支店", "紫凪銀行 株式会社 AG"},
		{"紫凪銀行株式会社Ltd Branch services", "紫凪銀行株式会社Ltd"},
		{"紫凪銀行 Ltd 2026 customer guide", "紫凪銀行 Ltd"},
		{"紫凪銀行 株式会社Ltd.", "紫凪銀行 株式会社Ltd"},
	} {
		_, e := runOne(t, doc("d", c.line), nil)
		eq(t, c.line, names(e), []string{c.want})
		if len(e.Orgs) == 1 && (e.Orgs[0].Org == nil || *e.Orgs[0].Org != "name:d:紫凪銀行") {
			t.Errorf("%q: key %s, want name:d:紫凪銀行", c.line, orgOf(e.Orgs[0]))
		}
	}
}

func makeTestIBAN(cc string, length int) string {
	const digits = "12345678901234567890123456789012"
	bban := digits[:length-4]
	rearranged := bban + cc + "00"
	var expanded string
	for i := 0; i < len(rearranged); i++ {
		ch := rearranged[i]
		if ch >= 'A' && ch <= 'Z' {
			expanded += fmt.Sprintf("%d", int(ch-'A')+10)
		} else {
			expanded += string(ch)
		}
	}
	n, _ := new(big.Int).SetString(expanded, 10)
	rem := new(big.Int).Mod(n, big.NewInt(97)).Int64()
	return fmt.Sprintf("%s%02d%s", cc, 98-rem, bban)
}

func TestEveryRegistryCountryIBANIsRecognised(t *testing.T) {
	raw, err := os.ReadFile("../../sample/iban-registry.json")
	if err != nil {
		t.Fatal(err)
	}
	var f struct {
		Countries map[string]struct {
			Length int    `json:"length"`
			Bank   string `json:"bank_identifier"`
		} `json:"countries"`
	}
	if err := json.Unmarshal(raw, &f); err != nil {
		t.Fatal(err)
	}
	if len(f.Countries) == 0 {
		t.Fatal("empty registry")
	}
	var codes []string
	for cc := range f.Countries {
		codes = append(codes, cc)
	}
	sort.Strings(codes)
	for _, cc := range codes {
		c := f.Countries[cc]
		var first, last int
		if n, _ := fmt.Sscanf(c.Bank, "%d-%d", &first, &last); n != 2 {
			t.Fatalf("%s: bank_identifier %q", cc, c.Bank)
		}
		iban := makeTestIBAN(cc, c.Length)
		wantKey := "IBAN " + cc + " " + iban[4+first-1:4+last]
		_, e := runOne(t, doc("d", "Beneficiary IBAN: "+iban), nil)
		if len(e.Identifiers) != 1 {
			t.Errorf("%s (%s): got %d identifiers, want 1", cc, iban, len(e.Identifiers))
			continue
		}
		id := e.Identifiers[0]
		if id.Kind != KindIBAN || id.Value != iban || id.Valid == nil || !*id.Valid || id.Tier != Certain {
			t.Errorf("%s (%s): got %+v", cc, iban, id)
		}
		if len(e.Links) != 1 || e.Links[0].Org != wantKey || e.Links[0].Tier != Certain {
			t.Errorf("%s (%s): links %+v, want %s certain", cc, iban, e.Links, wantKey)
		}
	}
	unregistered := makeTestIBAN("ZZ", 20)
	if _, e := runOne(t, doc("d", "Beneficiary IBAN: "+unregistered), nil); len(e.Identifiers) != 0 {
		t.Errorf("unregistered %s: got %+v, want none", unregistered, e.Identifiers)
	}
}

func TestHalfWidthKatakanaVoicedMarksStayInSpan(t *testing.T) {
	for _, c := range []struct {
		line string
		want string
		key  string
	}{
		{"ｶﾞﾗｽ銀行", "ｶﾞﾗｽ銀行", "name:d:ガラス銀行"},
		{"ﾊﾟﾙﾅ銀行株式会社", "ﾊﾟﾙﾅ銀行株式会社", "name:d:パルナ銀行"},
		{"受取銀行: ｶﾞﾗｽ銀行", "ｶﾞﾗｽ銀行", "name:d:ガラス銀行"},
		{"ご連絡はﾊﾟﾙﾅ銀行株式会社まで", "ﾊﾟﾙﾅ銀行株式会社", "name:d:パルナ銀行"},
		{"ﾐｽﾞ銀行", "ﾐｽﾞ銀行", "name:d:ミズ銀行"},
		{"ｶﾞﾗｽ銀行 株式会社 AG81 支店", "ｶﾞﾗｽ銀行 株式会社", "name:d:ガラス銀行"},
	} {
		_, e := runOne(t, doc("d", c.line), nil)
		eq(t, c.line, names(e), []string{c.want})
		if len(e.Orgs) == 1 && (e.Orgs[0].Org == nil || *e.Orgs[0].Org != c.key) {
			t.Errorf("%q: key %s, want %s", c.line, orgOf(e.Orgs[0]), c.key)
		}
	}
}
