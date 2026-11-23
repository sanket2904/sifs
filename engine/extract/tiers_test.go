package extract

import (
	"encoding/json"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/extract/orgkey"
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
	names := map[proof]string{read: "read", ibanRegistered: "ibanRegistered", zoneChecked: "zoneChecked", directoryCode: "directoryCode", zoneSays: "zoneSays", zoneAndPrintedYear: "zoneAndPrintedYear"}
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
		"passport expiry by zoneAndPrintedYear",
		"passport holder by zoneSays",
		"passport number by zoneSays",
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
		"IT60X0542811101000000123456 -> IBAN IT 05428 certain",
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
	printed := func(expiry string, zone ...string) Document {
		return doc("p", append([]string{"PASSPORT", "Passport No.: L898902C3", "Surname: Eriksson", "Given names: Anna Maria", expiry}, zone...)...)
	}
	zone1912 := []string{zone[0], "L898902C36UTO7408122F1204159ZE184226B<<<<<10"}
	for _, c := range []struct {
		name string
		d    Document
		want map[string]Tier
	}{
		{"no printed date: the zone gives no century", printed("Valid for ten years", zone...), map[string]Tier{"holder": Certain, "number": Certain, "expiry": Likely, "current": Likely}},
		{"the printed date is on the next line", printed("Date of expiry:\n15 APR 2012", zone...), map[string]Tier{"holder": Certain, "number": Certain, "expiry": Likely, "current": Likely}},
		{"a printed year of another century", printed("Date of expiry: 15 APR 1912", zone1912...), map[string]Tier{"holder": Certain, "number": Certain, "expiry": Certain, "current": Likely}},
		{"a printed date the zone does not give", printed("Date of expiry: 16 APR 2012", zone...), map[string]Tier{"holder": Certain, "number": Certain, "expiry": Likely, "current": Likely}},
		{"the zone says every fact", page("L898902C3", "Eriksson", zone...), map[string]Tier{"holder": Certain, "number": Certain, "expiry": Certain, "current": Likely}},
		{"a printed number the zone does not give", page("L898902C4", "Eriksson", zone...), map[string]Tier{"holder": Certain, "number": Likely, "expiry": Certain, "current": Likely}},
		{"a printed surname the zone does not give", page("L898902C3", "Ericsson", zone...), map[string]Tier{"holder": Likely, "number": Certain, "expiry": Certain, "current": Likely}},
		{"a full-width surname is the zone's", page("L898902C3", "ＥＲＩＫＳＳＯＮ", zone...), map[string]Tier{"holder": Certain, "number": Certain, "expiry": Certain, "current": Likely}},
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
		if p := e.Passport; p.Tiers["expiry"] == Certain {
			if at := p.ExpiryAt; at == nil || p.Expiry == nil || at.Text[len(at.Text)-4:] != (*p.Expiry)[:4] {
				t.Errorf("%s: a certain expiry %v cites the printed date that gives its year: %+v", c.name, p.Expiry, at)
			}
		}
	}
}

