package load

import (
	"context"
	"reflect"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/pack"

	"github.com/sanket2904/ontology-framework/ontology"
)

// The sample below is made up. Two statements of one account at one bank
// code, and a passport. Both statements give the account and the code's
// key; only the first gives its printed name's key and the header link.

func ptr[T any](v T) *T { return &v }

func span(page, start int, text string) extract.Span {
	return extract.Span{Page: page, Start: start, End: start + len(text), Text: text}
}

func statement(id string) extract.DocOut {
	name := "name:" + id + ":examplebank"
	return extract.DocOut{
		ID:   id,
		Orgs: []extract.OrgOut{{Span: span(1, 0, "Example Bank"), Org: ptr(name), SettledBy: ptr("name")}},
		Identifiers: []extract.IdentOut{
			{Span: span(1, 40, "000111222333"), ID: "i1", Kind: "account", Value: "000111222333", Tier: extract.Likely},
			{Span: span(1, 60, "EXMP0000001"), ID: "i2", Kind: "ifsc", Value: "EXMP0000001", Valid: ptr(true), Tier: extract.Certain},
		},
		Links: []extract.LinkOut{
			{Identifier: "i2", Org: "IFSC EXMP", Reasons: []string{"ifsc-prefix"}, Tier: extract.Certain},
			{Identifier: "i1", Org: "IFSC EXMP", Reasons: []string{"paired-code"}, Via: "i2", Tier: extract.Likely},
			{Identifier: "i1", Org: name, Reasons: []string{"statement-header"}, Tier: extract.Likely},
		},
		Statement: &extract.StmtOut{From: "2026-01-01", To: "2026-01-31", Accounts: []string{"i1"}, Issuer: name,
			Tiers: map[string]extract.Tier{"period": "likely", "accounts": "likely", "issuer": "likely"}},
		SameOrg: []extract.SameOrgOut{
			{Keys: [2]string{name, "IFSC EXMP"}, Reason: "directory", Tier: extract.Certain},
			{Keys: [2]string{"IFSC EXMP", name}, Reason: "spells-code", Tier: extract.Likely},
		},
	}
}

func passport() extract.DocOut {
	return extract.DocOut{
		ID:          "passport-sample",
		Orgs:        []extract.OrgOut{},
		Identifiers: []extract.IdentOut{{Span: span(1, 10, "X0000001"), ID: "i1", Kind: "passport_number", Value: "X0000001", Tier: extract.Likely}},
		Links:       []extract.LinkOut{},
		SameOrg:     []extract.SameOrgOut{},
		Passport: &extract.PassportOut{Holder: extract.HolderOut{Surname: "SAMPLE", Given: "ALEX"}, Number: ptr("i1"),
			Issued: ptr("2020-05-01"), Expiry: ptr("2030-04-30"), ExpiryAt: &extract.Span{Page: 1, Start: 30, End: 41, Text: "30 APR 2030"}, Current: ptr(true),
			Tiers: map[string]extract.Tier{"holder": "likely", "number": "likely", "expiry": "likely", "current": "likely"}},
	}
}

func sampleRun(docs ...extract.DocOut) *extract.Output {
	return &extract.Output{Format: 3, Extractor: "test", Directory: "on", TextSet: "text",
		Organisations: map[string]extract.Card{
			"IFSC EXMP":                 {Directory: ptr("D1")},
			"name:stmt-jan:examplebank": {Name: ptr("Example Bank"), Directory: ptr("D1")},
			"name:stmt-feb:examplebank": {Name: ptr("Example Bank"), Directory: ptr("D1")},
		},
		Documents: docs}
}

// stores runs a test on the memory store and on SQLite in a temporary
// folder; reopen closes and opens the SQLite store, as each sifs-load does.
func stores(t *testing.T, test func(t *testing.T, ctx context.Context, reopen func() *Engine)) {
	ctx := context.Background()
	t.Run("memory", func(t *testing.T) {
		e, err := OpenMemory(ctx)
		if err != nil {
			t.Fatal(err)
		}
		test(t, ctx, func() *Engine { return e })
	})
	t.Run("sqlite", func(t *testing.T) {
		dir := t.TempDir()
		var e *Engine
		reopen := func() *Engine {
			if e != nil {
				if err := e.Close(); err != nil {
					t.Fatal(err)
				}
			}
			var err error
			if e, err = Open(ctx, dir); err != nil {
				t.Fatal(err)
			}
			return e
		}
		t.Cleanup(func() { e.Close() })
		test(t, ctx, reopen)
	})
}

