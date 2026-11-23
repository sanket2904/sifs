package extract

import (
	"encoding/json"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"testing"
)

// TestTierTable holds the table to sample/FORMAT.md's: of every claim the
// extractor writes, only these are certain, each by its one proof.
func TestTierTable(t *testing.T) {
	var claims []claim
	for _, k := range []string{KindIBAN, KindBIC, KindIFSC, KindCard, KindPAN, KindMRZ, KindSortCode, KindFRBank, KindFRBranch, KindFRKey, KindAccount, KindPassportNo} {
		claims = append(claims, identClaim(k))
	}
	for _, r := range []string{ReasonIBANBankCode, ReasonBICInstitution, ReasonIFSCPrefix, ReasonPairedCode, ReasonInsideIBAN, ReasonSameAccount, ReasonStatementHeader} {
		claims = append(claims, linkClaim(r))
	}
	for _, f := range []string{"period", "accounts", "issuer"} {
		claims = append(claims, stmtClaim(f))
	}
	for _, f := range []string{"holder", "number", "expiry", "current"} {
		claims = append(claims, passportClaim(f))
	}
	for _, r := range []string{JoinDirectory, JoinSpellsCode, JoinPrintedTogether, "statement-header"} {
		claims = append(claims, joinClaim(r))
	}
	names := map[proof]string{read: "read", ibanRegistered: "ibanRegistered", zoneChecked: "zoneChecked", directoryCode: "directoryCode", zoneSays: "zoneSays", directoryJoin: "directoryJoin"}
	var got []string
	for _, c := range claims {
		for p := range names {
			if tierOf(c, p) == Certain {
				got = append(got, string(c)+" by "+names[p])
			}
		}
	}
	sort.Strings(got)
	eq(t, "certain claims", got, []string{
		"identifier bic by directoryCode",
		"identifier iban by ibanRegistered",
		"identifier ifsc by directoryCode",
		"identifier mrz by zoneChecked",
		"link bic-institution by directoryCode",
		"link iban-bank-code by ibanRegistered",
		"link ifsc-prefix by directoryCode",
		"passport expiry by zoneSays",
		"passport holder by zoneSays",
		"passport number by zoneSays",
		"same_org directory by directoryJoin",
	})
}

// tiersOf lists a document's identifiers and links with their tiers.
func tiersOf(e DocOut) []string {
	val := map[string]string{}
	var out []string
	for _, i := range e.Identifiers {
		val[i.ID] = i.Value
		out = append(out, i.Kind+" "+i.Value+" "+string(i.Tier))
	}
	for _, l := range e.Links {
		out = append(out, val[l.Identifier]+" -> "+l.Org+" "+string(l.Tier))
	}
	sort.Strings(out)
	return out
}

func TestCodesAreCertainOnlyByTheirProof(t *testing.T) {
	d := doc("d", "Pay to: 111122223333 IFSC: RIVR0BR0001")
	_, e := runOne(t, d, nil)
	eq(t, "directory off", tiersOf(e), []string{
		"111122223333 -> IFSC RIVR likely", "RIVR0BR0001 -> IFSC RIVR likely",
		"account 111122223333 likely", "ifsc RIVR0BR0001 likely",
	})
	_, e = runOne(t, d, NewDirectory([]DirEntry{entry("Q1", "Riverside Bank", "riversidebank", "RIVR")}))
	eq(t, "the directory holds the code", tiersOf(e), []string{
		"111122223333 -> IFSC RIVR likely", "RIVR0BR0001 -> IFSC RIVR certain",
		"account 111122223333 likely", "ifsc RIVR0BR0001 certain",
	})
	_, e = runOne(t, d, NewDirectory([]DirEntry{entry("Q1", "Riverside Bank", "riversidebank", "RIVR"), entry("Q2", "River Trust Bank", "rivertrustbank", "RIVR")}))
	eq(t, "two entries hold the code: it is an IFSC, but whose is open", tiersOf(e), []string{
		"account 111122223333 likely", "ifsc RIVR0BR0001 certain",
	})

	// an IBAN is proven by its registry row and check digits alone
	_, e = runOne(t, doc("d", "IBAN: DE89 3704 0044 0532 0130 00", "IBAN: DE89 3704 0044 0532 0130 01", "IBAN: IT60 X054 2811 1010 0000 0123 456"), nil)
	eq(t, "IBANs", tiersOf(e), []string{
		"DE89370400440532013000 -> IBAN DE 37040044 certain",
		"iban DE89370400440532013000 certain",
		"iban DE89370400440532013001 likely",
		"iban IT60X0542811101000000123456 certain",
	})

	// a card's Luhn digit and a printed label prove nothing
	_, e = runOne(t, doc("d", "Card: 4111 1111 1111 1111", "Passport No.: L898902C3", "PAN: ABCPK1234F"), nil)
	for _, i := range e.Identifiers {
		if i.Tier != Likely {
			t.Errorf("%s %s is %s", i.Kind, i.Value, i.Tier)
		}
	}
}