// TestPrintedExpiry holds printedExpiry to the scorer's test of a certain
// expiry's printed date (sample/tools/lib/tiers.mjs, printedExpiry).
func TestPrintedExpiry(t *testing.T) {
	for _, c := range []struct{ text, zone, want string }{
		{"15 APR 2012", "120415", "2012-04-15"},
		{"15.04.2012", "120415", "2012-04-15"},
		{"04/15/2012", "120415", "2012-04-15"},
		{"2012年4月15日", "120415", "2012-04-15"},
		{"１５ APR ２０１２", "120415", "2012-04-15"},
		{"⑮ APR 2012", "120415", "2012-04-15"},
		{"02 JUN 1998", "980602", "1998-06-02"},
		{"15 APR 2013", "120415", ""},
		{"16 APR 2012", "120415", ""},
		{"15 APR 12", "120415", ""},
		{"15.05.2012", "120415", ""},
		{"15 APR 2012 2012", "120415", ""},
		{"015 APR 2012", "120415", ""},
		{"१५ APR २०१२", "120415", ""},
	} {
		got := ""
		if d, ok := printedExpiry(c.text, c.zone); ok {
			got = d.String()
		}
		if got != c.want {
			t.Errorf("printedExpiry(%q, %s) = %q, want %q", c.text, c.zone, got, c.want)
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
	eq(t, "keys", keys, []string{"BIC COBADE33", "BIC COBADEFF", "IFSC ALFA", "IFSC BETA", "name:d:cobabank"})
	eq(t, "same_org", joinsOf(e), []string{"BIC COBADE33 = name:d:cobabank: spells-code likely", "BIC COBADEFF = name:d:cobabank: spells-code likely"})
}

// A payment order names two banks: the payer's and the payee's. Whatever
// order the lines come in and however many BICs it prints, no key carries
// identifiers of both, and no same_org row joins them.
func TestPayerAndPayeeStayApart(t *testing.T) {
	const (
		payerIBAN = "DE25 1002 0030 0123 4567 89"
		payeeIBAN = "DE56 5007 0080 9876 5432 10"
	)
	owner := map[string]string{
		"DE25100200300123456789": "payer", "IBAN DE 10020030": "payer", "PAYRDEFFXXX": "payer", "BIC PAYRDEFF": "payer",
		"DE56500700809876543210": "payee", "IBAN DE 50070080": "payee", "PAYEDEFFXXX": "payee", "BIC PAYEDEFF": "payee",
	}
	for name, lines := range map[string][]string{
		"one BIC, payer first":  {"Payment order", "Ordering customer IBAN: " + payerIBAN, "Beneficiary BIC: PAYEDEFFXXX", "Beneficiary IBAN: " + payeeIBAN},
		"one BIC, payee first":  {"Payment order", "Beneficiary IBAN: " + payeeIBAN, "Beneficiary BIC: PAYEDEFFXXX", "Ordering customer IBAN: " + payerIBAN},
		"two BICs, payer first": {"Payment order", "Ordering customer IBAN: " + payerIBAN, "Beneficiary BIC: PAYEDEFFXXX", "Beneficiary IBAN: " + payeeIBAN, "Ordering BIC: PAYRDEFFXXX"},
		"two BICs, payee first": {"Payment order", "Beneficiary IBAN: " + payeeIBAN, "Ordering BIC: PAYRDEFFXXX", "Ordering customer IBAN: " + payerIBAN, "Beneficiary BIC: PAYEDEFFXXX"},
	} {
		_, e := runOne(t, doc("d", lines...), nil)
		val := map[string]string{}
		for _, i := range e.Identifiers {
			val[i.ID] = i.Value
		}
		for _, l := range e.Links {
			if a, b := owner[val[l.Identifier]], owner[l.Org]; a == "" || a != b {
				t.Errorf("%s: %s (%s) linked to %s (%s)", name, val[l.Identifier], a, l.Org, b)
			}
		}
		for _, j := range e.SameOrg {
			if a, b := owner[j.Keys[0]], owner[j.Keys[1]]; a == "" || a != b {
				t.Errorf("%s: same_org %s (%s) = %s (%s)", name, j.Keys[0], a, j.Keys[1], b)
			}
		}
	}
}

// Where an IBAN's bank identifier is the letters a BIC begins with, that
// pair is the only reading: the other IBAN is joined to no BIC.
func TestLettersPickTheOnlyReading(t *testing.T) {
	_, e := runOne(t, doc("d",
		"Payment order",
		"Ordering customer IBAN: GB71 ALFA 2004 1531 9268 19",
		"Beneficiary BIC: BETAGB2LXXX",
		"Beneficiary IBAN: GB22 BETA 4005 1655 7799 11"), nil)
	eq(t, "same_org", joinsOf(e), []string{"BIC BETAGB2L = IBAN GB BETA: printed-together likely"})
}

func TestShortFormSettledByACodeKeepsItsOwnKey(t *testing.T) {
	o, e := runOne(t, doc("d", "Bank: EXMB", "Account: 4720195386 IFSC: EXMB0123456"), nil)
	if len(e.Orgs) != 1 || e.Orgs[0].Org == nil || *e.Orgs[0].Org != "name:d:exmb" || *e.Orgs[0].SettledBy != "code" {
		t.Fatalf("mention: %+v", e.Orgs)
	}
	if c := o.Organisations["name:d:exmb"]; c.Name == nil || *c.Name != "EXMB" {
		t.Errorf("card: %+v", c)
	}
	eq(t, "same_org", joinsOf(e), []string{"IFSC EXMB = name:d:exmb: spells-code likely"})
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

	// two documents print one name: each keeps its own key, whatever the order
	advice := doc("advice-ca", "SMBK", "Credit advice", "Account: 4720195386", "BIC: SMBKCAT0XXX")
	payroll := doc("payroll-gb", "SMBK", "Payroll release", "Account: 12345678", "BIC: SMBKGB20XXX")
	want := map[string]string{"advice-ca": "name:advice-ca:smbk", "payroll-gb": "name:payroll-gb:smbk"}
	forward, backward := Run([]Document{advice, payroll}, nil, registry, ref), Run([]Document{payroll, advice}, nil, registry, ref)
	eq(t, "claims with two documents that print one name, reversed", claimIDs(backward), claimIDs(forward))
	for _, o := range []Output{forward, backward} {
		for _, d := range o.Documents {
			if len(d.Orgs) != 1 || d.Orgs[0].Org == nil || *d.Orgs[0].Org != want[d.ID] {
				t.Errorf("%s: mentions %+v, want the key %s", d.ID, d.Orgs, want[d.ID])
			}
		}
	}
}

// TestNoMergeInTheSample checks the sample's runs against "No merging": a
// mention's key is a printed name's of its own document, spelt from the
// texts of the mentions on it; a code link goes to its code's key.
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
				texts := map[string][]string{}
				for _, m := range doc.Orgs {
					if m.Org != nil {
						texts[*m.Org] = append(texts[*m.Org], m.Text)
					}
				}
				for key, ts := range texts {
					if k, _ := orgkey.Name(doc.ID, ts...); k.String() != key {
						t.Errorf("%s %s: mentions %q on key %q, spelt %q", set, doc.ID, ts, key, k)
					}
				}
				for _, l := range doc.Links {
					i := ident[l.Identifier]
					own := codeReasons[i.Kind]
					if own != "" && contains(l.Reasons, own) && l.Org != codeKey(i.Kind, i.Value, i.Valid, registry).String() {
						t.Errorf("%s %s: %s %s by %s goes to %q", set, doc.ID, i.Kind, i.Value, own, l.Org)
					}
				}
			}
		}
	}
}