func mustLoad(t *testing.T, ctx context.Context, e *Engine, run *extract.Output) Report {
	t.Helper()
	rep, err := e.Load(ctx, run)
	if err != nil {
		t.Fatal(err)
	}
	return rep
}

func eq[T any](t *testing.T, what string, got, want T) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		t.Errorf("%s:\n got %v\nwant %v", what, got, want)
	}
}

func mustCount(t *testing.T, ctx context.Context, e *Engine) Counts {
	t.Helper()
	c, err := e.Count(ctx)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

// sorted reads a repeated value as a set: the stores return its values in
// different orders.
func sorted(values []any) []string {
	var out []string
	for _, v := range values {
		out = append(out, v.(string))
	}
	slices.Sort(out)
	return out
}

func relation(t *testing.T, ctx context.Context, e *Engine, id string) *ontology.Relation {
	t.Helper()
	r, err := e.interp.GetRelation(ctx, id)
	if err != nil && !strings.Contains(err.Error(), "not found") {
		t.Fatalf("relation %s: %v", id, err)
	}
	return r
}

var (
	janAccount = IdentifierID("account", "000111222333")
	janHeader  = ref("heldAt", "stmt-jan", janAccount, "name:stmt-jan:examplebank")
	janCode    = ref("heldAt", "stmt-jan", janAccount, "IFSC EXMP")
	janSameOrg = `sameOrg:["stmt-jan","IFSC EXMP","name:stmt-jan:examplebank"]`
)

// Mutation: skip doc.Orgs in Facts, and Mentions counts 0.
func TestFirstLoad(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		rep := mustLoad(t, ctx, e, sampleRun(statement("stmt-jan"), statement("stmt-feb"), passport()))
		eq(t, "report", rep, Report{Written: 3, TurnedDown: []TurnDown{}})
		eq(t, "counts", mustCount(t, ctx, e), Counts{
			Things: map[string]int{"Document": 3, "Organisation": 3, "Identifier": 3, "Statement": 2, "Passport": 1, "Holder": 1},
			Links: map[string]int{"Mentions": 2, "Shows": 5, "HeldAt": 6, "SameOrg": 2, "StatementOf": 2, "StatementAccount": 2, "IssuedBy": 2,
				"PassportOf": 1, "HolderOf": 1, "PassportNumber": 1, "PassportMRZ": 0},
		})
		same := relation(t, ctx, e, janSameOrg)
		if same == nil {
			t.Fatal("no same_org link")
		}
		eq(t, "same_org reasons", sorted(same.Attributes[pack.Reasons]), []string{"directory", "spells-code"})
		eq(t, "same_org tier", same.Attributes[pack.Tier], []any{"certain"})
		eq(t, "header link tier", relation(t, ctx, e, janHeader).Attributes[pack.Tier], []any{"likely"})
	})
}

// Mutation: count every taken record as written in Load, and the second
// load reports 3 written.
func TestSecondLoadUnchanged(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		run := sampleRun(statement("stmt-jan"), statement("stmt-feb"), passport())
		mustLoad(t, ctx, reopen(), run)
		before := mustCount(t, ctx, reopen())
		e := reopen()
		eq(t, "second load", mustLoad(t, ctx, e, run), Report{Unchanged: 3, TurnedDown: []TurnDown{}})
		eq(t, "counts", mustCount(t, ctx, e), before)
	})
}

// Mutation: drop the removal loop in Load, and stmt-jan's record and its
// header link stay.
func TestRemovedDocument(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		mustLoad(t, ctx, reopen(), sampleRun(statement("stmt-jan"), statement("stmt-feb"), passport()))
		e := reopen()
		eq(t, "load without stmt-jan", mustLoad(t, ctx, e, sampleRun(statement("stmt-feb"), passport())),
			Report{Unchanged: 2, Removed: 1, TurnedDown: []TurnDown{}, LeftOut: LeftOut{UnusedOrganisations: 1}})
		docs, err := e.Documents(ctx)
		if err != nil {
			t.Fatal(err)
		}
		slices.Sort(docs)
		eq(t, "documents", docs, []string{"passport-sample", "stmt-feb"})
		for _, id := range []string{janHeader, janCode, janSameOrg} {
			if relation(t, ctx, e, id) != nil {
				t.Errorf("%s stays after its only document went", id)
			}
		}
		for _, id := range []string{janAccount, OrgID("IFSC EXMP")} {
			if thing, err := e.interp.GetEntity(ctx, id); err != nil || thing == nil {
				t.Errorf("%s, which stmt-feb also gives, went: %v", id, err)
			}
		}
		if thing, _ := e.interp.GetEntity(ctx, OrgID("name:stmt-jan:examplebank")); thing != nil {
			t.Error("stmt-jan's own name key stays")
		}
	})
}

