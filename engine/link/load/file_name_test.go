package load

import (
	"context"
	"maps"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/pack"
	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/schema"
)

// moduleAt2afc7bb is the files pack as it was before a document had a file
// name, copied from pack.Module at 2afc7bb, so a test can fill a store the
// way a person's store was filled.
func moduleAt2afc7bb() *schema.ModuleDef {
	tier := schema.Annotations(ontology.ValuesAnnotation("certain", "likely"))
	m := schema.NewModule("files", "1")
	label, hidden, masked, names, shownAs := schema.Label, schema.Hidden(), schema.Masked(), schema.Alias(), schema.ShownAs
	one, date := schema.One(), schema.Shown(ontology.DisplayDate)

	m.Entity("Document").Label("Document").
		Display(schema.Collection(ontology.CollectionDocuments))
	m.Entity("Organisation").Label("Organisation").
		Attribute("key", "string", schema.Required(), label("Key"), hidden).
		Attribute("name", "string", label("Name"), names).
		Attribute("printed", "string", schema.Repeated(), label("Printed as"), names).
		Attribute("institution", "string", label("Institution code"), names).
		Attribute("directory", "string", label("Directory entry"), hidden).
		Display(schema.Title("name", "printed", "institution"), schema.Subtitle("printed"), schema.Collection(ontology.CollectionOrganisations))
	m.Entity("Identifier").Label("Number").
		Attribute("kind", "string", schema.Required(), label("Kind")).
		Attribute("value", "string", schema.Required(), label("Value"), names, masked).
		Display(schema.Title("value"), schema.Subtitle("kind"), schema.Collection(ontology.CollectionAccounts))
	m.Entity("BankCode").Label("Bank code").
		Attribute("kind", "string", schema.Required(), label("Kind")).
		Attribute("code", "string", schema.Required(), label("Code"), names).
		Display(schema.Title("code"), schema.Subtitle("kind"), schema.Collection(ontology.CollectionOrganisations))
	m.Entity("Statement").Label("Statement").
		Attribute("from", "timestamp", label("From"), date).
		Attribute("to", "timestamp", label("To"), date).
		Attribute("period", "string", label("Period")).
		Attribute("periodTier", "string", tier, label("Period tier")).
		Display(schema.Title("period"), schema.Collection(ontology.CollectionDocuments))
	m.Entity("Passport").Label("Passport").
		Attribute("issued", "timestamp", label("Issued"), date).
		Attribute("expiry", "timestamp", label("Expires"), date).
		Attribute("expiryTier", "string", tier, label("Expiry tier")).
		Attribute("expiryPage", "int64", label("Expiry page"), hidden).
		Attribute("expiryStart", "int64", label("Expiry start"), hidden).
		Attribute("expiryEnd", "int64", label("Expiry end"), hidden).
		Attribute("expiryText", "string", label("Expiry as printed")).
		Attribute("expiryDay", "string", label("Expiry day")).
		Attribute("current", "bool", label("Current")).
		Attribute("currentTier", "string", tier, label("Current tier")).
		Display(schema.Title("expiryText", "expiryDay"), schema.Collection(ontology.CollectionDocuments))
	m.Entity("Holder").Label("Passport holder").
		Attribute("surname", "string", label("Surname"), names).
		Attribute("given", "string", label("Given names"), names).
		Display(schema.Title("surname"), schema.Subtitle("given"), schema.Collection(ontology.CollectionPeople))

	m.Relation("Mentions").Label("Mentions").
		Role("document", "Document", one, label("Document"), shownAs("Mentions")).
		Role("organisation", "Organisation", one, label("Organisation"), shownAs("Mentioned in")).
		Attribute("page", "int64", schema.Required(), label("Page")).
		Attribute("start", "int64", schema.Required(), label("Start"), hidden).
		Attribute("end", "int64", schema.Required(), label("End"), hidden).
		Attribute("printedText", "string", schema.Required(), label("Printed text")).
		Attribute("settledBy", "string", label("Settled by"), hidden)
	m.Relation("Shows").Label("Shows").
		Role("document", "Document", one, label("Document"), shownAs("Shows")).
		Role("identifier", "Identifier", one, label("Number"), shownAs("Printed in")).
		Attribute("page", "int64", schema.Required()).
		Attribute("start", "int64", schema.Required()).
		Attribute("end", "int64", schema.Required()).
		// A number's printed text. Every store filled before bank codes had
		// their own type holds a number's and a bank's printed text under
		// text, and keeps it until the next load replaces that document, so
		// text stays masked.
		Attribute("text", "string", schema.Required(), label("Printed text"), masked).
		Attribute("valid", "bool", label("Check passes")).
		Attribute("tier", "string", schema.Required(), tier, label("Tier"))
	m.Relation("ShowsCode").Label("Shows bank code").
		Role("document", "Document", one, label("Document"), shownAs("Shows bank code")).
		Role("code", "BankCode", one, label("Bank code"), shownAs("Printed in")).
		Attribute("page", "int64", schema.Required()).
		Attribute("start", "int64", schema.Required()).
		Attribute("end", "int64", schema.Required()).
		Attribute("printedText", "string", schema.Required()).
		Attribute("valid", "bool").
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("HeldAt").Label("Held at").
		Role("identifier", "Identifier", one, label("Number"), shownAs("Held at")).
		Role("organisation", "Organisation", one, label("Organisation"), shownAs("Holds")).
		Role("document", "Document", one, label("Document"), shownAs("Says who holds")).
		Attribute("reasons", "string", schema.Repeated(), label("Reasons")).
		Attribute("via", "string", schema.Repeated(), label("Via")).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("CodeOf").Label("Code of").
		Role("code", "BankCode", one, label("Bank code"), shownAs("Code of")).
		Role("organisation", "Organisation", one, label("Organisation"), shownAs("Bank codes")).
		Role("document", "Document", one, label("Document"), shownAs("Says whose code")).
		Attribute("reasons", "string", schema.Repeated()).
		Attribute("via", "string", schema.Repeated()).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("SameOrg").Label("Same organisation").
		Role("one", "Organisation", one, label("One"), shownAs("Same organisation as")).
		Role("other", "Organisation", one, label("Other"), shownAs("Same organisation as")).
		Role("document", "Document", one, label("Document"), shownAs("Says same organisation")).
		Attribute("reasons", "string", schema.Repeated()).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("StatementOf").Label("Statement in").
		Role("statement", "Statement", one, label("Statement"), shownAs("In document")).
		Role("document", "Document", one, label("Document"), shownAs("Statement"))
	m.Relation("StatementAccount").Label("Statement account").
		Role("statement", "Statement", one, label("Statement"), shownAs("Account")).
		Role("account", "Identifier", one, label("Account"), shownAs("Statements")).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("IssuedBy").Label("Issued by").
		Role("statement", "Statement", one, label("Statement"), shownAs("Issued by")).
		Role("issuer", "Organisation", one, label("Issuer"), shownAs("Statements issued")).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("PassportOf").Label("Passport in").
		Role("passport", "Passport", one, label("Passport"), shownAs("In document")).
		Role("document", "Document", one, label("Document"), shownAs("Passport"))
	m.Relation("HolderOf").Label("Holder of").
		Role("holder", "Holder", one, label("Holder"), shownAs("Passport")).
		Role("passport", "Passport", one, label("Passport"), shownAs("Holder")).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("PassportNumber").Label("Passport number").
		Role("passport", "Passport", one, label("Passport"), shownAs("Number")).
		Role("identifier", "Identifier", one, label("Number"), shownAs("Number of passport")).
		Attribute("tier", "string", schema.Required(), tier)
	m.Relation("PassportMRZ").Label("Machine-readable zone").
		Role("passport", "Passport", one, label("Passport"), shownAs("Machine-readable zone")).
		Role("identifier", "Identifier", one, label("Zone"), shownAs("Zone of passport"))
	return m
}