// An IBAN, a BIC or an IFSC is held only at its own code's key: the one
// constructor that places an identifier elsewhere refuses it, so a link of
// an IBAN to a BIC's key cannot be made.
func TestNoLinkPlacesACodeAtAnotherKey(t *testing.T) {
	ids := findIdentifiers(splitLines(doc("d",
		"Beneficiary IBAN: PT13 0099 0002 3141 5926 5358 9",
		"Correspondent BIC: TAGOPT2L",
		"IFSC: ALFA0BR0001",
		"Account: 111122223333")), registry)
	byKind := map[string]Identifier{}
	for _, id := range ids {
		byKind[id.Kind] = id
	}
	bic := byKind[KindBIC]
	for _, k := range []string{KindIBAN, KindBIC, KindIFSC} {
		id := byKind[k]
		if _, ok := heldAt(id, bic.Code, ReasonPairedCode, &bic); ok {
			t.Errorf("%s %s placed at %s", k, id.Value, bic.Code)
		}
		if l, ok := ownLink(id); !ok || l.key != id.Code || id.Code.IsZero() {
			t.Errorf("%s %s: own link %+v, %v", k, id.Value, l, ok)
		}
	}
	if byKind[KindIBAN].Code.String() != "IBAN PT 0099" {
		t.Errorf("the IBAN's key: %s", byKind[KindIBAN].Code)
	}
	if _, ok := heldAt(byKind[KindAccount], bic.Code, ReasonPairedCode, &bic); !ok {
		t.Errorf("an account may be held at a code's key")
	}
}