// Mutation: skip a document the store already holds in Load, and the old
// header link stays.
func TestChangedDocument(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		mustLoad(t, ctx, reopen(), sampleRun(statement("stmt-jan"), statement("stmt-feb"), passport()))
		changed := statement("stmt-jan")
		changed.Links = changed.Links[:2]
		changed.Links[1].Tier = extract.Certain
		e := reopen()
		eq(t, "load with stmt-jan changed", mustLoad(t, ctx, e, sampleRun(changed, statement("stmt-feb"), passport())), Report{Written: 1, Unchanged: 2, TurnedDown: []TurnDown{}})
		if relation(t, ctx, e, janHeader) != nil {
			t.Error("the header link stmt-jan no longer gives stays")
		}
		code := relation(t, ctx, e, janCode)
		if code == nil {
			t.Fatal("the code link went")
		}
		eq(t, "code link tier", code.Attributes[pack.Tier], []any{"certain"})
	})
}

// Mutation: return false without keeping the turn-down in taken, and the
// record disappears from the report.
func TestTurnedDownRecord(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		bad := statement("stmt-feb")
		bad.Links[0].Tier = "maybe"
		e := reopen()
		rep := mustLoad(t, ctx, e, sampleRun(statement("stmt-jan"), bad, passport()))
		eq(t, "written", [3]int{rep.Written, rep.Unchanged, rep.Removed}, [3]int{2, 0, 0})
		if len(rep.TurnedDown) != 1 || rep.TurnedDown[0].Document != "stmt-feb" || !strings.Contains(rep.TurnedDown[0].Message, "maybe") {
			t.Fatalf("turned down: %+v", rep.TurnedDown)
		}
		docs, _ := e.Documents(ctx)
		slices.Sort(docs)
		eq(t, "documents", docs, []string{"passport-sample", "stmt-jan"})
	})
}

// Mutation: drop the Answers count in Load, and the report says 0 answers
// were left out.
func TestLeftOutRowsCounted(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		jan := statement("stmt-jan")
		jan.Orgs = append(jan.Orgs, extract.OrgOut{Span: span(1, 90, "Example Club")})
		run := sampleRun(jan, passport())
		delete(run.Organisations, "name:stmt-feb:examplebank")
		run.Organisations["BIC EXMPXXYY"] = extract.Card{}
		run.Answers.LatestStatement = []extract.AccountAnswer{{Account: "000111222333", Document: "stmt-jan"}}
		leftOut := LeftOut{UnusedOrganisations: 1, MentionsWithoutOrganisation: 1, Answers: 1}

		e := reopen()
		eq(t, "first load", mustLoad(t, ctx, e, run), Report{Written: 2, TurnedDown: []TurnDown{}, LeftOut: leftOut})
		eq(t, "counts", mustCount(t, ctx, e), Counts{
			Things: map[string]int{"Document": 2, "Organisation": 2, "Identifier": 3, "Statement": 1, "Passport": 1, "Holder": 1},
			Links: map[string]int{"Mentions": 1, "Shows": 3, "HeldAt": 3, "SameOrg": 1, "StatementOf": 1, "StatementAccount": 1, "IssuedBy": 1,
				"PassportOf": 1, "HolderOf": 1, "PassportNumber": 1, "PassportMRZ": 0},
		})
		eq(t, "second load", mustLoad(t, ctx, reopen(), run), Report{Unchanged: 2, TurnedDown: []TurnDown{}, LeftOut: leftOut})
	})
}

// Mutation: make Links query by the wrong type, and the read finds no
// same_org link.
func TestReadThingsAndLinks(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		mustLoad(t, ctx, reopen(), sampleRun(statement("stmt-jan"), passport()))
		e := reopen()
		links, err := e.Links(ctx, pack.SameOrg)
		if err != nil {
			t.Fatal(err)
		}
		if len(links) != 1 || links[0].ID != janSameOrg {
			t.Fatalf("same_org links: %+v", links)
		}
		holders, err := e.Things(ctx, pack.Holder)
		if err != nil {
			t.Fatal(err)
		}
		if len(holders) != 1 || !reflect.DeepEqual(holders[0].Attributes[pack.Surname], []any{"SAMPLE"}) {
			t.Fatalf("holders: %+v", holders)
		}
	})
}

