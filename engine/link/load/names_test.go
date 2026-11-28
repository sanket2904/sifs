package load

import (
	"context"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/pack"

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

// sortedIDs are ids in the order the tests compare them in.
func sortedIDs(ids ...string) []string {
	slices.Sort(ids)
	return ids
}

// The four holders the committed run's passports name with the surname
// KALPIT; ERIKSSON's passport is the fifth.
var kalpit = []string{"holder:passport-asha-current", "holder:passport-asha-expired", "holder:passport-meera-1988", "holder:passport-rohan"}

// runNames is every name the committed run gives a thing, by the thing's
// id: an organisation's card name, each text a document prints for its key
// and an institution code's key itself, a number's or a bank code's value,
// a holder's surname and given names.
func runNames(run *extract.Output) map[string][]string {
	names := map[string][]string{}
	for _, doc := range run.Documents {
		var keys []string
		for _, m := range doc.Orgs {
			if m.Org != nil {
				keys = append(keys, *m.Org)
			}
		}
		for _, l := range doc.Links {
			keys = append(keys, l.Org)
		}
		for _, s := range doc.SameOrg {
			keys = append(keys, s.Keys[:]...)
		}
		if s := doc.Statement; s != nil && s.Issuer != "" {
			keys = append(keys, s.Issuer)
		}
		for _, key := range keys {
			if !strings.HasPrefix(key, "name:") {
				names[OrgID(key)] = append(names[OrgID(key)], key)
			}
		}
	}
	for key, card := range run.Organisations {
		if card.Name != nil {
			names[OrgID(key)] = append(names[OrgID(key)], *card.Name)
		}
	}
	for _, doc := range run.Documents {
		for _, m := range doc.Orgs {
			if m.Org != nil {
				names[OrgID(*m.Org)] = append(names[OrgID(*m.Org)], m.Span.Text)
			}
		}
		for _, ident := range doc.Identifiers {
			id := IdentifierID(ident.Kind, ident.Value)
			if pack.Public(ident.Kind) {
				id = BankCodeID(ident.Kind, ident.Value)
			}
			names[id] = append(names[id], ident.Value)
		}
		if p := doc.Passport; p != nil {
			names["holder:"+doc.ID] = []string{p.Holder.Surname, p.Holder.Given}
		}
	}
	return names
}

// The engine's own name lookup on the committed run. Each expectation is
// read from the run file: the organisations whose card name, printed text
// or institution code's key starts with the typed word (case aside), the
// numbers and bank codes whose value does, the holders whose surname or
// given names do. Mutations: drop Alias from
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
		{"hdfc", sortedIDs(BankCodeID("ifsc", "HDFC0SAMPL1"), BankCodeID("ifsc", "HDFC0SAMPL3"),
			OrgID("IFSC HDFC"), OrgID("name:email-fee-payment:hdfcbank"), OrgID("name:hdfc-branch-letter:hdfcbank"), OrgID("name:hdfc-card-statement:hdfcbank"),
			OrgID("name:hdfc-life-receipt:hdfclifeinsurancecompany"), OrgID("name:hdfc-ltd-loan-2019:housingdevelopmentfinancecorporation"),
			OrgID("name:hdfc-statement-aug:hdfcbank"), OrgID("name:hdfc-statement-jul:hdfcbank"), OrgID("name:hdfc-statement-sep:hdfcbank"), OrgID("name:neft-receipt-tied:hdfcbank"))},
		{"HDFC Bank", sortedIDs(OrgID("IFSC HDFC"), OrgID("name:email-fee-payment:hdfcbank"), OrgID("name:hdfc-branch-letter:hdfcbank"), OrgID("name:hdfc-card-statement:hdfcbank"),
			OrgID("name:hdfc-statement-aug:hdfcbank"), OrgID("name:hdfc-statement-jul:hdfcbank"), OrgID("name:hdfc-statement-sep:hdfcbank"), OrgID("name:neft-receipt-tied:hdfcbank"))},
		{"HDFC Ltd", sortedIDs(OrgID("name:hdfc-ltd-loan-2019:housingdevelopmentfinancecorporation"))},
		{"KALPIT", kalpit},
		{"anna maria", []string{"holder:passport-bad-mrz"}},
		{"50100417263958", sortedIDs(IdentifierID("account", "50100417263958"))},
		{"SMBK", sortedIDs(BankCodeID("bic", "SMBKCAT0XXX"), BankCodeID("bic", "SMBKGB20XXX"), OrgID("name:credit-advice-smbk-ca:smbk"), OrgID("name:payroll-release-smbk-gb:smbk"))},
		{"BIC SMBKCAT0", sortedIDs(OrgID("BIC SMBKCAT0"))},
		{"COBADEFFXXX", sortedIDs(BankCodeID("bic", "COBADEFFXXX"))},
		{"601613", sortedIDs(BankCodeID("gb_sort_code", "601613"))},
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
				eq(t, "hdfc exactly", exact, []string{OrgID("name:neft-receipt-tied:hdfcbank")})
			case "50100417263958":
				eq(t, "the account exactly", exact, q.want)
			}
		}
	})
}
