package card

import (
	"bytes"
	"context"
	"encoding/json"
	"flag"
	"os"
	"path/filepath"
	"slices"
	"testing"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/load"
)

var update = flag.Bool("update", false, "write the goldens from the committed run")

const committedRun = "../../../sample/runs/go-m1a-on-text.json"

// referenceDate is fixed so the goldens do not change with the day.
const referenceDate = "2026-11-27"

// stores loads a run into the memory store and into SQLite in a temporary
// folder, and runs the test on each; the SQLite store is closed after the
// load and opened again to read, as sifs-card opens it.
func stores(t *testing.T, run *extract.Output, test func(t *testing.T, e *load.Engine)) {
	ctx := context.Background()
	t.Run("memory", func(t *testing.T) {
		e, err := load.OpenMemory(ctx)
		if err != nil {
			t.Fatal(err)
		}
		mustLoad(t, e, run)
		test(t, e)
	})
	t.Run("sqlite", func(t *testing.T) {
		dir := t.TempDir()
		e, err := load.Open(ctx, dir)
		if err != nil {
			t.Fatal(err)
		}
		mustLoad(t, e, run)
		if err := e.Close(); err != nil {
			t.Fatal(err)
		}
		if e, err = load.OpenExisting(ctx, dir); err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { e.Close() })
		test(t, e)
	})
}

func mustLoad(t *testing.T, e *load.Engine, run *extract.Output) {
	t.Helper()
	rep, err := e.Load(context.Background(), run)
	if err != nil || len(rep.TurnedDown) > 0 {
		t.Fatalf("load: %v %+v", err, rep.TurnedDown)
	}
}

func find(t *testing.T, e *load.Engine, query string) Answer {
	t.Helper()
	ans, err := Find(context.Background(), e, query)
	if err != nil {
		t.Fatal(err)
	}
	return ans
}

func ask(t *testing.T, e *load.Engine, question string) Asked {
	t.Helper()
	ans, err := Ask(context.Background(), e, question, referenceDate)
	if err != nil {
		t.Fatal(err)
	}
	return ans
}

func encode(t *testing.T, v any) []byte {
	t.Helper()
	b, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	return append(b, '\n')
}