// withoutFile is a document's attributes as sifs-load wrote them before a
// document had a file name: none.
func withoutFile(attrs map[string]any) map[string]any {
	attrs = maps.Clone(attrs)
	delete(attrs, pack.FileName)
	delete(attrs, pack.Folder)
	if len(attrs) == 0 {
		return nil
	}
	return attrs
}

// beforeFiles is a document's facts as sifs-load wrote them at 2afc7bb.
func beforeFiles(run *extract.Output) func(extract.DocOut) interpreter.RecordFacts {
	return func(doc extract.DocOut) interpreter.RecordFacts {
		doc.File = ""
		return Facts(doc, run.Organisations)
	}
}

// The file name and the folder are attribute names no earlier files pack
// declared, so a store filled before them holds no value under either.
// Mutation: write the file name under files.name, and the name is one the
// pack at 2afc7bb declared.
func TestFileNamesAreNewNames(t *testing.T) {
	now, err := pack.Compile()
	if err != nil {
		t.Fatal(err)
	}
	for _, attr := range []string{pack.FileName, pack.Folder} {
		if now.GetAttributeType(attr) == nil {
			t.Errorf("the files pack does not declare %s", attr)
		}
	}
	for at, module := range map[string]*schema.ModuleDef{"6c6f1d6": baseModule(), "efe2d0b": mainModule(), "2afc7bb": moduleAt2afc7bb()} {
		box, err := schema.CompileModules(module)
		if err != nil {
			t.Fatal(err)
		}
		for _, attr := range []string{pack.FileName, pack.Folder} {
			if box.GetAttributeType(attr) != nil {
				t.Errorf("the files pack at %s already declared %s", at, attr)
			}
		}
	}
}

