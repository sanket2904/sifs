package extract

import (
	"sort"
	"strings"
	"testing"
	"unicode/utf8"
)

var ref = Date{2026, 11, 22}

func doc(id string, lines ...string) Document {
	return Document{ID: id, Pages: []Page{{Text: strings.Join(lines, "\n")}}}
}

func runOne(t *testing.T, d Document, dir *Directory) (Output, DocOut) {
	t.Helper()
	o := Run([]Document{d}, dir, ref)
	for _, e := range o.Documents {
		if e.ID == d.ID {
			checkSpans(t, d, e)
			return o, e
		}
	}
	return o, DocOut{}
}

// checkSpans proves every span reads back its text, counted in scalar values.
func checkSpans(t *testing.T, d Document, e DocOut) {
	t.Helper()
	var spans []Span
	for _, m := range e.Orgs {
		spans = append(spans, m.Span)
	}
	for _, i := range e.Identifiers {
		spans = append(spans, i.Span)
	}
	for _, s := range spans {
		page := []rune(d.Pages[s.Page-1].Text)
		if s.Start < 0 || s.End > len(page) || string(page[s.Start:s.End]) != s.Text {
			t.Errorf("span %+v does not read back", s)
		}
	}
}

func names(e DocOut) []string {
	var out []string
	for _, m := range e.Orgs {
		out = append(out, m.Text)
	}
	return out
}

// linksOf lists links as "identifier value -> card name or key: reasons".
func linksOf(o Output, e DocOut) []string {
	val := map[string]string{}
	for _, i := range e.Identifiers {
		val[i.ID] = i.Value
	}
	var out []string
	for _, l := range e.Links {
		who := l.Org
		if c := o.Organisations[l.Org]; c.Name != nil {
			who = *c.Name
		}
		out = append(out, val[l.Identifier]+" -> "+who+": "+strings.Join(l.Reasons, ","))
	}
	sort.Strings(out)
	return out
}

func kinds(e DocOut) map[string]string {
	out := map[string]string{}
	for _, i := range e.Identifiers {
		out[i.Value] = i.Kind
	}
	return out
}

func eq(t *testing.T, what string, got, want []string) {
	t.Helper()
	if strings.Join(got, "|") != strings.Join(want, "|") {
		t.Errorf("%s:\n got  %q\n want %q", what, got, want)
	}
}

func TestNamesAreWholeNames(t *testing.T) {
	for _, c := range []struct {
		line string
		want []string
	}{
		{"Example Union Bank of Testland Ltd", []string{"Example Union Bank of Testland Ltd"}},
		{"Branch Manager, Riverside Bank", []string{"Riverside Bank"}},
		{"Thank you for banking with Riverside Co-operative Bank.", []string{"Riverside Co-operative Bank"}},
		{"Banque : La Banque Exemplaire", []string{"La Banque Exemplaire"}},
		{"The ACME BANK app showed it.", []string{"ACME BANK"}},
		{"Payments: Musterbank AG, Filiale Nord", []string{"Musterbank AG"}},
		{"Bank: Riverside Bank", []string{"Riverside Bank"}},
		{"Kreditinstitut: Beispielkasse", []string{"Beispielkasse"}},
		{"Banks involved: Alpha Paribas, Riverside Bank", []string{"Alpha Paribas", "Riverside Bank"}},
		{"Remitting Bank: Riverside Trust", []string{"Riverside Trust"}},
		{"Fees can be paid at any branch of these banks:", nil},
		{"School office <office@example.org>", nil},
		{"Sample Town Public School: term dates", []string{"Sample Town Public School"}},
		{"East Bank of Testland", []string{"East Bank of Testland"}},
		{":: Riverside Bank ::", []string{"Riverside Bank"}},
		{"== 見本銀行 ::", []string{"見本銀行"}},
		{"Gegründet von der Deutschen Musterbank in Köln", []string{"Deutschen Musterbank"}},
	} {
		_, e := runOne(t, doc("d", c.line), nil)
		eq(t, c.line, names(e), c.want)
	}
}

func TestNamesInOtherScripts(t *testing.T) {
	for _, c := range []struct {
		line string
		want []string
	}{
		{"受取銀行: さくら見本銀行", []string{"さくら見本銀行"}},
		{"お問い合わせは見本UFJ信託銀行ではなく、見本UFJ銀行へ", []string{"見本UFJ信託銀行", "見本UFJ銀行"}},
		{"शाखा प्रबंधक, नमूना बैंक", []string{"नमूना बैंक"}},
		{"नमूना बैंक ऑफ़ भारत", []string{"नमूना बैंक ऑफ़ भारत"}},
		{"नियोक्ता: नमूना टेक प्राइवेट लिमिटेड", []string{"नमूना टेक प्राइवेट लिमिटेड"}},
		{"यह एक बहुत लंबा वाक्य है जिसमें नमूना बैंक आता है", nil}, // no phrase start within four words
	} {
		_, e := runOne(t, doc("d", c.line), nil)
		eq(t, c.line, names(e), c.want)
	}
}