// golden compares an answer's bytes with its file in testdata, on both
// stores, and asks the same question twice. With -update it writes the
// file from the memory store.
func golden(t *testing.T, e *load.Engine, name string, answer func() any) {
	t.Helper()
	got := encode(t, answer())
	if again := encode(t, answer()); !bytes.Equal(got, again) {
		t.Errorf("%s: the same store gave two answers:\n%s\n%s", name, got, again)
	}
	path := filepath.Join("testdata", name+".json")
	if *update && t.Name() == "TestGoldens/memory" {
		if err := os.WriteFile(path, got, 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	want, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want) {
		t.Errorf("%s differs from %s:\n%s", name, path, got)
	}
}

// The goldens are the committed run's answers. Mutation: drop the sort in
// strs, and SQLite gives the statement joins' reasons in another order
// than the golden.
func TestGoldens(t *testing.T) {
	run, err := load.ReadRun(committedRun)
	if err != nil {
		t.Fatal(err)
	}
	stores(t, run, func(t *testing.T, e *load.Engine) {
		golden(t, e, "hdfc", func() any { return find(t, e, "HDFC") })
		golden(t, e, "passport", func() any { return ask(t, e, "when does my passport expire") })
		golden(t, e, "nothing", func() any { return find(t, e, "Zebra Savings") })
	})
}

// The HDFC golden as the brief promises it, read from the answer itself.
func TestHDFCCard(t *testing.T) {
	run, err := load.ReadRun(committedRun)
	if err != nil {
		t.Fatal(err)
	}
	stores(t, run, func(t *testing.T, e *load.Engine) {
		ans := find(t, e, "HDFC")
		bank := ans.Cards[0]
		if bank.Head != "IFSC HDFC" || bank.StandsOn != "certain" {
			t.Fatalf("first card: head %s, stands on %s", bank.Head, bank.StandsOn)
		}
		for _, k := range bank.Keys {
			if want := map[bool]Tier{true: Certain, false: Likely}[k.Key == "IFSC HDFC"]; k.Joined != want {
				t.Errorf("key %s joined %s, want %s", k.Key, k.Joined, want)
			}
		}
		if len(bank.Keys) != 5 {
			t.Errorf("keys: %+v", bank.Keys)
		}
		for _, j := range bank.Joins {
			if j.Tier != Likely || j.CardTier != Likely {
				t.Errorf("join %+v is not likely", j)
			}
		}
		held := map[string]Held{}
		for _, h := range bank.Held {
			held[h.Value] = h
		}
		account := held["50100417263958"]
		if account.Kind != "account" || account.CardTier != Likely || len(account.Links) == 0 {
			t.Errorf("account: %+v", account)
		}
		for _, l := range account.Links {
			if l.CardTier != Likely || len(l.Reasons) == 0 {
				t.Errorf("account link %+v", l)
			}
		}
		if ifsc := held["HDFC0SAMPL1"]; ifsc.CardTier != Certain || !slices.Equal(ifsc.Links[0].Reasons, []string{"ifsc-prefix"}) {
			t.Errorf("ifsc: %+v", ifsc)
		}
		docs := cardDocuments(bank)
		named := map[string]bool{}
		for _, n := range ans.AlsoNamedIn {
			named[n.Document] = true
		}
		for _, other := range []string{"hdfc-life-receipt", "hdfc-ltd-loan-2019", "hdfc-card-statement"} {
			if docs[other] {
				t.Errorf("%s is on the bank's card", other)
			}
			if !named[other] {
				t.Errorf("%s is not listed as also named in", other)
			}
		}
	})
}

// The passport golden as the brief promises it.
func TestPassportAnswer(t *testing.T) {
	run, err := load.ReadRun(committedRun)
	if err != nil {
		t.Fatal(err)
	}
	stores(t, run, func(t *testing.T, e *load.Engine) {
		ans := ask(t, e, "when does my passport expire")
		byDoc := map[string]Passport{}
		holders := map[string]bool{}
		for _, p := range ans.Passports {
			byDoc[p.Document] = p
			holders[p.Holder] = true
		}
		if asha := byDoc["passport-asha-current"]; asha.Holder != "KALPIT, ASHA" || asha.Expiry.Date != "2034-03-17" || asha.Expiry.Page != 1 || asha.Expiry.Status != "in-date" {
			t.Errorf("asha: %+v", asha)
		}
		for _, doc := range []string{"passport-meera-1988", "passport-bad-mrz"} {
			if p := byDoc[doc]; p.Expiry.Date != "" || p.Expiry.Status != "cannot-tell" {
				t.Errorf("%s gives a date: %+v", doc, p.Expiry)
			}
		}
		if want := []string{"ERIKSSON, ANNA MARIA", "KALPIT, ASHA", "KALPIT, MEERA", "KALPIT, ROHAN"}; !slices.Equal(sortedKeys(holders), want) {
			t.Errorf("holders %v, want %v", sortedKeys(holders), want)
		}
		if len(ans.Passports) != 5 {
			t.Errorf("%d passports, want 5", len(ans.Passports))
		}
	})
}

func TestOtherQuestionNotAnswered(t *testing.T) {
	stores(t, &extract.Output{Format: 3}, func(t *testing.T, e *load.Engine) {
		for _, q := range []string{"what is my balance", "who is my bank", "passport number"} {
			if ans := ask(t, e, q); ans.Answer != NotAnswered || len(ans.Passports) != 0 {
				t.Errorf("%q: %+v", q, ans)
			}
		}
		for _, q := range []string{"When does my PASSPORT expire?", "passport expiry", "is my passport valid until 2030"} {
			if ans := ask(t, e, q); ans.Answer != PassportExpiry {
				t.Errorf("%q: %+v", q, ans)
			}
		}
	})
}

// The runs below are made up.

func ptr[T any](v T) *T { return &v }

func span(page, start int, text string) extract.Span {
	return extract.Span{Page: page, Start: start, End: start + len(text), Text: text}
}

func receipt(id string) extract.DocOut {
	key := "name:" + id + ":examplebank"
	return extract.DocOut{ID: id, Orgs: []extract.OrgOut{{Span: span(1, 0, "Example Bank"), Org: ptr(key), SettledBy: ptr("name")}}}
}

// Two documents print the same name and no same_org link joins their keys:
// they are two cards. Mutation: in groups, also join keys whose printed
// names are the same, and one card holds both.
func TestSameNameTwoCards(t *testing.T) {
	run := &extract.Output{Format: 3, Documents: []extract.DocOut{receipt("receipt-a"), receipt("receipt-b")}}
	stores(t, run, func(t *testing.T, e *load.Engine) {
		ans := find(t, e, "example")
		if len(ans.Cards) != 2 {
			t.Fatalf("%d cards, want 2: %s", len(ans.Cards), encode(t, ans))
		}
		for _, c := range ans.Cards {
			if len(c.Keys) != 1 || len(c.Documents) != 1 || c.StandsOn != "likely-only" {
				t.Errorf("card %s", encode(t, c))
			}
		}
		if len(ans.AlsoNamedIn) != 1 || ans.AlsoNamedIn[0].Document == ans.Cards[0].Documents[0].Document {
			t.Errorf("also named in: %+v", ans.AlsoNamedIn)
		}
	})
}

// A chain: the code key and the printed name are joined certain, the name
// and a second printed name likely. What reaches the card only through the
// likely join is likely, though its own row says certain. Mutation: in
// joined, follow likely same_org links too, and the second name's link is
// certain on the card.
func TestLikelyJoinLowersWhatPassesThrough(t *testing.T) {
	name, other := "name:chain-doc:examplebank", "name:chain-doc:exmpcorp"
	doc := extract.DocOut{
		ID: "chain-doc",
		Orgs: []extract.OrgOut{
			{Span: span(1, 0, "Example Bank"), Org: ptr(name), SettledBy: ptr("name")},
			{Span: span(1, 20, "Exmp Corp"), Org: ptr(other), SettledBy: ptr("name")},
		},
		Identifiers: []extract.IdentOut{
			{Span: span(1, 40, "000111222333"), ID: "i1", Kind: "account", Value: "000111222333", Tier: extract.Likely},
			{Span: span(1, 60, "000444555666"), ID: "i2", Kind: "account", Value: "000444555666", Tier: extract.Likely},
		},
		Links: []extract.LinkOut{
			{Identifier: "i1", Org: name, Reasons: []string{"statement-header"}, Tier: extract.Certain},
			{Identifier: "i2", Org: other, Reasons: []string{"statement-header"}, Tier: extract.Certain},
		},
		SameOrg: []extract.SameOrgOut{
			{Keys: [2]string{"IFSC EXMP", name}, Reason: "directory", Tier: extract.Certain},
			{Keys: [2]string{name, other}, Reason: "printed-together", Tier: extract.Likely},
		},
	}
	stores(t, &extract.Output{Format: 3, Documents: []extract.DocOut{doc}}, func(t *testing.T, e *load.Engine) {
		ans := find(t, e, "EXMP")
		if len(ans.Cards) != 1 || ans.Cards[0].Head != "IFSC EXMP" {
			t.Fatalf("cards: %s", encode(t, ans))
		}
		got := map[string]Tier{}
		for _, h := range ans.Cards[0].Held {
			got[h.Value] = h.CardTier
		}
		if want := map[string]Tier{"000111222333": Certain, "000444555666": Likely}; !mapsEqual(got, want) {
			t.Errorf("held card tiers %v, want %v", got, want)
		}
		for _, j := range ans.Cards[0].Joins {
			if j.Other == other && j.CardTier != Likely {
				t.Errorf("join %+v", j)
			}
		}
	})
}

func mapsEqual[K comparable, V comparable](a, b map[K]V) bool {
	if len(a) != len(b) {
		return false
	}
	for k, v := range a {
		if w, ok := b[k]; !ok || w != v {
			return false
		}
	}
	return true
}

func passportDoc(id string, expiry string, tier extract.Tier, at *extract.Span) extract.DocOut {
	return extract.DocOut{ID: id, Passport: &extract.PassportOut{
		Holder: extract.HolderOut{Surname: "SAMPLE", Given: id}, Expiry: ptr(expiry), ExpiryAt: at, Current: ptr(true),
		Tiers: map[string]extract.Tier{"holder": extract.Likely, "expiry": tier, "current": extract.Likely}}}
}

// An expiry is a date only when it is certain and has its page.
// Mutations: drop the no-page case in expiry, and the certain expiry
// without a page gives its date; drop the likely case, and the likely
// expiry with a page gives its date.
func TestExpiryOnlyWhenCertainWithPage(t *testing.T) {
	at := &extract.Span{Page: 1, Start: 10, End: 21, Text: "30 APR 2030"}
	run := &extract.Output{Format: 3, Documents: []extract.DocOut{
		passportDoc("NOPAGE", "2030-04-30", extract.Certain, nil),
		passportDoc("LIKELY", "2030-04-30", extract.Likely, at),
		passportDoc("GIVEN", "2030-04-30", extract.Certain, at),
	}}
	stores(t, run, func(t *testing.T, e *load.Engine) {
		got := map[string]Expiry{}
		for _, p := range ask(t, e, "passport expiry").Passports {
			got[p.Holder] = p.Expiry
		}
		want := map[string]Expiry{
			"SAMPLE, NOPAGE": {Status: "cannot-tell", Why: "no-page", Tier: Certain},
			"SAMPLE, LIKELY": {Status: "cannot-tell", Why: "likely-only", Tier: Likely},
			"SAMPLE, GIVEN":  {Status: "in-date", Date: "2030-04-30", Tier: Certain, Page: 1, Start: 10, End: 21, Text: "30 APR 2030"},
		}
		if !mapsEqual(got, want) {
			t.Errorf("expiries:\n got %+v\nwant %+v", got, want)
		}
	})
}
