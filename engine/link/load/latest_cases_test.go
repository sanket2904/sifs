package load

import (
	"context"
	"fmt"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/pack"
	"github.com/sanket2904/sifs/engine/link/show"
)

// Made-up accounts of banks in several countries, as the committed run
// writes such numbers.
var (
	deAccount = extract.IdentOut{Kind: "iban", Value: "DE02100500000024290661"}
	nlAccount = extract.IdentOut{Kind: "iban", Value: "NL02ABNA0123456789"}
	jpAccount = extract.IdentOut{Kind: "account", Value: "7654321"}
	frAccount = extract.IdentOut{Kind: "iban", Value: "FR7630006000011234567890189"}
	gbAccount = extract.IdentOut{Kind: "iban", Value: "GB33BUKB20201555555555"}
	gbNumber  = extract.IdentOut{Kind: "account", Value: "55555555"}
	inAccount = extract.IdentOut{Kind: "account", Value: "00011122233344"}
)

func accountID(a extract.IdentOut) string { return IdentifierID(a.Kind, a.Value) }

// madeStatement is a made-up statement of accounts, its period's tier and
// its accounts' tier as given. An end that is not a day is one the
// extractor could not read.
func madeStatement(id, from, to, periodTier, accountsTier string, accounts ...extract.IdentOut) extract.DocOut {
	doc := extract.DocOut{ID: id, File: "statements/" + id + ".pdf", Orgs: []extract.OrgOut{}, Links: []extract.LinkOut{}, SameOrg: []extract.SameOrgOut{}}
	var ids []string
	for i, a := range accounts {
		a.ID = fmt.Sprintf("i%d", i+1)
		a.Span = span(1, 40*i, a.Value)
		a.Tier = extract.Likely
		a.Text = a.Value
		doc.Identifiers = append(doc.Identifiers, a)
		ids = append(ids, a.ID)
	}
	doc.Statement = &extract.StmtOut{From: from, To: to, Accounts: ids,
		Tiers: map[string]extract.Tier{"period": extract.Tier(periodTier), "accounts": extract.Tier(accountsTier)}}
	return doc
}

func likely(id, from, to string, accounts ...extract.IdentOut) extract.DocOut {
	return madeStatement(id, from, to, "likely", "likely", accounts...)
}

func madeRun(docs ...extract.DocOut) *extract.Output {
	return &extract.Output{Format: 3, Extractor: "test", Organisations: map[string]extract.Card{}, Documents: docs}
}

// derived is every derived link the store holds, as "<link> <statement's
// document> <account>", sorted.
func derived(t *testing.T, ctx context.Context, e *Engine) []string {
	t.Helper()
	var out []string
	for _, typeID := range pack.Derived {
		links, err := e.Links(ctx, typeID)
		if err != nil {
			t.Fatalf("links %s: %v", typeID, err)
		}
		for _, l := range links {
			ends := map[string]string{}
			for _, p := range l.Players {
				ends[strings.TrimPrefix(p.RoleTypeID, typeID+":")] = p.PlayerID
			}
			out = append(out, fmt.Sprintf("%s %s %s", strings.TrimPrefix(typeID, "files."), strings.TrimPrefix(ends["statement"], "statement:"), ends["account"]))
		}
	}
	slices.Sort(out)
	return out
}

func fact(link, doc string, account extract.IdentOut) string {
	return fmt.Sprintf("%s %s %s", link, doc, accountID(account))
}

func loaded(t *testing.T, ctx context.Context, e *Engine, run *extract.Output) {
	t.Helper()
	if rep := mustLoad(t, ctx, e, run); len(rep.TurnedDown) > 0 {
		t.Fatalf("turned down: %+v", rep.TurnedDown)
	}
}

func derivedFacts(want ...string) []string {
	slices.Sort(want)
	return want
}

// rows are the titles of the things on a page's rows under a label.
func rows(p show.Page, label string) []string {
	var out []string
	for _, g := range p.Links {
		if g.Label != label {
			continue
		}
		for _, row := range g.Rows {
			for _, end := range row.Ends {
				out = append(out, end.Thing.Title)
			}
		}
	}
	return out
}

