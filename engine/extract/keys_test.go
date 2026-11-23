package extract

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"testing"
)

// A key is spelt from every mention on it, whichever was read first: "NW
// N.V." is the initials of "T Q Limited" with a legal form the key keeps,
// so its match key, tqnv, is the longer, and every row on the key,
// the statement's issuer too, takes it.
func TestKeyIsSpeltFromEveryMentionOnIt(t *testing.T) {
	for _, d := range []Document{
		doc("d", "T Q Limited", "Account statement", "Account number: 111122223333", "Period: 01 Aug 2026 to 31 Aug 2026", "Questions: TQ N.V. account desk"),
		doc("d", "TQ N.V.", "Account statement", "Account number: 111122223333", "Period: 01 Aug 2026 to 31 Aug 2026", "Issued for T Q Limited"),
	} {
		o, e := runOne(t, d, nil)
		for _, m := range e.Orgs {
			if m.Org == nil || *m.Org != "name:d:tqnv" {
				t.Errorf("%q: mention %q on %s, want name:d:tqnv", d.ID, m.Text, orgOf(m))
			}
		}
		if e.Statement == nil || e.Statement.Issuer != "name:d:tqnv" {
			t.Errorf("issuer: %+v", e.Statement)
		}
		eq(t, "links", linksOf(o, e), []string{"111122223333 -> TQ N.V.: statement-header"})
	}
}

func orgOf(m OrgOut) string {
	if m.Org == nil {
		return "no key"
	}
	return *m.Org
}

// ibanKeys spells, from sample/iban-registry.json itself, the key FORMAT.md
// gives an IBAN: its country and the bank identifier at the table's place.
func ibanKeys(t *testing.T) func(string) string {
	t.Helper()
	raw, err := os.ReadFile("../../sample/iban-registry.json")
	if err != nil {
		t.Fatal(err)
	}
	var f struct {
		Countries map[string]struct {
			Bank string `json:"bank_identifier"`
		} `json:"countries"`
	}
	if err := json.Unmarshal(raw, &f); err != nil {
		t.Fatal(err)
	}
	return func(iban string) string {
		var from, to int
		c, ok := f.Countries[iban[:2]]
		if !ok {
			return ""
		}
		if n, _ := fmt.Sscanf(c.Bank, "%d-%d", &from, &to); n != 2 {
			t.Fatalf("%s: bank_identifier %q", iban[:2], c.Bank)
		}
		return "IBAN " + iban[:2] + " " + iban[4+from-1:4+to]
	}
}

// Every IBAN sits on the key the registry gives it, and only there, in the
// sample's four runs and in pages where a code of another bank is printed
// beside it: a correspondent's BIC, a beneficiary bank's name.
func TestIBANSitsOnItsRegistryKey(t *testing.T) {
	key := ibanKeys(t)
	check := func(what string, o Output) int {
		n := 0
		for _, d := range o.Documents {
			ident := map[string]IdentOut{}
			for _, i := range d.Identifiers {
				ident[i.ID] = i
			}
			own := map[string]bool{}
			for _, l := range d.Links {
				i := ident[l.Identifier]
				if i.Kind != KindIBAN {
					continue
				}
				n++
				if l.Org != key(i.Value) {
					t.Errorf("%s %s: IBAN %s on %s, want %s", what, d.ID, i.Value, l.Org, key(i.Value))
				}
				own[i.ID] = true
			}
			for _, i := range d.Identifiers {
				if i.Kind == KindIBAN && *i.Valid && !own[i.ID] && o.Directory == "off" {
					t.Errorf("%s %s: IBAN %s is on no key", what, d.ID, i.Value)
				}
			}
		}
		return n
	}
	pages := []Document{
		doc("corr", "Transfer advice", "Beneficiary bank: Banco Exemplo Atlantico", "Beneficiary IBAN: PT13 0099 0002 3141 5926 5358 9", "Correspondent BIC: TAGOPT2L"),
		doc("corr-row", "Transfer advice", "Beneficiary IBAN: PT13 0099 0002 3141 5926 5358 9  Correspondent BIC: TAGOPT2L"),
		doc("de", "Zahlung", "IBAN: DE89 3704 0044 0532 0130 00", "BIC: COBADEFFXXX"),
		doc("es", "Transfer", "IBAN: ES91 2100 0418 4502 0005 1332", "BIC: CAIXESBBXXX"),
	}
	named := entry("Q8", "Tago Bank", "tagobank", "")
	named.Codes.BIC = []string{"TAGOPT2LXXX"}
	ibans := 0
	for _, dir := range []*Directory{nil, NewDirectory([]DirEntry{named})} {
		ibans += check("pages", Run(pages, dir, registry, ref))
	}
	for _, set := range []string{"text", "text-ocr"} {
		docs, day, dir := loadSample(t, set)
		for _, d := range []*Directory{nil, dir} {
			ibans += check(set, Run(docs, d, registry, day))
		}
	}
	if ibans < 20 {
		t.Errorf("only %d IBAN links checked", ibans)
	}

	// what may still tie the IBAN to the BIC beside it is a likely row
	_, e := runOne(t, pages[0], nil)
	eq(t, "same_org", joinsOf(e), []string{"BIC TAGOPT2L = IBAN PT 0099: printed-together likely"})
}