func TestPassportFactsAreCertainOnlyAsTheZoneSaysThem(t *testing.T) {
	zone := []string{"P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<", "L898902C36UTO7408122F1204159ZE184226B<<<<<10"}
	page := func(number, surname string, zone ...string) Document {
		return doc("p", append([]string{"PASSPORT", "Passport No.: " + number, "Surname: " + surname, "Given names: Anna  Maria", "Date of expiry: 15 APR 2012"}, zone...)...)
	}
	for _, c := range []struct {
		name string
		d    Document
		want map[string]Tier
	}{
		{"the zone says every fact", page("L898902C3", "Eriksson", zone...), map[string]Tier{"holder": Certain, "number": Certain, "expiry": Certain, "current": Likely}},
		{"a printed number the zone does not give", page("L898902C4", "Eriksson", zone...), map[string]Tier{"holder": Certain, "number": Likely, "expiry": Certain, "current": Likely}},
		{"a printed surname the zone does not give", page("L898902C3", "Ericsson", zone...), map[string]Tier{"holder": Likely, "number": Certain, "expiry": Certain, "current": Likely}},
		{"a zone that fails its check", page("L898902C3", "Eriksson", zone[0], zone[1][:43]+"1"), map[string]Tier{"holder": Likely, "number": Likely, "expiry": Likely, "current": Likely}},
		{"no zone", page("L898902C3", "Eriksson"), map[string]Tier{"holder": Likely, "number": Likely, "expiry": Likely, "current": Likely}},
	} {
		_, e := runOne(t, c.d, nil)
		if e.Passport == nil {
			t.Errorf("%s: no passport", c.name)
			continue
		}
		if !mapsEqual(e.Passport.Tiers, c.want) {
			t.Errorf("%s: tiers %v, want %v", c.name, e.Passport.Tiers, c.want)
		}
	}
}

func mapsEqual(a, b map[string]Tier) bool {
	if len(a) != len(b) {
		return false
	}
	for k, v := range a {
		if b[k] != v {
			return false
		}
	}
	return true
}