// A later statement of a German account whose end cannot be read may be
// the latest: no statement of that account is called the latest, and the
// one whose end is known says another's is not. A Dutch account beside it
// keeps its latest.
// Mutation: drop the rule for an end that is not known, and the German
// August statement is called the latest.
func TestLaterEndNotKnown(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		loaded(t, ctx, e, madeRun(
			likely("kontoauszug-08", "2026-08-01", "2026-08-31", deAccount),
			likely("kontoauszug-09", "2026-09-01", "cannot-tell", deAccount),
			likely("afschrift-08", "2026-08-01", "2026-08-31", nlAccount),
			likely("afschrift-09", "2026-09-01", "2026-09-30", nlAccount),
		))
		eq(t, "derived", derived(t, ctx, e), derivedFacts(
			fact("EndNotKnown", "kontoauszug-08", deAccount),
			fact("LaterStatement", "afschrift-08", nlAccount),
			fact("ProbablyLatestStatement", "afschrift-09", nlAccount),
		))
	})
}

// Two statements of a French account that end the same day are both its
// latest, and each says it ends the same day as another.
// Mutation: drop the rule for the same end, and neither says so.
func TestSameEndBothLatest(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		loaded(t, ctx, e, madeRun(
			likely("releve-09", "2026-09-01", "2026-09-30", frAccount),
			likely("releve-09-copie", "2026-09-01", "2026-09-30", frAccount),
			likely("releve-08", "2026-08-01", "2026-08-31", frAccount),
		))
		eq(t, "derived", derived(t, ctx, e), derivedFacts(
			fact("LaterStatement", "releve-08", frAccount),
			fact("ProbablyLatestStatement", "releve-09", frAccount),
			fact("ProbablyLatestStatement", "releve-09-copie", frAccount),
			fact("SameEnd", "releve-09", frAccount),
			fact("SameEnd", "releve-09-copie", frAccount),
		))
	})
}

// Two statements of a Japanese account with the same end and different
// starts: the rules compare ends, so both are the latest and both say so.
func TestSameEndDifferentStarts(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		loaded(t, ctx, e, madeRun(
			likely("torihiki-09", "2026-09-01", "2026-09-30", jpAccount),
			likely("torihiki-09-late-half", "2026-09-16", "2026-09-30", jpAccount),
		))
		eq(t, "derived", derived(t, ctx, e), derivedFacts(
			fact("ProbablyLatestStatement", "torihiki-09", jpAccount),
			fact("ProbablyLatestStatement", "torihiki-09-late-half", jpAccount),
			fact("SameEnd", "torihiki-09", jpAccount),
			fact("SameEnd", "torihiki-09-late-half", jpAccount),
		))
	})
}

// One British statement covers an IBAN and a domestic number. The IBAN has
// a later statement of its own; the number has not. The statement is the
// latest of the number and not of the IBAN, and each account's page says
// its own latest.
// Mutation: make the latest a fact of the statement alone (the account
// dropped from the later statement's rule), and the statement is the
// latest of neither.
func TestStatementOfTwoAccounts(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		loaded(t, ctx, e, madeRun(
			likely("statement-gb-09", "2026-09-01", "2026-09-30", gbAccount, gbNumber),
			likely("statement-gb-10", "2026-10-01", "2026-10-31", gbAccount),
			likely("kontoauszug-10", "2026-10-01", "2026-10-31", deAccount),
		))
		eq(t, "derived", derived(t, ctx, e), derivedFacts(
			fact("LaterStatement", "statement-gb-09", gbAccount),
			fact("ProbablyLatestStatement", "statement-gb-09", gbNumber),
			fact("ProbablyLatestStatement", "statement-gb-10", gbAccount),
			fact("ProbablyLatestStatement", "kontoauszug-10", deAccount),
		))
		r := show.New(reopen().interp, false)
		eq(t, "the IBAN's latest", rows(page(t, ctx, r, accountID(gbAccount)), "Probably the latest statement"), []string{"2026-10-01 to 2026-10-31"})
		eq(t, "the number's latest", rows(page(t, ctx, r, accountID(gbNumber)), "Probably the latest statement"), []string{"2026-09-01 to 2026-09-30"})
		sep := page(t, ctx, r, "statement:statement-gb-09")
		eq(t, "the September statement is the latest of", rows(sep, "Probably the latest statement of"), []string{"••••5555"})
		eq(t, "and not the latest of", rows(sep, "Not the latest statement of"), []string{"••••••••••••••••••5555"})
	})
}