// A person's store filled at 2afc7bb. The next load gives every document of
// the committed run its file name, turns none down and leaves the store
// equal to one this loader filled; a second load changes nothing. Mutation:
// leave the file out of Facts, and the store filled at 2afc7bb already
// equals a fresh one.
func TestStoreFilledBeforeFileNames(t *testing.T) {
	run := committedRun(t)
	existing(t, moduleAt2afc7bb(), run, beforeFiles(run), func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		fresh := freshly(t, ctx, run)
		e := reopen()
		if before := held(t, ctx, e); len(before) == 0 || maps.Equal(before, fresh) {
			t.Fatal("filled at 2afc7bb, the store is empty or already equals a fresh one")
		}
		eq(t, "first load", mustLoad(t, ctx, e, run), Report{Written: len(run.Documents), TurnedDown: []TurnDown{}, LeftOut: committedLeftOut})
		eq(t, "the store after the load", held(t, ctx, reopen()), fresh)
		eq(t, "second load", mustLoad(t, ctx, reopen(), run), Report{Unchanged: len(run.Documents), TurnedDown: []TurnDown{}, LeftOut: committedLeftOut})
	})
}

// foundDocuments is the documents the reader finds for words, by id, with
// the file name each was found by.
func foundDocuments(t *testing.T, ctx context.Context, r *show.Reader, words string) map[string]string {
	t.Helper()
	found, err := r.Find(ctx, words)
	if err != nil {
		t.Fatal(err)
	}
	out := map[string]string{}
	for _, g := range found.Groups {
		if g.Type != pack.Document {
			continue
		}
		for _, m := range g.Things {
			out[m.ID] = m.Matched.Value
		}
	}
	return out
}

// stem is a file name without its extension.
func stem(name string) string {
	if i := strings.LastIndexByte(name, '.'); i > 0 {
		return name[:i]
	}
	return name
}

// firstWord is a file name's first word: up to its first space, hyphen,
// underscore or dot.
func firstWord(name string) string {
	if i := strings.IndexAny(name, " -_."); i > 0 {
		return name[:i]
	}
	return name
}

