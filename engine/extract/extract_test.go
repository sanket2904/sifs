package extract

import (
	"sort"
	"strings"
	"testing"
	"unicode/utf8"
)

var ref = Date{2026, 11, 22}

// registry is the IBAN registry table the scorer reads.
var registry = func() *IBANRegistry {
	r, err := LoadIBANRegistry("../../sample/iban-registry.json")
	if err != nil {
		panic(err)
	}
	return r
}()

func doc(id string, lines ...string) Document {
	return Document{ID: id, Pages: []Page{{Text: strings.Join(lines, "\n")}}}
}

func runOne(t *testing.T, d Document, dir *Directory) (Output, DocOut) {
	t.Helper()
	o := Run([]Document{d}, dir, registry, ref)
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

// joinsOf lists same_org rows as "key = key: reason tier".
func joinsOf(e DocOut) []string {
	var out []string
	for _, j := range e.SameOrg {
		out = append(out, j.Keys[0]+" = "+j.Keys[1]+": "+j.Reason+" "+string(j.Tier))
	}
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
		{"Banks involved: Alpha Group, Riverside Bank", []string{"Alpha Group", "Riverside Bank"}},
		{"Remitting Bank: Riverside Trust", []string{"Riverside Trust"}},
		{"Payment is accepted at the branches of these banks:", nil},
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
		{"ご連絡は見本ABC信託銀行ではなく、見本ABC銀行まで", []string{"見本ABC信託銀行", "見本ABC銀行"}},
		{"शाखा प्रबंधक, नमूना बैंक", []string{"नमूना बैंक"}},
		{"नमूना बैंक ऑफ़ भारत", []string{"नमूना बैंक ऑफ़ भारत"}},
		{"नियोक्ता: नमूना टेक प्राइवेट लिमिटेड", []string{"नमूना टेक प्राइवेट लिमिटेड"}},
		{"यह एक बहुत लंबा वाक्य है जिसमें नमूना बैंक आता है", nil}, // no phrase start within four words
	} {
		_, e := runOne(t, doc("d", c.line), nil)
		eq(t, c.line, names(e), c.want)
	}
}

func TestBareFieldWord(t *testing.T) {
	alias := entry("Q9", "Example Bank", "examplebank", "")
	alias.Names = append(alias.Names, struct {
		Text string `json:"text"`
		Key  string `json:"key"`
	}{"EXB", "exb"})
	dir := NewDirectory([]DirEntry{alias})
	for _, d := range []*Directory{nil, dir} {
		for _, lines := range [][]string{
			{"Bank: EXB", "Ask which one this is."},
			{"Bank: EXB", "EXB stands for our blue pattern here."},
			{"Kreditinstitut: Beispielkasse"},
		} {
			o, e := runOne(t, doc("d", lines...), d)
			for _, m := range e.Orgs {
				if m.Org != nil {
					t.Errorf("%q: a bare word in a bank field is no organisation by the label alone: %+v %+v", lines, m, o.Organisations[*m.Org])
				}
			}
		}
	}
	// a tie in the document still settles it
	_, e := runOne(t, doc("d", "Bank: EXMB", "Account: 4720195386 IFSC: EXMB0123456"), nil)
	if len(e.Orgs) != 1 || e.Orgs[0].Org == nil || *e.Orgs[0].SettledBy != "code" {
		t.Errorf("a bare word that is the code beside it is settled by the code: %+v", e.Orgs)
	}
	_, e = runOne(t, doc("d", "Example Bureau Bank", "Bank: Example"), nil)
	if len(e.Orgs) != 2 || e.Orgs[1].Org == nil || *e.Orgs[1].Org != *e.Orgs[0].Org {
		t.Errorf("a bare word that leads a full name in the document is that name: %+v", e.Orgs)
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
		"111122223333 -> IFSC ACME: paired-code",
		"444455556666 -> IFSC ZETA: paired-code",
		"ACME0BR0001 -> IFSC ACME: ifsc-prefix",
		"ZETA0BR0002 -> IFSC ZETA: ifsc-prefix",
	})
	eq(t, "same_org", joinsOf(e), []string{
		"IFSC ACME = name:d:acmebank: spells-code likely",
		"IFSC ZETA = name:d:zetabank: spells-code likely",
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
		"Line Code Name Number",
		"1 ALFA0BR0001 First Party, savings 111122223333",
		"2 BETA0BR0002 Second Party, current 444455556666"), nil)
	eq(t, "links", linksOf(o, e), []string{
		"111122223333 -> IFSC ALFA: paired-code",
		"444455556666 -> IFSC BETA: paired-code",
		"ALFA0BR0001 -> IFSC ALFA: ifsc-prefix",
		"BETA0BR0002 -> IFSC BETA: ifsc-prefix",
	})

	// two accounts and one code on a line: the line does not say whose code
	o, e = runOne(t, doc("d", "From account: 111122223333 To account: 444455556666 IFSC: ALFA0BR0001"), nil)
	eq(t, "links", linksOf(o, e), []string{"ALFA0BR0001 -> IFSC ALFA: ifsc-prefix"})
}