func TestShortFormTies(t *testing.T) {
	o, e := runOne(t, doc("d",
		"ACME Life Assurance Company Limited",
		"Thank you for choosing ACME Life.",
		"Pay at any ACME Bank branch."), nil)
	eq(t, "names", names(e), []string{"ACME Life Assurance Company Limited", "ACME Life", "ACME Bank"})
	if *e.Orgs[0].Org != *e.Orgs[1].Org || *e.Orgs[1].SettledBy != "mention" {
		t.Errorf("ACME Life is a short form of the assurance company: %+v", e.Orgs[:2])
	}
	if *e.Orgs[2].Org == *e.Orgs[0].Org {
		t.Errorf("ACME Bank is another organisation")
	}
	_ = o

	_, e = runOne(t, doc("d", "XYFF Ltd", "Xavier Young Finance Fund Limited"), nil)
	if e.Orgs[0].Org == nil || *e.Orgs[0].Org != *e.Orgs[1].Org || *e.Orgs[0].SettledBy != "mention" {
		t.Errorf("XYZF Ltd is the initials of the full name: %+v", e.Orgs)
	}

	// an acronym with a legal form and nothing in the document behind it
	_, e = runOne(t, doc("d", "XYZF Ltd", "Pay by 1 May."), nil)
	if len(e.Orgs) != 1 || e.Orgs[0].Org != nil {
		t.Errorf("an unsettled acronym name has no organisation: %+v", e.Orgs)
	}

	// SAMPLE in a heading is no short form of a name spelt Sample
	_, e = runOne(t, doc("d", "SAMPLE DOCUMENT.", "Sample Nagar Bank"), nil)
	eq(t, "names", names(e), []string{"Sample Nagar Bank"})

	// the same acronym fits two organisations: settled by neither
	_, e = runOne(t, doc("d", "ACME Bank", "ACME Life Assurance Company", "Asked ACME about it."), nil)
	eq(t, "names", names(e), []string{"ACME Bank", "ACME Life Assurance Company"})
}

func TestCodeTies(t *testing.T) {
	o, e := runOne(t, doc("d",
		"ACME NetBanking receipt",
		"Debit account: 111122223333 IFSC: ACME0BR0001 Bank: ACME Bank",
		"Credit account: 444455556666 IFSC: ZETA0BR0002 Bank: Zeta Bank"), nil)
	eq(t, "links", linksOf(o, e), []string{
		"111122223333 -> ACME Bank: paired-code",
		"444455556666 -> Zeta Bank: paired-code",
		"ACME0BR0001 -> ACME Bank: ifsc-prefix",
		"ZETA0BR0002 -> Zeta Bank: ifsc-prefix",
	})
	if e.Orgs[0].Text != "ACME" || *e.Orgs[0].SettledBy != "code" && *e.Orgs[0].SettledBy != "mention" {
		t.Errorf("ACME in the heading is settled by the code beside it: %+v", e.Orgs[0])
	}

	// the code's prefix fits a name with no bank word only: no tie
	o, e = runOne(t, doc("d", "ACME Life Assurance Company Limited", "Account: 111122223333 IFSC: ACME0BR0001"), nil)
	for _, l := range linksOf(o, e) {
		if strings.Contains(l, "Assurance") {
			t.Errorf("an IFSC never names an assurance company: %s", l)
		}
	}
}

func TestNoTieNoLink(t *testing.T) {
	o, e := runOne(t, doc("d",
		"Transfer advice",
		"From account: 111122223333",
		"To account: 444455556666",
		"Banks: Alpha Bank, Beta Bank"), nil)
	if l := linksOf(o, e); len(l) != 0 {
		t.Errorf("two banks and two accounts with nothing tying them: %q", l)
	}
}