// On the committed run, a document reads as its file: its page is titled
// by the file name, and typing the whole file name, its first word or the
// name without its extension finds it, matched by its file name. The
// documents are files from the Netherlands, Germany, Britain, Japan, France,
// Canada and India. Mutation: drop the document's title in the pack, and
// each page is titled by its id.
func TestDocumentReadsAsItsFile(t *testing.T) {
	run := committedRun(t)
	files := map[string]string{}
	for _, doc := range run.Documents {
		files[doc.ID] = doc.File
	}
	ids := []string{"abn-amro-statement", "commerzbank-kontoauszug", "natwest-statement", "mizuho-statement-ja", "rib-fr", "credit-advice-smbk-ca", "hdfc-statement-sep"}
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		eq(t, "turned down", mustLoad(t, ctx, reopen(), run).TurnedDown, []TurnDown{})
		r := show.New(reopen().interp, false)
		for _, id := range ids {
			file := files[id]
			if file == "" || strings.Contains(file, "/") {
				t.Fatalf("the run gives %s the file %q, want a name in the folder read", id, file)
			}
			p := page(t, ctx, r, DocumentPrefix+id)
			eq(t, id+" title", p.Title, file)
			eq(t, id+" subtitle", p.Subtitle, "")
			eq(t, id+" file name", value(p, "File name"), []string{file})
			for _, words := range []string{file, firstWord(file), stem(file), strings.ToUpper(stem(file))} {
				if got := foundDocuments(t, ctx, r, words)[DocumentPrefix+id]; got != file {
					t.Errorf("%q finds %s by %q, want by its file name %q", words, id, got, file)
				}
			}
		}
	})
}

// A run of made-up files from several countries: files in folders two deep,
// names with spaces and with letters outside ASCII, two files of one name in
// two folders, a very long name, and a document whose run gives no file.
// Each is titled by its file name with its folder as subtitle, and found by
// the whole name, its first word and its name without its extension; the
// two files of one name are two documents told apart by their folders; the
// one with no file is titled by its id and has no file name. Mutation: drop
// the folder in Facts, and the subtitles are empty.
func TestFileNamesOfMadeUpRun(t *testing.T) {
	long := "Relevé de compte courant de la coopérative d'habitation des Pâquis pour le troisième trimestre deux mille vingt-six avec annexes et justificatifs de paiement des charges communes et du fonds de rénovation.pdf"
	cases := []struct{ id, file, title, subtitle string }{
		{"giro-munich", "Banken/2026/Kontoauszug März.pdf", "Kontoauszug März.pdf", "Banken/2026"},
		{"invoice-osaka", "請求書 2026-08.pdf", "請求書 2026-08.pdf", ""},
		{"receipt-lisbon", "Recibos/Lisboa/recibo.pdf", "recibo.pdf", "Recibos/Lisboa"},
		{"receipt-sao-paulo", "Recibos/São Paulo/recibo.pdf", "recibo.pdf", "Recibos/São Paulo"},
		{"lease-geneva", "Logement/Genève/" + long, long, "Logement/Genève"},
		{"note-nairobi", "", DocumentPrefix + "note-nairobi", ""},
	}
	run := &extract.Output{Format: 3, Organisations: map[string]extract.Card{}}
	for _, c := range cases {
		run.Documents = append(run.Documents, extract.DocOut{ID: c.id, File: c.file})
	}
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		eq(t, "turned down", mustLoad(t, ctx, reopen(), run).TurnedDown, []TurnDown{})
		r := show.New(reopen().interp, false)
		for _, c := range cases {
			p := page(t, ctx, r, DocumentPrefix+c.id)
			eq(t, c.id+" title", p.Title, c.title)
			eq(t, c.id+" subtitle", p.Subtitle, c.subtitle)
			if c.file == "" {
				eq(t, c.id+" values", p.Values, []show.Value{})
				continue
			}
			for _, words := range []string{c.title, firstWord(c.title), stem(c.title)} {
				if got := foundDocuments(t, ctx, r, words)[DocumentPrefix+c.id]; got != c.title {
					t.Errorf("%q finds %s by %q, want by %q", words, c.id, got, c.title)
				}
			}
		}
		same := foundDocuments(t, ctx, r, "recibo.pdf")
		eq(t, "recibo.pdf finds", slices.Sorted(maps.Keys(same)), []string{DocumentPrefix + "receipt-lisbon", DocumentPrefix + "receipt-sao-paulo"})
		eq(t, "the old id", foundDocuments(t, ctx, r, "note-nairobi"), map[string]string{})
	})
}