func TestIdentifiers(t *testing.T) {
	_, e := runOne(t, doc("d",
		"IBAN: GB29 NWBK 6016 1331 9268 19",
		"BIC: EXMPGB2L",
		"Account number: 31926819 Sort code: 60-16-13",
		"Pay to IBAN: DE89 3704 0044 0532 0130 01",
		"Card on file: 4111 1111 1111 1113",
		"Invoice number: 77004720195386",
		"Invoice number: 4000 0566 5566 5556",
		"Order no. 3782 822463 10005",
		"Paid by card: 4111 1111 1111 1111",
		"Phone: +1 555 7351 864209",
		"PAN: ABCPK1234F",
		"TAN: ABCDK1234F",
		"Code banque: 20041, Code guichet: 01005, Numéro de compte: 0500013M026, Clé RIB: 06"), nil)
	k := kinds(e)
	for v, want := range map[string]string{
		"GB29NWBK60161331926819": KindIBAN, "EXMPGB2L": KindBIC, "601613": KindSortCode, "31926819": KindAccount,
		"DE89370400440532013001": KindIBAN, "4111111111111113": KindCard, "4111111111111111": KindCard, "ABCPK1234F": KindPAN,
		"20041": KindFRBank, "01005": KindFRBranch, "0500013M026": KindAccount, "06": KindFRKey,
	} {
		if k[v] != want {
			t.Errorf("%s: kind %q, want %q", v, k[v], want)
		}
	}
	for _, v := range []string{"77004720195386", "864209", "ABCDK1234F", "4000056655665556", "378282246310005"} {
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
	if valid["DE89370400440532013001"] || valid["4111111111111113"] || !valid["GB29NWBK60161331926819"] || !valid["06"] {
		t.Errorf("validity: %v", valid)
	}
}

func TestInvalidIdentifiersAreNeverLinked(t *testing.T) {
	o, e := runOne(t, doc("d", "Pay to IBAN: DE89 3704 0044 0532 0130 01", "BIC: EXMPDEFFXXX"), nil)
	for _, l := range linksOf(o, e) {
		if strings.HasPrefix(l, "DE89370400440532013001") {
			t.Errorf("an IBAN that fails its check is linked: %s", l)
		}
	}
}

func TestIBANParts(t *testing.T) {
	o, e := runOne(t, doc("d",
		"RIB du compte",
		"IBAN : FR14 2004 1010 0505 0001 3M02 606",
		"BIC : EXMPFRPPXXX",
		"Code banque: 20041, Code guichet: 01005, Numéro de compte: 0500013M026, Clé RIB: 06"), nil)
	eq(t, "links", linksOf(o, e), []string{
		"01005 -> IBAN FR 20041: inside-iban",
		"0500013M026 -> IBAN FR 20041: same-account",
		"20041 -> IBAN FR 20041: inside-iban",
		"EXMPFRPPXXX -> BIC EXMPFRPP: bic-institution",
		"FR1420041010050500013M02606 -> IBAN FR 20041: iban-bank-code",
	})
	eq(t, "same_org", joinsOf(e), []string{"BIC EXMPFRPP = IBAN FR 20041: printed-together likely"})
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
		"Account statement",
		"Account number: 111122223333",
		"Statement period: 01 Aug 2026 - 31 Aug 2026",
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
		{"Account statement. This is not a genuine statement.", true},
		{"Ein echter Kontoauszug ist das nicht", true},
		{"本物の明細ではありません。取引明細", true},
		{"明細ではありません", false},
		{"असली पासबुक नहीं, खाता पासबुक", true},
		{"This statement is not a tax invoice", true},
		{"Sheet No. 7 statement", true},
		{"No 12 statement", true},
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
	o := Run([]Document{old, bad, visa}, nil, registry, ref)
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

func TestScannedLabelsAllowOneMisread(t *testing.T) {
	text := "PASSPORT\nPASSPORT NO: AB1234567\nSURNAHE: DOE\nGIVEN NAMESI JANE\nDATE OF ISSUE: 01 JAN 2020\nDATE 0F EXPIRY: 01 JAN 2030"
	for _, scanned := range []bool{true, false} {
		_, e := runOne(t, Document{ID: "p", Pages: []Page{{Text: text, Scanned: scanned}}}, nil)
		got := e.Passport != nil && e.Passport.Holder == HolderOut{"DOE", "JANE"} && e.Passport.Expiry != nil && *e.Passport.Expiry == "2030-01-01"
		if got != scanned {
			t.Errorf("scanned %v: passport %+v", scanned, e.Passport)
		}
	}
	_, e := runOne(t, Document{ID: "p", Pages: []Page{{Text: "PASSPORT\nPASSPORT NO: AB1234567\nSUBNAMES: DOE\nGIVEN NAMES: JANE", Scanned: true}}}, nil)
	if e.Passport != nil {
		t.Errorf("two misread characters are no label: %+v", e.Passport)
	}
}

func TestRenewalSupersedes(t *testing.T) {
	pp := func(id, issued, expiry string, extra ...string) Document {
		lines := []string{"PASSPORT", "Passport No.: " + strings.ToUpper(id) + "0001", "Surname: DOE", "Given names: JANE"}
		if issued != "" {
			lines = append(lines, "Date of issue: "+issued)
		}
		return doc(id, append(append(lines, "Date of expiry: "+expiry), extra...)...)
	}
	current := func(o Output) map[string]string {
		out := map[string]string{}
		for _, d := range o.Documents {
			if p := d.Passport; p != nil {
				out[d.ID] = "null"
				if p.Current != nil {
					out[d.ID] = map[bool]string{true: "true", false: "false"}[*p.Current]
				}
			}
		}
		return out
	}
	for _, c := range []struct {
		name   string
		docs   []Document
		want   map[string]string
		answer string
	}{
		{"the later issue is current", []Document{pp("aa", "01 Jan 2020", "01 Jan 2030"), pp("bb", "01 Sep 2026", "01 Sep 2036"), pp("cc", "01 Jan 2020", "01 Jan 2030")},
			map[string]string{"aa": "false", "bb": "true", "cc": "false"}, "bb"},
		{"issue dates cannot order them", []Document{pp("aa", "01 Jan 2020", "01 Jan 2030"), pp("bb", "", "01 Sep 2036")},
			map[string]string{"aa": "null", "bb": "null"}, ""},
		{"same issue date", []Document{pp("aa", "01 Jan 2020", "01 Jan 2030"), pp("bb", "01 Jan 2020", "01 Sep 2036")},
			map[string]string{"aa": "null", "bb": "null"}, ""},
		{"a stamped cancellation", []Document{pp("aa", "01 Jan 2020", "01 Jan 2030", "*** VOID ***"), pp("bb", "01 Jan 2021", "01 Jan 2031", "STATUS: ANNULÉ")},
			map[string]string{"aa": "false", "bb": "false"}, ""},
		{"the word in prose is no stamp", []Document{pp("aa", "01 Jan 2020", "01 Jan 2030", "Altered pages make it void.", "VOID IF ALTERED")},
			map[string]string{"aa": "true"}, "aa"},
	} {
		o := Run(c.docs, nil, registry, ref)
		got := current(o)
		for id, w := range c.want {
			if got[id] != w {
				t.Errorf("%s: %s current %s, want %s", c.name, id, got[id], w)
			}
		}
		ans := ""
		if len(o.Answers.CurrentPassport) == 1 {
			ans = o.Answers.CurrentPassport[0].Document
		}
		if ans != c.answer || len(o.Answers.CurrentPassport) > 1 {
			t.Errorf("%s: answer %+v, want %q", c.name, o.Answers.CurrentPassport, c.answer)
		}
	}
}

func TestLatestStatementByPeriodNotFile(t *testing.T) {
	mk := func(id, period string) Document {
		return doc(id, "Riverside Bank", "Account statement", "Account number: 111122223333", "Period: "+period)
	}
	o := Run([]Document{mk("sep", "01 Sep 2026 to 30 Sep 2026"), mk("oct", "01 Oct 2026 to 31 Oct 2026"), mk("aug", "01 Aug 2026 to 31 Aug 2026")}, nil, registry, ref)
	if len(o.Answers.LatestStatement) != 1 || o.Answers.LatestStatement[0].Document != "oct" {
		t.Errorf("latest: %+v", o.Answers.LatestStatement)
	}
}

func TestGroupedAccount(t *testing.T) {
	for _, c := range []struct{ line, value, text string }{
		{"Account: 4720 1953 86", "4720195386", "4720 1953 86"},
		{"Member number / account: 47201 95386", "4720195386", "47201 95386"},
		{"Account no.: 123 456 789  Branch: North", "123456789", "123 456 789"},
		{"Account: 4720 1953 86 (savings)", "4720195386", "4720 1953 86"},
		{"Account: 123456 01 Jan 2026", "123456", "123456"},
		{"Invoice account: 4720 1953 86", "", ""},
	} {
		_, e := runOne(t, doc("d", c.line), nil)
		got := ""
		for _, i := range e.Identifiers {
			if i.Kind == KindAccount {
				got = i.Value + "|" + i.Text
			}
		}
		if want := c.value + "|" + c.text; c.value != "" && got != want || c.value == "" && got != "" {
			t.Errorf("%q: account %q, want %q", c.line, got, want)
		}
	}

	// one account in two spellings is one account for the latest answer
	mk := func(id, acct, period string) Document {
		return doc(id, "Riverside Bank", "Account statement", "Account: "+acct, "Period: "+period)
	}
	o := Run([]Document{mk("sep", "4720195386", "01 Sep 2026 to 30 Sep 2026"), mk("oct", "4720 1953 86", "01 Oct 2026 to 31 Oct 2026")}, nil, registry, ref)
	if len(o.Answers.LatestStatement) != 1 || o.Answers.LatestStatement[0].Document != "oct" {
		t.Errorf("latest: %+v", o.Answers.LatestStatement)
	}
}

func TestLatestWithheldWhenALaterStatementIsUnread(t *testing.T) {
	mk := func(id, acct, period string) Document {
		return doc(id, "Riverside Bank", "Account statement", "Account: "+acct, "Period: "+period)
	}
	sep := mk("sep", "4720195386", "01 Sep 2026 to 30 Sep 2026")
	oct := mk("oct", "12-34-56-78-90", "01 Oct 2026 to 31 Oct 2026")
	if o := Run([]Document{sep, oct}, nil, registry, ref); len(o.Answers.LatestStatement) != 0 {
		t.Errorf("October's account was not read, yet September is the answer: %+v", o.Answers.LatestStatement)
	}
	aug := mk("aug", "12-34-56-78-90", "01 Aug 2026 to 31 Aug 2026")
	if o := Run([]Document{sep, aug}, nil, registry, ref); len(o.Answers.LatestStatement) != 1 {
		t.Errorf("an unread earlier statement does not change the latest: %+v", o.Answers.LatestStatement)
	}
	// a printed name is its document's own: with the directory off, another
	// name may still be the same bank, and only the directory tells two apart
	other := doc("other", "Zeta Bank", "Account statement", "Account: 12-34-56-78-90", "Period: 01 Oct 2026 to 31 Oct 2026")
	if o := Run([]Document{sep, other}, nil, registry, ref); len(o.Answers.LatestStatement) != 0 {
		t.Errorf("with the directory off, an unread later statement under another name may be this account's: %+v", o.Answers.LatestStatement)
	}
	two := NewDirectory([]DirEntry{entry("Q1", "Riverside Bank", "riversidebank", ""), entry("Q2", "Zeta Bank", "zetabank", "")})
	if o := Run([]Document{sep, other}, two, registry, ref); len(o.Answers.LatestStatement) != 1 {
		t.Errorf("the directory says the unread statement's issuer is another bank: %+v", o.Answers.LatestStatement)
	}
	if o := Run([]Document{sep, oct}, two, registry, ref); len(o.Answers.LatestStatement) != 0 {
		t.Errorf("the directory says the unread statement's issuer is this bank: %+v", o.Answers.LatestStatement)
	}
	one := NewDirectory([]DirEntry{entry("Q1", "Riverside Bank", "riversidebank", "")})
	if o := Run([]Document{sep, other}, one, registry, ref); len(o.Answers.LatestStatement) != 0 {
		t.Errorf("the directory knows one issuer only, so it cannot tell them apart: %+v", o.Answers.LatestStatement)
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
	eq(t, "same_org", joinsOf(e), []string{"IFSC RIVR = name:d:riversidebank: directory certain"})
	if c := o.Organisations["IFSC RIVR"]; c.Directory == nil || *c.Directory != "Q1" {
		t.Errorf("the code's card is its entry: %+v", c)
	}

	_, off := runOne(t, doc("d", "Riverside Bank"), nil)
	if *off.Orgs[0].SettledBy == "directory" {
		t.Errorf("with the directory off nothing is settled by it")
	}
}

func TestCodeAloneMakesNoCardWithTheDirectory(t *testing.T) {
	d := doc("d", "Riverside Bank", "Account details", "Konto: 4720195386", "SWIFT: ZZZZCH2X")
	known := entry("Q7", "Zeta Bank", "zetabank", "")
	known.Codes.BIC = []string{"ZZZZCH2X"}
	for _, c := range []struct {
		dir   *Directory
		links int
	}{
		{NewDirectory([]DirEntry{entry("Q1", "Riverside Bank", "riversidebank", "")}), 0},
		{NewDirectory([]DirEntry{known}), 2},
		{nil, 2}, // with the directory off a code keeps its card
	} {
		o, e := runOne(t, d, c.dir)
		if len(e.Links) != c.links {
			t.Errorf("links %q, want %d", linksOf(o, e), c.links)
		}
		for k, card := range o.Organisations {
			if c.dir != nil && card.Name == nil {
				t.Errorf("a card with no name and no directory entry: %s", k)
			}
		}
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
	d := doc("d", "नमूना दस्तावेज़। परीक्षण", "खाता संख्या: 47201953861", "見本銀行", "口座番号: 7654321")
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
