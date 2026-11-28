package load

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/pack"
)

// latestOf is, by account, each statement the rules call that account's
// latest, as "<document> certain" or "<document> probably".
func latestOf(t *testing.T, ctx context.Context, e *Engine) map[string][]string {
	t.Helper()
	out := map[string][]string{}
	for typeID, tier := range map[string]string{pack.LatestStatement: "certain", pack.ProbablyLatestStatement: "probably"} {
		links, err := e.Links(ctx, typeID)
		if err != nil {
			t.Fatalf("links %s: %v", typeID, err)
		}
		for _, l := range links {
			ends := map[string]string{}
			for _, p := range l.Players {
				ends[p.RoleTypeID] = p.PlayerID
			}
			account := ends[typeID+":account"]
			out[account] = append(out[account], strings.TrimPrefix(ends[typeID+":statement"], "statement:")+" "+tier)
		}
	}
	for _, docs := range out {
		slices.Sort(docs)
	}
	return out
}

// statementsOf are, by account, the statements a run gives it.
func statementsOf(run *extract.Output) map[string][]extract.DocOut {
	out := map[string][]extract.DocOut{}
	for _, doc := range run.Documents {
		if doc.Statement == nil {
			continue
		}
		for _, a := range doc.Statement.Accounts {
			for _, ident := range doc.Identifiers {
				if ident.ID == a {
					id := IdentifierID(ident.Kind, ident.Value)
					out[id] = append(out[id], doc)
				}
			}
		}
	}
	return out
}

// sureness is how sure a statement's claims make its being an account's
// latest: certain when its period and its accounts are, probably when
// either is only likely.
func sureness(doc extract.DocOut) string {
	if doc.Statement.Tiers["period"] == extract.Certain && doc.Statement.Tiers["accounts"] == extract.Certain {
		return "certain"
	}
	return "probably"
}

// runAnswers are, by account, the latest statement the run's own answers
// name, with how sure that statement's claims are. An answer names an
// account by any spelling of it.
func runAnswers(run *extract.Output) map[string][]string {
	byAccount := statementsOf(run)
	out := map[string][]string{}
	for _, a := range run.Answers.LatestStatement {
		for id, docs := range byAccount {
			if !spells(run, id, a.Account) {
				continue
			}
			for _, doc := range docs {
				if doc.ID == a.Document {
					out[id] = append(out[id], doc.ID+" "+sureness(doc))
				}
			}
		}
	}
	return out
}

func spells(run *extract.Output, id, account string) bool {
	squeeze := func(s string) string { return strings.ReplaceAll(s, " ", "") }
	for _, doc := range run.Documents {
		for _, ident := range doc.Identifiers {
			if IdentifierID(ident.Kind, ident.Value) == id && squeeze(ident.Value) == squeeze(account) {
				return true
			}
		}
	}
	return false
}

// byPeriods are, by account, the statements of it that end last, read off
// the documents' periods, when every statement of it has an end that is a
// day.
func byPeriods(run *extract.Output) map[string][]string {
	out := map[string][]string{}
	for id, docs := range statementsOf(run) {
		last, known := "", true
		for _, doc := range docs {
			known = known && isDay(doc.Statement.To)
			last = max(last, doc.Statement.To)
		}
		if !known {
			continue
		}
		for _, doc := range docs {
			if doc.Statement.To == last {
				out[id] = append(out[id], doc.ID+" "+sureness(doc))
			}
		}
		slices.Sort(out[id])
	}
	return out
}

// Mutation: compare the other way round in the rule for a later statement
// (an earlier end instead of a later one), and every account with two or
// more statements gets its earliest.
func TestLatestStatementIsTheRunsAnswer(t *testing.T) {
	run := committedRun(t)
	answers, periods := runAnswers(run), byPeriods(run)
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		if rep := mustLoad(t, ctx, e, run); len(rep.TurnedDown) > 0 {
			t.Fatalf("turned down: %+v", rep.TurnedDown)
		}
		got := latestOf(t, ctx, e)
		accounts := statementsOf(run)
		if len(accounts) != 13 || len(answers) != 12 {
			t.Fatalf("the sample has %d accounts with a statement and %d answers, want 13 and 12", len(accounts), len(answers))
		}
		var table []string
		for id, docs := range accounts {
			want, from := answers[id], "the run's answer"
			if want == nil {
				want, from = periods[id], "the documents' periods"
			}
			eq(t, fmt.Sprintf("latest of %s (%d statements, by %s)", id, len(docs), from), got[id], want)
			table = append(table, fmt.Sprintf("%s: %d statements, rules %v, %s %v", id, len(docs), got[id], from, want))
		}
		slices.Sort(table)
		t.Log("\n" + strings.Join(table, "\n"))
		for id := range got {
			if accounts[id] == nil {
				t.Errorf("a latest statement for %s, which has no statement", id)
			}
		}
	})
}