func TestPairedCodeNeedsTheRow(t *testing.T) {
	o, e := runOne(t, doc("d",
		"Row IFSC Party Account",
		"1 ALFA0BR0001 First Party, savings 111122223333",
		"2 BETA0BR0002 Second Party, current 444455556666"), nil)
	eq(t, "links", linksOf(o, e), []string{
		"111122223333 -> org-1: paired-code",
		"444455556666 -> org-2: paired-code",
		"ALFA0BR0001 -> org-1: ifsc-prefix",
		"BETA0BR0002 -> org-2: ifsc-prefix",
	})

	// two accounts and one code on a line: the line does not say whose code
	o, e = runOne(t, doc("d", "From account: 111122223333 To account: 444455556666 IFSC: ALFA0BR0001"), nil)
	eq(t, "links", linksOf(o, e), []string{"ALFA0BR0001 -> org-1: ifsc-prefix"})
}

func TestIdentifiers(t *testing.T) {
	_, e := runOne(t, doc("d",
		"IBAN: GB29 NWBK 6016 1331 9268 19",
		"BIC: NWBKGB2L",
		"Sort code: 60-16-13 Account number: 31926819",
		"Pay to IBAN: DE88 3704 0044 0532 0130 00",
		"Card on file: 4111 1111 1111 1112",
		"Invoice number: 50100987654321",
		"Phone: +44 7700 900123",
		"PAN: ABCPK1234F",
		"TAN: ABCDK1234F",
		"Code banque : 20041 Code guichet : 01005 Numéro de compte : 0500013M026 Clé RIB : 06"), nil)
	k := kinds(e)
	for v, want := range map[string]string{
		"GB29NWBK60161331926819": KindIBAN, "NWBKGB2L": KindBIC, "601613": KindSortCode, "31926819": KindAccount,
		"DE88370400440532013000": KindIBAN, "4111111111111112": KindCard, "ABCPK1234F": KindPAN,
		"20041": KindFRBank, "01005": KindFRBranch, "0500013M026": KindAccount, "06": KindFRKey,
	} {
		if k[v] != want {
			t.Errorf("%s: kind %q, want %q", v, k[v], want)
		}
	}
	for _, v := range []string{"50100987654321", "900123", "ABCDK1234F"} {
		if _, ok := k[v]; ok {
			t.Errorf("%s is no identifier", v)
		}
	}
	valid := map[string]bool{}
	for _, i := range e.Identifiers {
		if i.Valid != nil {
			valid[i.Value] = *i.Valid
		}
	}
	if valid["DE88370400440532013000"] || valid["4111111111111112"] || !valid["GB29NWBK60161331926819"] || !valid["06"] {
		t.Errorf("validity: %v", valid)
	}
}

func TestInvalidIdentifiersAreNeverLinked(t *testing.T) {
	o, e := runOne(t, doc("d", "Pay to IBAN: DE88 3704 0044 0532 0130 00", "BIC: COBADEFFXXX"), nil)
	for _, l := range linksOf(o, e) {
		if strings.HasPrefix(l, "DE88") {
			t.Errorf("an IBAN that fails its check is linked: %s", l)
		}
	}
}

func TestIBANParts(t *testing.T) {
	o, e := runOne(t, doc("d",
		"Relevé d'identité bancaire",
		"IBAN : FR14 2004 1010 0505 0001 3M02 606",
		"BIC : PSSTFRPPXXX",
		"Code banque : 20041 Code guichet : 01005 Numéro de compte : 0500013M026 Clé RIB : 06"), nil)
	got := strings.Join(linksOf(o, e), "\n")
	for _, want := range []string{"20041 -> org-1: inside-iban", "01005 -> org-1: inside-iban", "0500013M026 -> org-1: same-account", "FR1420041010050500013M02606 -> org-1: iban-bank-code,paired-code", "PSSTFRPPXXX -> org-1: bic-institution"} {
		if !strings.Contains(got, want) {
			t.Errorf("missing %q in\n%s", want, got)
		}
	}
}

func TestOCRRepairOnlyOnScans(t *testing.T) {
	clean := Document{ID: "d", Pages: []Page{{Text: "IFSC: ABCDØBR0001"}}}
	if _, e := runOne(t, clean, nil); len(e.Identifiers) != 0 {
		t.Errorf("a text page is read as written: %+v", e.Identifiers)
	}
	scan := Document{ID: "d", Pages: []Page{{Text: "IFSC: ABCDØBR0001", Scanned: true}}}
	_, e := runOne(t, scan, nil)
	if len(e.Identifiers) != 1 || e.Identifiers[0].Value != "ABCD0BR0001" || e.Identifiers[0].Text != "ABCDØBR0001" {
		t.Errorf("the fifth character of a scanned IFSC is the digit zero: %+v", e.Identifiers)
	}
}

