package main

import (
	"encoding/json"
	"fmt"
	"reflect"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/link/load"
	"github.com/sanket2904/sifs/engine/link/show"
)

func shownPage(t *testing.T, dir, id string) show.Page {
	t.Helper()
	code, stdout, stderr := sifsShow(t, "--data-dir", dir, "--page", id)
	if code != 0 {
		t.Fatalf("page %s: exit %d: %s", id, code, stderr)
	}
	var p show.Page
	if err := json.Unmarshal([]byte(stdout), &p); err != nil {
		t.Fatal(err)
	}
	return p
}

// rowTo is the one row a page gives under a label to the thing titled
// end.
func rowTo(t *testing.T, p show.Page, label, end string) show.Link {
	t.Helper()
	var found []show.Link
	for _, g := range p.Links {
		for _, row := range g.Rows {
			if g.Label == label && title(row) == end {
				found = append(found, row)
			}
		}
	}
	if len(found) != 1 {
		t.Fatalf("%s: %d rows under %q to %s, want 1", p.Title, len(found), label, end)
	}
	return found[0]
}

// why is a row's proof as a person reads it: what it rests on, then what
// the rule found none of.
func why(row show.Link) []string {
	if row.Proof == nil {
		return nil
	}
	var out []string
	for _, b := range row.Proof.Because {
		out = append(out, b.Fact)
	}
	for _, r := range row.Proof.RuledOut {
		out = append(out, r.Line)
	}
	return out
}

func eq[T any](t *testing.T, what string, got, want T) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		t.Errorf("%s:\n got %v\nwant %v", what, got, want)
	}
}

func title(row show.Link) string { return row.Ends[0].Thing.Title }

// The latest statement of a German, a Dutch, a Japanese and a British
// account, read through sifs-show on the committed sample: the account's
// page gives it as a row with the statement's period, the statement's page
// gives the account, and each row's why says the statement belongs to the
// account, how sure its period is, that it has an end, and that the rule
// found no later statement of the account and no other whose end is not
// known.
// Mutation: swap the two ends' labels of the latest statement, and the
// account's page has no "Probably the latest statement" row.
func TestLatestStatementReads(t *testing.T) {
	dir := loaded(t)
	for _, c := range []struct{ country, kind, value, statement, period, masked string }{
		{"German", "iban", "DE89370400440532013000", "commerzbank-kontoauszug", "2026-10-01 to 2026-10-31", "••••••••••••••••••3000"},
		{"Dutch", "iban", "NL91ABNA0417164300", "abn-amro-statement", "2026-08-01 to 2026-08-31", "••••••••••••••4300"},
		{"Japanese", "account", "12345678", "mizuho-statement-ja", "2026-08-01 to 2026-08-31", "••••5678"},
		{"British", "iban", "GB29NWBK60161331926819", "natwest-statement", "2026-09-01 to 2026-09-30", "••••••••••••••••••6819"},
	} {
		t.Run(c.country, func(t *testing.T) {
			account := shownPage(t, dir, load.IdentifierID(c.kind, c.value))
			statement := shownPage(t, dir, "statement:"+c.statement)
			want := []string{
				fmt.Sprintf("Statement account: Statement %s, Account %s", c.period, c.masked),
				fmt.Sprintf("%s Period tier likely", c.period),
				fmt.Sprintf("%s has End date", c.period),
				fmt.Sprintf("and no: A later statement exists (for %s, %s)", c.masked, c.period),
				fmt.Sprintf("and no: Another statement's end is not known (for %s, %s)", c.masked, c.period),
			}
			var latest []string
			for _, g := range account.Links {
				for _, row := range g.Rows {
					if strings.Contains(g.Label, "latest statement") {
						latest = append(latest, g.Label+": "+title(row))
					}
				}
			}
			eq(t, "the account's latest", latest, []string{"Probably the latest statement: " + c.period})
			for _, side := range []struct {
				page        show.Page
				label, ends string
			}{{account, "Probably the latest statement", c.period}, {statement, "Probably the latest statement of", c.masked}} {
				row := rowTo(t, side.page, side.label, side.ends)
				eq(t, side.page.Title+" why", why(row), want)
				if len(row.Sources) > 0 || row.Proof.Rule != "files.probably-latest-statement" {
					t.Errorf("%s: sources %v, rule %q: want a derived row", side.page.Title, row.Sources, row.Proof.Rule)
				}
			}
		})
	}
}

// An older statement's page says it is not the latest of its account, and
// its why names the later statement and both ends.
func TestOlderStatementNamesTheLater(t *testing.T) {
	dir := loaded(t)
	aug := shownPage(t, dir, "statement:hdfc-statement-aug")
	row := rowTo(t, aug, "Not the latest statement of", "••••••••••3958")
	got := why(row)
	slices.Sort(got)
	eq(t, "why", got, []string{
		"2026-08-01 to 2026-08-31 End date 2026-08-31",
		"2026-09-01 to 2026-09-30 End date 2026-09-30",
		"Statement account: Statement 2026-08-01 to 2026-08-31, Account ••••••••••3958",
		"Statement account: Statement 2026-09-01 to 2026-09-30, Account ••••••••••3958",
	})
	for _, g := range aug.Links {
		if strings.Contains(g.Label, "latest statement of") && g.Label != "Not the latest statement of" {
			t.Errorf("the August statement reads %q", g.Label)
		}
	}
	account := shownPage(t, dir, load.IdentifierID("account", "50100417263958"))
	var older []string
	for _, g := range account.Links {
		if g.Label == "Older statements" {
			for _, r := range g.Rows {
				older = append(older, title(r))
			}
		}
	}
	eq(t, "older statements", older, []string{"2026-07-01 to 2026-07-31", "2026-08-01 to 2026-08-31"})
}