// mergeDoc states two links and two same_org claims twice each: one claim
// of each kind in a certain row and a likely row, the other in two likely
// rows. certainFirst puts the certain rows first; otherwise they come last.
func mergeDoc(certainFirst bool) extract.DocOut {
	name := "name:merge-doc:examplebank"
	doc := extract.DocOut{
		ID:   "merge-doc",
		Orgs: []extract.OrgOut{{Span: span(1, 0, "Example Bank"), Org: ptr(name), SettledBy: ptr("name")}},
		Identifiers: []extract.IdentOut{
			{Span: span(1, 20, "XX00EXMP0001"), ID: "i1", Kind: "iban", Value: "XX00EXMP0001", Valid: ptr(true), Tier: extract.Certain},
			{Span: span(1, 40, "000111222333"), ID: "i2", Kind: "account", Value: "000111222333", Tier: extract.Likely},
			{Span: span(1, 60, "EXMP0000001"), ID: "i3", Kind: "ifsc", Value: "EXMP0000001", Tier: extract.Likely},
			{Span: span(1, 80, "EXMPXXYY"), ID: "i4", Kind: "bic", Value: "EXMPXXYY", Tier: extract.Likely},
		},
		Links: []extract.LinkOut{
			{Identifier: "i1", Org: "IBAN XX EXMP", Reasons: []string{"iban-bank-code"}, Tier: extract.Certain},
			{Identifier: "i1", Org: "IBAN XX EXMP", Reasons: []string{"paired-code"}, Via: "i3", Tier: extract.Likely},
			{Identifier: "i2", Org: "IFSC EXMP", Reasons: []string{"paired-code"}, Via: "i3", Tier: extract.Likely},
			{Identifier: "i2", Org: "IFSC EXMP", Reasons: []string{"same-account"}, Via: "i4", Tier: extract.Likely},
		},
		SameOrg: []extract.SameOrgOut{
			{Keys: [2]string{name, "IFSC EXMP"}, Reason: "directory", Tier: extract.Certain},
			{Keys: [2]string{"IFSC EXMP", name}, Reason: "spells-code", Tier: extract.Likely},
			{Keys: [2]string{name, "BIC EXMPXXYY"}, Reason: "spells-code", Tier: extract.Likely},
			{Keys: [2]string{"BIC EXMPXXYY", name}, Reason: "printed-together", Tier: extract.Likely},
		},
	}
	if !certainFirst {
		slices.Reverse(doc.Links)
		slices.Reverse(doc.SameOrg)
	}
	return doc
}

// Rows with one id are one claim, certain when one row is (sample/FORMAT.md,
// "Tiers" and "Ids"). Mutation: make claim.tier return the lowest tier, and
// the mixed claims come out likely.
func TestMergedClaims(t *testing.T) {
	name := "name:merge-doc:examplebank"
	iban, account := IdentifierID("iban", "XX00EXMP0001"), IdentifierID("account", "000111222333")
	ifsc, bic := IdentifierID("ifsc", "EXMP0000001"), IdentifierID("bic", "EXMPXXYY")
	want := []struct {
		id, tier     string
		reasons, via []string
	}{
		{ref("heldAt", "merge-doc", iban, "IBAN XX EXMP"), "certain", []string{"iban-bank-code", "paired-code"}, sortedIDs(ifsc)},
		{ref("heldAt", "merge-doc", account, "IFSC EXMP"), "likely", []string{"paired-code", "same-account"}, sortedIDs(bic, ifsc)},
		{ref("sameOrg", "merge-doc", "IFSC EXMP", name), "certain", []string{"directory", "spells-code"}, nil},
		{ref("sameOrg", "merge-doc", "BIC EXMPXXYY", name), "likely", []string{"printed-together", "spells-code"}, nil},
	}
	for _, certainFirst := range []bool{true, false} {
		t.Run(map[bool]string{true: "certain first", false: "likely first"}[certainFirst], func(t *testing.T) {
			stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
				e := reopen()
				mustLoad(t, ctx, e, &extract.Output{Format: 3, Documents: []extract.DocOut{mergeDoc(certainFirst)}})
				c := mustCount(t, ctx, e)
				eq(t, "links", [2]int{c.Links["HeldAt"], c.Links["SameOrg"]}, [2]int{2, 2})
				for _, w := range want {
					r := relation(t, ctx, e, w.id)
					if r == nil {
						t.Errorf("no link %s", w.id)
						continue
					}
					eq(t, w.id+" tier", r.Attributes[pack.Tier], []any{w.tier})
					eq(t, w.id+" reasons", sorted(r.Attributes[pack.Reasons]), w.reasons)
					eq(t, w.id+" via", sorted(r.Attributes[pack.Via]), w.via)
				}
			})
		})
	}
}