// An insurer's renewal sheet prints its brand, which the directory holds
// as a short name of a bank, and the bank's BIC, which the directory holds
// too. No row says for certain that the brand is the bank, and the brand's
// card claims no directory identity.
func TestAPrintedNameIsNeverTheDirectorysForCertain(t *testing.T) {
	bank := entry("Q900001", "Koru Bank", "korubank", "")
	bank.Names = append(bank.Names, struct {
		Text string `json:"text"`
		Key  string `json:"key"`
	}{"KORU", "koru"})
	bank.Codes.BIC = []string{"KORUTR2AXXX"}
	dir := NewDirectory([]DirEntry{bank})
	for _, d := range []Document{
		doc("cover", "Insurance renewal", "KORU", "Underwritten by Koru Sigorta", "Premium to collecting bank", "BIC: KORUTR2A"),
		doc("advice", "Koru Bank", "Payment advice", "BIC: KORUTR2AXXX"),
	} {
		o, e := runOne(t, d, dir)
		for _, j := range e.SameOrg {
			if j.Tier == Certain {
				t.Errorf("%s: certain same_org %v by %s", d.ID, j.Keys, j.Reason)
			}
		}
		for _, m := range e.Orgs {
			if m.Org == nil {
				continue
			}
			if c := o.Organisations[*m.Org]; c.Directory != nil || c.Name == nil || *c.Name != m.Text {
				raw, _ := json.Marshal(c)
				t.Errorf("%s: the card of %q is %s, want the printed name and no directory id", d.ID, m.Text, raw)
			}
		}
		if c := o.Organisations["BIC KORUTR2A"]; c.Directory == nil || *c.Directory != "Q900001" {
			t.Errorf("%s: the code's own card is its entry: %+v", d.ID, c)
		}
	}
}

// A mention's span is the whole name as printed: the legal forms the key
// drops after a name written without spaces stay in the span.
func TestSpanKeepsTheLegalFormsTheKeyDrops(t *testing.T) {
	for _, c := range []struct{ line, name string }{
		{"白浜銀行株式会社Ltd", "白浜銀行株式会社Ltd"},
		{"白浜銀行株式会社", "白浜銀行株式会社"},
		{"白浜銀行 株式会社 本店", "白浜銀行 株式会社"},
		{"白浜銀行 Ltd", "白浜銀行 Ltd"},
		{"白浜銀行 Limited", "白浜銀行 Limited"},
		{"白浜銀行 GmbH", "白浜銀行"},
		{"白浜銀行 Ltda", "白浜銀行"},
		{"株式会社白浜銀行", "株式会社白浜銀行"},
	} {
		_, e := runOne(t, doc("d", c.line, "Account number: 1234567  BIC: SRHMJP2T"), nil)
		eq(t, c.line, names(e), []string{c.name})
		if len(e.Orgs) == 1 && (e.Orgs[0].Org == nil || !strings.HasSuffix(*e.Orgs[0].Org, ":白浜銀行")) {
			t.Errorf("%q: key %v, want the match key 白浜銀行", c.line, *e.Orgs[0].Org)
		}
	}
}
