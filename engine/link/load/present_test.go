package load

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/pack"
	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/store"
)

// A made-up letter from a French bank that prints its BIC, a British sort
// code, an IBAN, an account, a passport number and a number of a kind the
// pack does not know.
func letter() extract.DocOut {
	name := "name:letter-fr:banqueexemple"
	return extract.DocOut{
		ID:   "letter-fr",
		Orgs: []extract.OrgOut{{Span: span(1, 0, "Banque Exemple"), Org: ptr(name), SettledBy: ptr("name")}},
		Identifiers: []extract.IdentOut{
			{Span: span(1, 20, "EXMPFRPPXXX"), ID: "i1", Kind: "bic", Value: "EXMPFRPPXXX", Tier: extract.Likely},
			{Span: span(1, 40, "99-00-11"), ID: "i2", Kind: "gb_sort_code", Value: "990011", Tier: extract.Likely},
			{Span: span(1, 60, "FR76 0000 0000 0000 0000 0000 097"), ID: "i3", Kind: "iban", Value: "FR7600000000000000000000097", Tier: extract.Likely},
			{Span: span(1, 100, "87654321"), ID: "i4", Kind: "account", Value: "87654321", Tier: extract.Likely},
			{Span: span(1, 120, "X0000002"), ID: "i5", Kind: "passport_number", Value: "X0000002", Tier: extract.Likely},
			{Span: span(1, 140, "M-447120"), ID: "i6", Kind: "xx_member_number", Value: "M-447120", Tier: extract.Likely},
		},
		Links: []extract.LinkOut{
			{Identifier: "i1", Org: "BIC EXMPFRPP", Reasons: []string{"bic-institution"}, Tier: extract.Likely},
			{Identifier: "i3", Org: name, Reasons: []string{"paired-code"}, Via: "i1", Tier: extract.Likely},
		},
		SameOrg: []extract.SameOrgOut{},
	}
}

func page(t *testing.T, ctx context.Context, r *show.Reader, id string) show.Page {
	t.Helper()
	p, err := r.Page(ctx, id)
	if err != nil {
		t.Fatalf("page %s: %v", id, err)
	}
	return p
}

// value is what a page gives under a label: a value of the thing, or of
// the links in the group with that label.
func value(p show.Page, label string) []string {
	var out []string
	for _, v := range p.Values {
		if v.Label == label {
			out = append(out, v.Values...)
		}
	}
	return out
}

func rowValue(p show.Page, group, label string) []string {
	var out []string
	for _, g := range p.Links {
		if g.Label != group {
			continue
		}
		for _, row := range g.Rows {
			for _, a := range row.Attributes {
				if a.Label == label {
					out = append(out, a.Values...)
				}
			}
		}
	}
	return out
}

// A bank code is shown in full, found by its code and not counted among
// accounts; an IBAN, an account, a passport number and a number of a kind
// the pack does not know are masked, on their page and in the printed text
// of the row that shows them; a bank's printed name is shown in full.
// Mutations: put "bic" out of PublicKinds, and the BIC's page is no page;
// mask Mentions' printed text in the pack, and the bank's printed name is
// masked.
func TestPublicShownPrivateMasked(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		mustLoad(t, ctx, reopen(), &extract.Output{Format: 3, Documents: []extract.DocOut{letter()}})
		r := show.New(reopen().interp, false)
		kinds, err := r.Kinds(ctx)
		if err != nil {
			t.Fatal(err)
		}
		count := map[string]string{}
		for _, k := range kinds.Kinds {
			count[k.Label] = fmt.Sprintf("%s %d", k.Collection, k.Count)
		}
		eq(t, "numbers", count["Number"], "accounts 4")
		eq(t, "bank codes", count["Bank code"], "organisations 2")

		for _, c := range []struct{ kind, code, printed string }{{"bic", "EXMPFRPPXXX", "EXMPFRPPXXX"}, {"gb_sort_code", "990011", "99-00-11"}} {
			p := page(t, ctx, r, BankCodeID(c.kind, c.code))
			eq(t, c.code+" title", p.Title, c.code)
			eq(t, c.code+" code", value(p, "Code"), []string{c.code})
			eq(t, c.code+" printed", rowValue(p, "Printed in", "Printed text"), []string{c.printed})
			found, err := r.Find(ctx, c.code)
			if err != nil {
				t.Fatal(err)
			}
			if len(found.Groups) != 1 || len(found.Groups[0].Things) != 1 || found.Groups[0].Things[0].ID != p.ID {
				t.Errorf("find %s: %+v", c.code, found)
			}
		}
		eq(t, "the bank the BIC is the code of", page(t, ctx, r, OrgID("BIC EXMPFRPP")).Title, "BIC EXMPFRPP")

		for _, n := range letter().Identifiers[2:] {
			p := page(t, ctx, r, IdentifierID(n.Kind, n.Value))
			shown := append([]string{p.Title}, value(p, "Value")...)
			shown = append(shown, rowValue(p, "Printed in", "Printed text")...)
			if len(shown) != 3 {
				t.Errorf("%s gives %q", n.Kind, shown)
			}
			for _, s := range shown {
				if strings.Contains(s, n.Value) || strings.Contains(s, n.Text) || !strings.HasPrefix(s, "•") {
					t.Errorf("%s %q is given as %q", n.Kind, n.Value, s)
				}
			}
		}
		eq(t, "the bank's printed name", rowValue(page(t, ctx, r, OrgID("name:letter-fr:banqueexemple")), "Mentioned in", "Printed text"), []string{"Banque Exemple"})
	})
}