// An account with a single statement: it is the latest. An account whose
// statements have no end that can be read: none is.
func TestSingleAndNoEnd(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		loaded(t, ctx, e, madeRun(
			likely("afschrift-10", "2026-10-01", "2026-10-31", nlAccount),
			likely("releve-a", "2026-08-01", "cannot-tell", frAccount),
			likely("releve-b", "2026-09-01", "cannot-tell", frAccount),
		))
		eq(t, "derived", derived(t, ctx, e), derivedFacts(
			fact("EndNotKnown", "releve-a", frAccount),
			fact("EndNotKnown", "releve-b", frAccount),
			fact("ProbablyLatestStatement", "afschrift-10", nlAccount),
		))
	})
}

// The latest statement's document is removed and the run loaded again:
// the one before it becomes the latest and nothing of the removed one
// remains; the whole run loaded again gives back the first facts.
func TestLatestRemovedAndBack(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		docs := []extract.DocOut{
			likely("kontoauszug-08", "2026-08-01", "2026-08-31", deAccount),
			likely("kontoauszug-09", "2026-09-01", "2026-09-30", deAccount),
			likely("torihiki-08", "2026-08-01", "2026-08-31", jpAccount),
			likely("statement-in-09", "2026-09-01", "2026-09-30", inAccount),
		}
		loaded(t, ctx, reopen(), madeRun(docs...))
		first := derived(t, ctx, reopen())
		eq(t, "first", first, derivedFacts(
			fact("LaterStatement", "kontoauszug-08", deAccount),
			fact("ProbablyLatestStatement", "kontoauszug-09", deAccount),
			fact("ProbablyLatestStatement", "torihiki-08", jpAccount),
			fact("ProbablyLatestStatement", "statement-in-09", inAccount),
		))
		without := slices.Delete(slices.Clone(docs), 1, 2)
		loaded(t, ctx, reopen(), madeRun(without...))
		eq(t, "without the September statement", derived(t, ctx, reopen()), derivedFacts(
			fact("ProbablyLatestStatement", "kontoauszug-08", deAccount),
			fact("ProbablyLatestStatement", "torihiki-08", jpAccount),
			fact("ProbablyLatestStatement", "statement-in-09", inAccount),
		))
		loaded(t, ctx, reopen(), madeRun(docs...))
		eq(t, "the whole run again", derived(t, ctx, reopen()), first)
	})
}

// The latest is certain only when its period and its link to the account
// both are; when either is likely it is probably the latest.
// Mutation: let the certain rule ask for the period alone, and the Dutch
// statement with a likely account link is called certain.
func TestLatestNoSurerThanItsPremises(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		loaded(t, ctx, e, madeRun(
			madeStatement("kontoauszug-10", "2026-10-01", "2026-10-31", "certain", "certain", deAccount),
			madeStatement("afschrift-10", "2026-10-01", "2026-10-31", "certain", "likely", nlAccount),
			madeStatement("releve-10", "2026-10-01", "2026-10-31", "likely", "certain", frAccount),
		))
		eq(t, "derived", derived(t, ctx, e), derivedFacts(
			fact("LatestStatement", "kontoauszug-10", deAccount),
			fact("ProbablyLatestStatement", "afschrift-10", nlAccount),
			fact("ProbablyLatestStatement", "releve-10", frAccount),
		))
	})
}

// A statement whose period ends after today is the latest when it ends
// last: the rules compare the statements' periods, never the clock, so the
// answer is the same on any day the run is loaded.
func TestFutureStatementComparedByPeriod(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		loaded(t, ctx, e, madeRun(
			likely("torihiki-10", "2026-10-01", "2026-10-31", jpAccount),
			likely("torihiki-2099", "2099-01-01", "2099-01-31", jpAccount),
		))
		eq(t, "derived", derived(t, ctx, e), derivedFacts(
			fact("LaterStatement", "torihiki-10", jpAccount),
			fact("ProbablyLatestStatement", "torihiki-2099", jpAccount),
		))
	})
}

// An account whose one statement has no end that can be read gets no
// latest; a Dutch account beside it keeps its own.
// Mutation: drop "has an end" from the rule for the latest, and the
// Japanese statement is called the latest.
func TestOneStatementNoEnd(t *testing.T) {
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		e := reopen()
		loaded(t, ctx, e, madeRun(
			likely("torihiki-no-end", "2026-08-01", "cannot-tell", jpAccount),
			likely("afschrift-08", "2026-08-01", "2026-08-31", nlAccount),
		))
		eq(t, "derived", derived(t, ctx, e), derivedFacts(
			fact("ProbablyLatestStatement", "afschrift-08", nlAccount),
		))
	})
}
