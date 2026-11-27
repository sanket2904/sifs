package load

import (
	"context"
	"slices"
	"testing"

	"github.com/sanket2904/sifs/engine/extract"

	"github.com/sanket2904/ontology-framework/interpreter"
)

// committedRun is the run sample/README.md loads.
func committedRun(t *testing.T) *extract.Output {
	t.Helper()
	run, err := ReadRun("../../../sample/runs/go-m1a-on-text.json")
	if err != nil {
		t.Fatal(err)
	}
	return run
}

// resolve is the engine's own name lookup for a typed word: the ids it
// finds, sorted, with no limit the sample can reach.
func resolve(t *testing.T, ctx context.Context, e *Engine, query string) []string {
	t.Helper()
	found, err := e.interp.Resolve(ctx, query, interpreter.ResolveOptions{Limit: 1000})
	if err != nil {
		t.Fatal(err)
	}
	var ids []string
	for _, c := range found {
		ids = append(ids, c.EntityID)
	}
	slices.Sort(ids)
	return ids
}

// The committed run's rows the loader leaves out (sample/README.md).
var committedLeftOut = LeftOut{MentionsWithoutOrganisation: 1, Answers: 16}

// The four holders the committed run's passports name with the surname
// KALPIT; ERIKSSON's passport is the fifth.
var kalpit = []string{"holder:passport-asha-current", "holder:passport-asha-expired", "holder:passport-meera-1988", "holder:passport-rohan"}

// runNames is every name the committed run gives a thing, by the thing's
// id: an organisation's card name and each text a document prints for its
// key, an identifier's value, a holder's surname and given names.
func runNames(run *extract.Output) map[string][]string {
	names := map[string][]string{}
	for key, card := range run.Organisations {
		if card.Name != nil {
			names["org:"+key] = append(names["org:"+key], *card.Name)
		}
	}
	for _, doc := range run.Documents {
		for _, m := range doc.Orgs {
			if m.Org != nil {
				names["org:"+*m.Org] = append(names["org:"+*m.Org], m.Span.Text)
			}
		}
		for _, ident := range doc.Identifiers {
			id := "identifier:" + ident.Kind + ":" + ident.Value
			names[id] = append(names[id], ident.Value)
		}
		if p := doc.Passport; p != nil {
			names["holder:"+doc.ID] = []string{p.Holder.Surname, p.Holder.Given}
		}
	}
	return names
}

// The engine's own name lookup on the committed run. Each expectation is
// read from the run file: the organisations whose card name or printed text
// starts with the typed word (case aside), the identifiers whose value does,
// the holders whose surname or given names do. Mutations: drop Alias from
// the holder's surname in the pack, and kalpit finds nothing; drop the
// printed names in Facts, and HDFC Ltd finds nothing and hdfc finds no exact
// match.
func TestNames(t *testing.T) {
	run := committedRun(t)
	given := runNames(run)
	queries := []struct {
		query string
		want  []string
	}{
		{"hdfc", []string{"identifier:ifsc:HDFC0SAMPL1", "identifier:ifsc:HDFC0SAMPL3",
			"org:IFSC HDFC", "org:name:email-fee-payment:hdfcbank", "org:name:hdfc-branch-letter:hdfcbank", "org:name:hdfc-card-statement:hdfcbank",
			"org:name:hdfc-life-receipt:hdfclifeinsurancecompany", "org:name:hdfc-ltd-loan-2019:housingdevelopmentfinancecorporation",
			"org:name:hdfc-statement-aug:hdfcbank", "org:name:hdfc-statement-jul:hdfcbank", "org:name:hdfc-statement-sep:hdfcbank", "org:name:neft-receipt-tied:hdfcbank"}},
		{"HDFC Bank", []string{"org:IFSC HDFC", "org:name:email-fee-payment:hdfcbank", "org:name:hdfc-branch-letter:hdfcbank", "org:name:hdfc-card-statement:hdfcbank",
			"org:name:hdfc-statement-aug:hdfcbank", "org:name:hdfc-statement-jul:hdfcbank", "org:name:hdfc-statement-sep:hdfcbank", "org:name:neft-receipt-tied:hdfcbank"}},
		{"HDFC Ltd", []string{"org:name:hdfc-ltd-loan-2019:housingdevelopmentfinancecorporation"}},
		{"KALPIT", kalpit},
		{"anna maria", []string{"holder:passport-bad-mrz"}},
		{"50100417263958", []string{"identifier:account:50100417263958"}},
		{"SMBK", []string{"identifier:bic:SMBKCAT0XXX", "identifier:bic:SMBKGB20XXX", "org:name:credit-advice-smbk-ca:smbk", "org:name:payroll-release-smbk-gb:smbk"}},
		{"Zebrafinch Savings", nil},
	}
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		mustLoad(t, ctx, reopen(), run)
		e := reopen()
		for _, q := range queries {
			found, err := e.interp.Resolve(ctx, q.query, interpreter.ResolveOptions{Limit: 1000})
			if err != nil {
				t.Fatal(err)
			}
			var ids, exact []string
			for _, c := range found {
				ids = append(ids, c.EntityID)
				if c.Exact {
					exact = append(exact, c.EntityID)
				}
				if !slices.Contains(given[c.EntityID], c.Name) {
					t.Errorf("%q finds %s by %q, which the run does not give it", q.query, c.EntityID, c.Name)
				}
			}
			slices.Sort(ids)
			eq(t, q.query, ids, q.want)
			switch q.query {
			case "hdfc":
				eq(t, "hdfc exactly", exact, []string{"org:name:neft-receipt-tied:hdfcbank"})
			case "50100417263958":
				eq(t, "the account exactly", exact, q.want)
			}
		}
	})
}