// Dates read as the day the run gives, wherever the loader and the reader
// run: two zones a day apart. A passport's expiry the run cannot tell is no
// value and no word; its tier stays. Every organisation, every statement
// and every passport the run prints an expiry for has a title of its own; a
// statement's is its first and last day. Mutations: parse a date in
// time.Local in putDate, and the dates move a day; store `cannot-tell` as
// an expiry, and the passport is turned down; drop "institution" from the
// organisation's title, and three organisations are titled by their id;
// drop the statement's period, and every statement is titled by its id.
func TestDatesAndTitles(t *testing.T) {
	run := committedRun(t)
	defer func(local *time.Location) { time.Local = local }(time.Local)
	for _, zone := range []*time.Location{time.FixedZone("UTC-10", -10*3600), time.FixedZone("UTC+14", 14*3600)} {
		time.Local = zone
		t.Run(zone.String(), func(t *testing.T) {
			stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
				eq(t, "turned down", mustLoad(t, ctx, reopen(), run).TurnedDown, []TurnDown{})
				e := reopen()
				r := show.New(e.interp, false)
				day := func(what string, got []string, want *string) {
					t.Helper()
					if want == nil || *want == "cannot-tell" {
						eq(t, what, got, []string(nil))
						return
					}
					eq(t, what, got, []string{*want})
				}
				titled := map[string]string{}
				for _, doc := range run.Documents {
					if s := doc.Statement; s != nil {
						p := page(t, ctx, r, "statement:"+doc.ID)
						day(doc.ID+" from", value(p, "From"), &s.From)
						day(doc.ID+" to", value(p, "To"), &s.To)
						eq(t, doc.ID+" title", p.Title, s.From+" to "+s.To)
						titled[doc.ID] = p.Title
					}
					if pp := doc.Passport; pp != nil {
						p := page(t, ctx, r, "passport:"+doc.ID)
						day(doc.ID+" issued", value(p, "Issued"), pp.Issued)
						day(doc.ID+" expires", value(p, "Expires"), pp.Expiry)
						eq(t, doc.ID+" expiry tier", value(p, "Expiry tier"), []string{string(pp.Tiers["expiry"])})
						if pp.ExpiryAt != nil {
							eq(t, doc.ID+" title", p.Title, pp.ExpiryAt.Text)
						}
					}
				}
				for _, id := range []string{"natwest-statement", "abn-amro-statement", "commerzbank-kontoauszug", "mizuho-statement-ja", "hdfc-statement-aug"} {
					if titled[id] == "" {
						t.Errorf("the run has no statement %s", id)
					}
				}
				things, err := e.interp.QueryEntities(ctx, store.EntityPattern{})
				if err != nil {
					t.Fatal(err)
				}
				for _, thing := range things {
					for attrID, vs := range thing.Attributes {
						if slices.Contains(vs, any("cannot-tell")) {
							t.Errorf("%s holds cannot-tell under %s", thing.ID, attrID)
						}
					}
					if thing.TypeID == pack.Organisation {
						if p := page(t, ctx, r, thing.ID); p.Title == p.ID {
							t.Errorf("the organisation %s is titled by its id", p.ID)
						}
					}
				}
			})
		})
	}
}

// A statement whose run gives one day is titled by that day, and one with
// none has no title, so the reader titles it by its id. Mutation: join the
// two dates whether or not each is a day, and the titles read " to ".
func TestStatementWithOneDay(t *testing.T) {
	statement := func(id, from, to string) extract.DocOut {
		return extract.DocOut{ID: id, Statement: &extract.StmtOut{From: from, To: to, Tiers: map[string]extract.Tier{"period": extract.Likely}}}
	}
	run := &extract.Output{Format: 3, Documents: []extract.DocOut{
		statement("lloyds-from-only", "2026-05-01", ""),
		statement("ing-to-only", "", "2026-05-31"),
		statement("westpac-none", "", ""),
	}}
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		eq(t, "turned down", mustLoad(t, ctx, reopen(), run).TurnedDown, []TurnDown{})
		r := show.New(reopen().interp, false)
		for _, c := range []struct {
			id, title string
			period    []string
		}{
			{"lloyds-from-only", "2026-05-01", []string{"2026-05-01"}},
			{"ing-to-only", "2026-05-31", []string{"2026-05-31"}},
			{"westpac-none", "statement:westpac-none", nil},
		} {
			p := page(t, ctx, r, "statement:"+c.id)
			eq(t, c.id+" title", p.Title, c.title)
			eq(t, c.id+" period", value(p, "Period"), c.period)
		}
	})
}