func TestStatement(t *testing.T) {
	d := doc("s1",
		"Riverside Bank",
		"Statement of account",
		"Account number: 111122223333",
		"Statement period: 01 Aug 2026 to 31 Aug 2026",
		"Your next statement will cover 01 Sep 2026 to 30 Sep 2026.")
	o, e := runOne(t, d, nil)
	if e.Statement == nil || e.Statement.From != "2026-08-01" || e.Statement.To != "2026-08-31" {
		t.Fatalf("statement: %+v", e.Statement)
	}
	eq(t, "links", linksOf(o, e), []string{"111122223333 -> Riverside Bank: statement-header"})

	// a bill has a period too, and is no statement
	_, e = runOne(t, doc("b", "Riverside Power Ltd", "Electricity bill", "Account: 111122223333", "Billing period: 01 Aug 2026 to 31 Aug 2026"), nil)
	if e.Statement != nil {
		t.Errorf("a bill is no statement: %+v", e.Statement)
	}

	// an account its code ties elsewhere is not tied by the header
	o, e = runOne(t, doc("h", "Riverside Bank", "Account statement", "Linked account: 111122223333 IFSC: ZETA0BR0002", "Period: 01 Aug 2026 to 31 Aug 2026"), nil)
	for _, l := range linksOf(o, e) {
		if strings.Contains(l, "statement-header") {
			t.Errorf("the header ties an account its code puts elsewhere: %s", l)
		}
	}
}

func TestDeniedRole(t *testing.T) {
	body := []string{"Riverside Bank", "", "Account number: 111122223333", "Period: 01 Aug 2026 to 31 Aug 2026"}
	for _, c := range []struct {
		title string
		want  bool
	}{
		{"Not a statement - account lookup sheet", false},
		{"Dies ist kein Kontoauszug", false},
		{"Ce n'est pas un relevé de compte", false},
		{"खाता विवरण नहीं है", false},
		{"Statement of account. Not a real statement.", true},
		{"Kein echter Kontoauszug", true},
		{"実在の明細ではありません。取引明細", true},
		{"明細ではありません", false},
		{"असली पासबुक नहीं, खाता पासबुक", true},
		{"This statement is not a tax invoice", true},
	} {
		lines := append([]string{}, body...)
		lines[1] = c.title
		o, e := runOne(t, doc("s", lines...), nil)
		if got := e.Statement != nil; got != c.want {
			t.Errorf("%q: statement %v, want %v", c.title, got, c.want)
		}
		if !c.want && (len(e.Links) != 0 || len(o.Answers.LatestStatement) != 0) {
			t.Errorf("%q: no statement, yet links %+v answers %+v", c.title, e.Links, o.Answers.LatestStatement)
		}
	}

	_, e := runOne(t, doc("p", "PASSPORT", "This is not a passport", "Passport No.: AB1234567", "Surname: DOE", "Given names: JANE", "Date of expiry: 01 Jan 2030"), nil)
	if e.Passport != nil {
		t.Errorf("a page that says it is no passport is none: %+v", e.Passport)
	}
	_, e = runOne(t, doc("p", "PASSPORT", "Not a real passport", "Passport No.: AB1234567", "Surname: DOE", "Given names: JANE", "Date of expiry: 01 Jan 2030"), nil)
	if e.Passport == nil {
		t.Errorf("a specimen passport is a passport")
	}
}

func TestPeriodOrder(t *testing.T) {
	for _, c := range []struct {
		s        string
		from, to string
	}{
		{"01-08-2026 to 31-08-2026", "2026-08-01", "2026-08-31"},
		{"08/01/2026 - 08/31/2026", "2026-08-01", "2026-08-31"},
		{"01.10.2026 bis 31.10.2026", "2026-10-01", "2026-10-31"},
		{"2026年8月1日～2026年8月31日", "2026-08-01", "2026-08-31"},
		{"01 जुलाई 2026 से 30 सितंबर 2026", "2026-07-01", "2026-09-30"},
		{"1 octobre 2026 au 31 octobre 2026", "2026-10-01", "2026-10-31"},
		{"03-04-2026 to 05-06-2026", "", ""}, // either order reads: no period
	} {
		f, to, ok := readPeriod(c.s)
		if c.from == "" {
			if ok {
				t.Errorf("%s: read %s..%s, want no period", c.s, f, to)
			}
			continue
		}
		if !ok || f.String() != c.from || to.String() != c.to {
			t.Errorf("%s: got %s..%s (%v), want %s..%s", c.s, f, to, ok, c.from, c.to)
		}
	}
}