func TestNoTwoOrganisationsShareAKey(t *testing.T) {
	o, e := runOne(t, doc("d",
		"COBA Bank AG",
		"BIC: COBADEFFXXX",
		"Correspondent BIC: COBADE33XXX",
		"Debit account: 111122223333 IFSC: ALFA0BR0001",
		"Credit account: 444455556666 IFSC: BETA0BR0002"), nil)
	var keys []string
	for k := range o.Organisations {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	eq(t, "keys", keys, []string{"BIC COBADE33", "BIC COBADEFF", "IFSC ALFA", "IFSC BETA", "name:cobabank"})
	eq(t, "same_org", joinsOf(e), []string{"BIC COBADE33 = name:cobabank: spells-code likely", "BIC COBADEFF = name:cobabank: spells-code likely"})
}

func TestShortFormSettledByACodeKeepsItsOwnKey(t *testing.T) {
	o, e := runOne(t, doc("d", "Bank: EXMB", "Account: 4720195386 IFSC: EXMB0123456"), nil)
	if len(e.Orgs) != 1 || e.Orgs[0].Org == nil || *e.Orgs[0].Org != "name:exmb" || *e.Orgs[0].SettledBy != "code" {
		t.Fatalf("mention: %+v", e.Orgs)
	}
	if c := o.Organisations["name:exmb"]; c.Name == nil || *c.Name != "EXMB" {
		t.Errorf("card: %+v", c)
	}
	eq(t, "same_org", joinsOf(e), []string{"IFSC EXMB = name:exmb: spells-code likely"})
}

func loadSample(t *testing.T, set string) ([]Document, Date, *Directory) {
	t.Helper()
	root := "../../sample"
	var index struct {
		Documents     []string `json:"documents"`
		ReferenceDate string   `json:"reference_date"`
	}
	raw, err := os.ReadFile(filepath.Join(root, set, "index.json"))
	if err != nil {
		t.Skip("no sample beside the module")
	}
	if err := json.Unmarshal(raw, &index); err != nil {
		t.Fatal(err)
	}
	docs := make([]Document, len(index.Documents))
	for i, id := range index.Documents {
		raw, err := os.ReadFile(filepath.Join(root, set, id+".json"))
		if err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(raw, &docs[i]); err != nil {
			t.Fatal(err)
		}
	}
	day, ok := ParseDate(index.ReferenceDate)
	if !ok {
		t.Fatalf("reference date %q", index.ReferenceDate)
	}
	dir, err := LoadDirectory(filepath.Join(root, "directory", "banks.json"))
	if err != nil {
		t.Fatal(err)
	}
	return docs, day, dir
}

// claimIDs names every claim of a run as FORMAT.md's "Ids" do, from its
// content, never its position: a link by its document, its identifier's
// kind and value, and its key; a same_org row by its document and its two
// keys. Mentions and identifiers are named by their spans, cards by keys.
func claimIDs(o Output) []string {
	var out []string
	for k, c := range o.Organisations {
		raw, _ := json.Marshal(c)
		out = append(out, "card|"+k+"|"+string(raw))
	}
	for _, d := range o.Documents {
		ident := map[string]IdentOut{}
		for _, i := range d.Identifiers {
			ident[i.ID] = i
			out = append(out, strings.Join([]string{"identifier", d.ID, i.Kind, i.Value, itoa(i.Page), itoa(i.Start), string(i.Tier)}, "|"))
		}
		for _, m := range d.Orgs {
			key := ""
			if m.Org != nil {
				key = *m.Org
			}
			out = append(out, strings.Join([]string{"mention", d.ID, itoa(m.Page), itoa(m.Start), key}, "|"))
		}
		for _, l := range d.Links {
			i := ident[l.Identifier]
			out = append(out, strings.Join([]string{"link", d.ID, i.Kind, i.Value, l.Org, strings.Join(l.Reasons, ","), string(l.Tier)}, "|"))
		}
		for _, j := range d.SameOrg {
			out = append(out, strings.Join([]string{"same_org", d.ID, j.Keys[0], j.Keys[1], j.Reason, string(j.Tier)}, "|"))
		}
	}
	sort.Strings(out)
	return out
}

// TestIdsAreStable runs the sample twice, and once with its documents in
// the other order: every claim must come out with the same id.
func TestIdsAreStable(t *testing.T) {
	for _, set := range []string{"text", "text-ocr"} {
		docs, day, dir := loadSample(t, set)
		reversed := slices.Clone(docs)
		slices.Reverse(reversed)
		for _, d := range []*Directory{nil, dir} {
			first, again, other := Run(docs, d, registry, day), Run(docs, d, registry, day), Run(reversed, d, registry, day)
			a, _ := json.Marshal(first)
			b, _ := json.Marshal(again)
			if string(a) != string(b) {
				t.Errorf("%s: two runs over the same files differ", set)
			}
			ids := claimIDs(first)
			if len(ids) < 100 {
				t.Fatalf("%s: only %d claims", set, len(ids))
			}
			eq(t, set+": claims with the documents reversed", claimIDs(other), ids)
		}
	}
}

// TestNoMergeInTheSample checks the sample's runs against "No merging": a
// mention's key is a printed name's, spelt from text; a code link goes to
// its code's key.
func TestNoMergeInTheSample(t *testing.T) {
	for _, set := range []string{"text", "text-ocr"} {
		docs, day, dir := loadSample(t, set)
		for _, d := range []*Directory{nil, dir} {
			o := Run(docs, d, registry, day)
			for _, doc := range o.Documents {
				ident := map[string]IdentOut{}
				for _, i := range doc.Identifiers {
					ident[i.ID] = i
				}
				for _, m := range doc.Orgs {
					if m.Org != nil && (!strings.HasPrefix(*m.Org, "name:") || *m.Org == "name:") {
						t.Errorf("%s %s: mention %q on key %q", set, doc.ID, m.Text, *m.Org)
					}
				}
				for _, l := range doc.Links {
					i := ident[l.Identifier]
					own := codeReasons[i.Kind]
					if own != "" && contains(l.Reasons, own) && l.Org != codeKey(i.Kind, i.Value, i.Valid, registry) {
						t.Errorf("%s %s: %s %s by %s goes to %q", set, doc.ID, i.Kind, i.Value, own, l.Org)
					}
				}
			}
		}
	}
}