func TestPassports(t *testing.T) {
	old := doc("p-old",
		"PASSPORT",
		"Passport No.: L898902C3",
		"Surname: ERIKSSON", "Given names: ANNA MARIA",
		"Date of issue: 15 APR 2002", "Date of expiry: 15 APR 2012",
		"P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<",
		"L898902C36UTO7408122F1204159ZE184226B<<<<<10")
	bad := doc("p-bad",
		"PASSPORT",
		"Passport No.: L898902C3",
		"Surname: ERIKSSON", "Given names: ANNA MARIA",
		"Date of expiry: 15 APR 2012",
		"P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<",
		"L898902C36UTO7408122F1204159ZE184226B<<<<<11")
	visa := doc("v",
		"VISA",
		"Passport number: L898902C3",
		"V<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<",
		"L8988901C4XXX4009078F96121096ZE184226B<<<<<<")
	o := Run([]Document{old, bad, visa}, nil, ref)
	byID := map[string]DocOut{}
	for _, d := range o.Documents {
		byID[d.ID] = d
	}
	if p := byID["p-old"].Passport; p == nil || *p.Expiry != "2012-04-15" || *p.Current {
		t.Errorf("expired passport: %+v", p)
	}
	if p := byID["p-bad"].Passport; p == nil || *p.Expiry != "cannot-tell" || p.Current == nil || *p.Current {
		t.Errorf("a zone that fails its check: expiry cannot be told, but both dates are past: %+v", p)
	}
	if byID["v"].Passport != nil {
		t.Errorf("a visa page is no passport")
	}
	if len(o.Answers.CurrentPassport) != 0 {
		t.Errorf("no current passport: %+v", o.Answers.CurrentPassport)
	}
}

func TestLatestStatementByPeriodNotFile(t *testing.T) {
	mk := func(id, period string) Document {
		return doc(id, "Riverside Bank", "Statement of account", "Account number: 111122223333", "Period: "+period)
	}
	o := Run([]Document{mk("sep", "01 Sep 2026 to 30 Sep 2026"), mk("oct", "01 Oct 2026 to 31 Oct 2026"), mk("aug", "01 Aug 2026 to 31 Aug 2026")}, nil, ref)
	if len(o.Answers.LatestStatement) != 1 || o.Answers.LatestStatement[0].Document != "oct" {
		t.Errorf("latest: %+v", o.Answers.LatestStatement)
	}
}

func TestDirectory(t *testing.T) {
	dir := NewDirectory([]DirEntry{
		entry("Q1", "Riverside Bank", "riversidebank", "RIVR"),
		entry("Q2", "Riverside Holdings", "rh", ""),
		entry("Q3", "Rhine Harbour Bank", "rh", ""),
	})
	o, e := runOne(t, doc("d", "Riverside Bank", "Account: 111122223333 IFSC: RIVR0BR0001", "RH Ltd"), dir)
	c := o.Organisations[*e.Orgs[0].Org]
	if c.Directory == nil || *c.Directory != "Q1" {
		t.Errorf("an exact name gets its directory id: %+v", c)
	}
	if e.Orgs[1].Org != nil {
		t.Errorf("a clash in the directory settles nothing: %+v", e.Orgs[1])
	}
	if len(e.Links) == 0 || e.Links[0].Org != *e.Orgs[0].Org {
		t.Errorf("the code and the name join one card: %+v", e.Links)
	}

	_, off := runOne(t, doc("d", "Riverside Bank"), nil)
	if *off.Orgs[0].SettledBy == "directory" {
		t.Errorf("with the directory off nothing is settled by it")
	}
}

func entry(id, name, key, ifsc string) DirEntry {
	e := DirEntry{ID: id, Name: name}
	e.Names = append(e.Names, struct {
		Text string `json:"text"`
		Key  string `json:"key"`
	}{name, key})
	if ifsc != "" {
		e.Codes.IFSCPrefix = []string{ifsc}
	}
	return e
}

func TestSpansCountScalarValues(t *testing.T) {
	d := doc("d", "नमूना दस्तावेज़। परीक्षण", "खाता संख्या: 30123456789", "見本銀行", "口座番号: 7654321")
	_, e := runOne(t, d, nil)
	for _, i := range e.Identifiers {
		page := d.Pages[0].Text
		b := strings.Index(page, i.Text)
		if want := utf8.RuneCountInString(page[:b]); i.Start != want {
			t.Errorf("%s starts at %d, want scalar offset %d", i.Text, i.Start, want)
		}
	}
	if len(e.Identifiers) != 2 || len(e.Orgs) != 1 {
		t.Errorf("found %+v and %+v", e.Identifiers, e.Orgs)
	}
}
