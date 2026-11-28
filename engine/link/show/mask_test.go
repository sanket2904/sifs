package show_test

import (
	"context"
	"encoding/json"
	"fmt"
	"maps"
	"slices"
	"strings"
	"testing"
	"time"
	"unicode"

	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/store"
)

// held is what a store holds that a person may be kept from: every value
// under an attribute the schema masks or hides, every value it holds at
// all (to find by), each thing by id, and the text its schema declares.
type held struct {
	box            *ontology.TBox
	masked, hidden []any
	all            []string
	things         map[string]*ontology.Entity
	schema         map[string]bool
	pages          map[string]string
}

func holds(t *testing.T, interp *interpreter.Interpreter) held {
	t.Helper()
	ctx := context.Background()
	h := held{box: interp.TBox(), things: map[string]*ontology.Entity{}, schema: map[string]bool{}, pages: map[string]string{}}
	add := func(attrs map[string][]any) {
		for attrID, vs := range attrs {
			d := h.display(attrID)
			for _, v := range vs {
				h.all = append(h.all, fmt.Sprint(v))
				switch {
				case d.Mask:
					h.masked = append(h.masked, v)
				case d.Hidden:
					h.hidden = append(h.hidden, v)
				}
			}
		}
	}
	things, err := interp.QueryEntities(ctx, store.EntityPattern{})
	if err != nil {
		t.Fatal(err)
	}
	for _, e := range things {
		h.things[e.ID] = e
		add(e.Attributes)
	}
	links, err := interp.QueryRelations(ctx, store.RelationPattern{})
	if err != nil {
		t.Fatal(err)
	}
	for _, l := range links {
		add(l.Attributes)
	}
	for id, et := range h.box.EntityTypes {
		h.schema[id], h.schema[et.Label] = true, true
	}
	for id, rt := range h.box.RelationTypes {
		h.schema[id], h.schema[rt.Label] = true, true
	}
	for id, rt := range h.box.RoleTypes {
		h.schema[id], h.schema[rt.Label] = true, true
		if rt.Display != nil {
			h.schema[rt.Display.Label] = true
		}
	}
	for id, at := range h.box.AttributeTypes {
		h.schema[id], h.schema[at.Label] = true, true
	}
	slices.Sort(h.all)
	h.all = slices.Compact(h.all)
	return h
}

func (h held) display(attrID string) *ontology.AttributeDisplay {
	if at := h.box.GetAttributeType(attrID); at != nil && at.Display != nil {
		return at.Display
	}
	return &ontology.AttributeDisplay{}
}

// leaks reads one read's JSON field by field and names each field that
// gives a person a value kept from them: a hidden value always, a masked
// one unless reveal. What a field may hold depends on the field, never on
// the value, so a text the store holds both as a masked value and as a
// shown one (a bank's name printed in a document and held as its name) is
// told apart by where it is:
//   - the values and the matched value of an attribute: none of a hidden
//     attribute; of a masked one, only when revealed or with no value kept
//     from a person in it; of any other attribute, anything;
//   - a thing's title and subtitle: a value of that thing's own attribute
//     that is shown as it is (a time by its instant or its day in UTC), or
//     text with no value kept from a person in it;
//   - a type, label, link, role or attribute: a name the schema declares, or
//     text with no value kept from a person in it;
//   - a proof's fact and each line of what it ruled out: the titles of the
//     things that same node's own data names, each only when it is the
//     title that thing's own page gives it and it passes as a title; the
//     values that same node's claim gives, each only when it passes as a
//     value of its attribute; and around them text with no value kept from
//     a person in it;
//   - every other text (ids, sources, claims and reasons): text with no
//     value kept from a person in it.
//
// A text value is in a field when it appears there whole: no letter or digit
// joins it on either side. A value of another type is in a field that
// equals it.
func (h held) leaks(read []byte, reveal bool) []string {
	var doc any
	if err := json.Unmarshal(read, &doc); err != nil {
		return []string{err.Error()}
	}
	kept := h.hidden
	if !reveal {
		kept = slices.Concat(h.masked, h.hidden)
	}
	var out []string
	free := func(path, s string) {
		for _, v := range kept {
			text, isText := v.(string)
			if isText && text != "" && holdsWhole(s, text) || !isText && s == fmt.Sprint(v) {
				out = append(out, fmt.Sprintf("%s %q gives %q", path, s, fmt.Sprint(v)))
			}
		}
	}
	line := func(node map[string]any) func(path, s string) {
		aside := h.aside(node, kept, reveal)
		return func(path, s string) {
			for _, v := range kept {
				text, isText := v.(string)
				if isText && text != "" && outside(s, text, aside) || !isText && s == fmt.Sprint(v) {
					out = append(out, fmt.Sprintf("%s %q gives %q", path, s, fmt.Sprint(v)))
				}
			}
		}
	}
	var walk func(path string, v any, field func(path, s string))
	walk = func(path string, v any, field func(path, s string)) {
		switch t := v.(type) {
		case string:
			field(path, t)
		case []any:
			for i, sub := range t {
				walk(fmt.Sprintf("%s[%d]", path, i), sub, field)
			}
		case map[string]any:
			attr, _ := t["attribute"].(string)
			id, isThing := t["id"].(string)
			if _, titled := t["title"]; !titled {
				isThing = false
			}
			for k, sub := range t {
				p := path + "." + k
				switch {
				case attr != "" && (k == "values" || k == "value"):
					walk(p, sub, func(path, s string) {
						switch d := h.display(attr); {
						case d.Hidden:
							out = append(out, fmt.Sprintf("%s %q is of hidden %s", path, s, attr))
						case d.Mask && !reveal:
							free(path, s)
						}
					})
				case isThing && (k == "title" || k == "subtitle"):
					walk(p, sub, func(path, s string) {
						if !slices.ContainsFunc(h.shownAsIs(id, reveal), func(v any) bool { return gives(s, v) }) {
							free(path, s)
						}
					})
				case k == "fact" || k == "line":
					walk(p, sub, line(t))
				case k == "type" || k == "label" || k == "link" || k == "role" || k == "attribute":
					walk(p, sub, func(path, s string) {
						if !h.schema[s] {
							free(path, s)
						}
					})
				default:
					walk(p, sub, free)
				}
			}
		}
	}
	walk("", doc, free)
	return out
}

// shownAsIs are the values of a thing's attributes that a person is given
// as they are.
func (h held) shownAsIs(id string, reveal bool) []any {
	var out []any
	if e := h.things[id]; e != nil {
		for attrID, vs := range e.Attributes {
			if d := h.display(attrID); d.Hidden || d.Mask && !reveal {
				continue
			}
			out = append(out, vs...)
		}
	}
	return out
}

// gives reports whether the text s is the value v: v itself, or a time by
// its instant or by its day, in UTC.
func gives(s string, v any) bool {
	if t, ok := v.(time.Time); ok {
		return s == t.UTC().Format(time.RFC3339) || s == t.UTC().Format(time.DateOnly)
	}
	return s == fmt.Sprint(v)
}

// titled records the title the page of the thing id gives it.
func (h held) titled(id, title string) { h.pages[id] = title }

// aside are what a proof's fact or a ruled-out line, node, may hold whole:
// the title of each thing node's own data names, when it is the title that
// thing's page gives it and it passes as a title (a value of the thing's
// own that is shown as it is, or text with no value kept from a person in
// it), and each value node's claim gives that passes as a value of its
// attribute (none of a hidden one; of a masked one, only as a page may
// show it).
func (h held) aside(node map[string]any, kept []any, reveal bool) []string {
	var out []string
	for _, thing := range slices.Concat(claimThings(object(node["claim"])), list(node["for"])) {
		id, title := text(thing["id"]), text(thing["title"])
		own := slices.ContainsFunc(h.shownAsIs(id, reveal), func(v any) bool { return gives(title, v) })
		if title != "" && h.pages[id] == title && (own || !holdsAny(title, kept)) {
			out = append(out, title)
		}
	}
	for _, c := range claimValues(object(node["claim"])) {
		d := h.display(text(c["attribute"]))
		if c["attribute"] != nil && !d.Hidden && (!d.Mask || reveal || !holdsAny(text(c["value"]), kept)) {
			out = append(out, text(c["value"]))
		}
	}
	return out
}

// claimValues are the claims at any depth of c that give a value.
func claimValues(c map[string]any) []map[string]any {
	if c == nil {
		return nil
	}
	var out []map[string]any
	if text(c["value"]) != "" {
		out = append(out, c)
	}
	out = append(out, claimValues(object(c["with"]))...)
	for _, part := range list(c["of"]) {
		out = append(out, claimValues(part)...)
	}
	return out
}

// holdsAny reports whether s gives any of kept: a text whole, any other
// value by being it.
func holdsAny(s string, kept []any) bool {
	return slices.ContainsFunc(kept, func(v any) bool {
		text, isText := v.(string)
		return isText && text != "" && holdsWhole(s, text) || !isText && s == fmt.Sprint(v)
	})
}

// outside reports whether v stands whole in s somewhere that no whole text
// of aside covers.
func outside(s, v string, aside []string) bool {
	for _, at := range wholeAt(s, v) {
		covered := slices.ContainsFunc(aside, func(a string) bool {
			return slices.ContainsFunc(wholeAt(s, a), func(t int) bool { return t <= at && at+len(v) <= t+len(a) })
		})
		if !covered {
			return true
		}
	}
	return false
}

// wholeAt are where v stands whole in s: no letter or digit joins it on
// either side.
func wholeAt(s, v string) []int {
	var out []int
	for i := 0; i <= len(s); {
		at := strings.Index(s[i:], v)
		if at < 0 {
			break
		}
		start, end := i+at, i+at+len(v)
		before, after := []rune(s[:start]), []rune(s[end:])
		if (len(before) == 0 || !word(before[len(before)-1])) && (len(after) == 0 || !word(after[0])) {
			out = append(out, start)
		}
		i = start + 1
	}
	return out
}

func holdsWhole(s, v string) bool {
	for i := 0; ; {
		at := strings.Index(s[i:], v)
		if at < 0 {
			return false
		}
		start, end := i+at, i+at+len(v)
		before, after := []rune(s[:start]), []rune(s[end:])
		if (len(before) == 0 || !word(before[len(before)-1])) && (len(after) == 0 || !word(after[0])) {
			return true
		}
		i = start + 1
	}
}

func word(r rune) bool { return unicode.IsLetter(r) || unicode.IsDigit(r) }

// everything is every read a person can make of the store, each as its
// JSON: its kinds, a find by each value it holds, and the page of each
// thing.
func everything(t *testing.T, r *show.Reader, h held) [][]byte {
	t.Helper()
	ctx := context.Background()
	kinds, err := r.Kinds(ctx)
	if err != nil {
		t.Fatal(err)
	}
	out := [][]byte{encode(t, kinds)}
	for _, words := range h.all {
		found, err := r.Find(ctx, words)
		if err != nil {
			t.Fatal(err)
		}
		out = append(out, encode(t, found))
	}
	for _, id := range sortedIDs(h.things) {
		page, err := r.Page(ctx, id)
		if err != nil {
			t.Fatal(err)
		}
		out = append(out, encode(t, page))
	}
	return out
}

// reportLeaks reports each leak in reads, which hold the page of every
// thing: a proof line may name a thing by the title its page gives it.
func reportLeaks(t *testing.T, h held, reads [][]byte, reveal bool) {
	t.Helper()
	for _, read := range reads {
		var page struct{ ID, Title string }
		if json.Unmarshal(read, &page) == nil && page.ID != "" {
			h.titled(page.ID, page.Title)
		}
	}
	n := 0
	for _, read := range reads {
		for _, leak := range h.leaks(read, reveal) {
			if n++; n <= 20 {
				t.Errorf("reveal %v: %s", reveal, leak)
			}
		}
	}
	if n > 20 {
		t.Errorf("reveal %v: %d more", reveal, n-20)
	}
}

// Every read of both schemas on both stores, searched field by field for
// every masked value without reveal and every hidden value with and
// without it. Mutations: give a title's first value raw in shown.first, and
// account numbers are given; put an organisation's key back in its id, and
// the ids give hidden keys; skip ids, subtitles or the ends of links in
// leaks, and that planted value is not found.
func TestMaskedAndHidden(t *testing.T) {
	for _, s := range []struct {
		name string
		open func(*testing.T) *interpreter.Interpreter
		raw  string
	}{
		{"files on memory", filesOnMemory, "DE89370400440532013000"},
		{"files on sqlite", func(t *testing.T) *interpreter.Interpreter { return openSQLite(t, filesOnSQLite(t)) }, "DE89370400440532013000"},
		{"shop on memory", shopOnMemory, anaTax},
		{"shop on sqlite", shopOnSQLite, anaTax},
	} {
		t.Run(s.name, func(t *testing.T) {
			interp := s.open(t)
			h := holds(t, interp)
			if !slices.Contains(h.masked, any(s.raw)) {
				t.Fatalf("%s is not among the masked values", s.raw)
			}
			i := slices.IndexFunc(h.hidden, func(v any) bool { _, ok := v.(string); return ok })
			if i < 0 {
				t.Fatal("no hidden text to plant")
			}
			planted := encode(t, show.Page{
				Thing: show.Thing{ID: "planted:" + h.hidden[i].(string), Type: "planted", Title: s.raw, Subtitle: s.raw},
				Links: []show.LinkGroup{{Link: "planted", Rows: []show.Link{{Ends: []show.End{{Role: "planted", Thing: show.Thing{ID: "planted", Title: s.raw}}}}}}},
			})
			got := h.leaks(planted, false)
			for _, field := range []string{".id ", ".title ", ".subtitle ", ".links[0].rows[0].ends[0].thing.title "} {
				if !slices.ContainsFunc(got, func(leak string) bool { return strings.HasPrefix(leak, field) }) {
					t.Fatalf("a hidden value planted in an id and a masked one in a title, a subtitle and a link row: %s not found in %q", field, got)
				}
			}
			plantedLines(t, h, s.raw, h.hidden[i].(string))
			t.Logf("%d masked and %d hidden values searched for", len(h.masked), len(h.hidden))
			reportLeaks(t, h, everything(t, show.New(interp, false), h), false)
			revealed := everything(t, show.New(interp, true), h)
			if !slices.ContainsFunc(revealed, func(read []byte) bool { return strings.Contains(string(read), s.raw) }) {
				t.Errorf("revealed, %q is not given", s.raw)
			}
			reportLeaks(t, h, revealed, true)
		})
	}
}

// plantedLines are proof lines planted in a page beside a store's own
// things, each found where a value kept from a person stands outside the
// titles of the things that line's own data names and the values its claim
// gives, and only there: a title or a value excuses nothing unless it
// passes its own check.
func plantedLines(t *testing.T, h held, raw, hidden string) {
	t.Helper()
	const period, short, end, depot = "2031-07-01 to 2031-07-31", "07", "2031-07-31", "Lagos Depot"
	p := h
	p.masked = append(slices.Clone(h.masked), short)
	p.things = maps.Clone(h.things)
	p.pages = maps.Clone(h.pages)
	plant := func(id, title string, attrs map[string][]any) show.Thing {
		p.things[id] = &ontology.Entity{ID: id, TypeID: "planted", Attributes: attrs}
		p.titled(id, title)
		return show.Thing{ID: id, Type: "planted", Title: title}
	}
	statement := plant("planted:period", period, map[string][]any{"planted.period": {period}, "planted.to": {end}})
	titledRaw := plant("planted:raw", raw, nil)
	twin := plant("planted:twin", raw, map[string][]any{"planted.name": {raw}})
	lagos := plant("planted:depot", depot, map[string][]any{"planted.name": {depot}, "planted.code": {raw}})
	misnamed := lagos
	misnamed.Title = raw
	var maskedAttr, hiddenAttr string
	for _, id := range sortedIDs(h.box.AttributeTypes) {
		switch d := h.display(id); {
		case d.Mask && maskedAttr == "":
			maskedAttr = id
		case d.Hidden && hiddenAttr == "":
			hiddenAttr = id
		}
	}
	link := func(things ...show.Thing) *show.Claim {
		c := &show.Claim{Test: "link", Link: "planted", LinkLabel: "Planted", Ends: []show.End{}}
		for _, thing := range things {
			c.Ends = append(c.Ends, show.End{Role: "planted", Label: "Thing", Thing: thing})
		}
		return c
	}
	value := func(thing show.Thing, attr, v string) *show.Claim {
		return &show.Claim{Test: "is", Thing: &thing, Attribute: attr, Label: "Value", Value: v}
	}
	for _, c := range []struct {
		name          string
		proof         show.Proof
		found, inData bool
	}{
		{"a masked value in a fact", show.Proof{Fact: "Statement account: Account " + raw}, true, false},
		{"a masked value in a ruled-out line", show.Proof{RuledOut: []show.RuledOut{{Line: "and no: a match (for " + raw + ", Statement " + period + ")"}}}, true, false},
		{"a hidden value in a fact", show.Proof{Fact: "Held at: Key " + hidden}, true, false},
		{"a short masked value inside a title the line names", show.Proof{Fact: "Latest statement: Statement " + period, Claim: link(statement)}, false, false},
		{"the same value beside that title", show.Proof{Fact: "Latest statement: Statement " + period + "; Key " + short, Claim: link(statement)}, true, false},
		{"a thing titled by a raw masked value, named", show.Proof{RuledOut: []show.RuledOut{{Line: "and no: Older statements (for " + raw + ")"}}}, true, false},
		{"a masked value raw, the same text as the public title of a thing the data does not name", show.Proof{Fact: "Planted: Thing " + depot + ", Thing " + raw, Claim: link(lagos)}, true, false},
		{"the same text, that thing named", show.Proof{Fact: "Planted: Thing " + raw, Claim: link(twin)}, false, false},
		{"a masked value raw before the titles", show.Proof{Fact: raw + " Planted: Thing " + depot + ", Thing " + period, Claim: link(lagos, statement)}, true, false},
		{"a masked value raw after the titles", show.Proof{Fact: "Planted: Thing " + depot + ", Thing " + period + " " + raw, Claim: link(lagos, statement)}, true, false},
		{"a masked value raw between the titles", show.Proof{Fact: "Planted: Thing " + depot + ", " + raw + ", Thing " + period, Claim: link(lagos, statement)}, true, false},
		{"a public date given as the value, a short masked value inside it", show.Proof{Fact: period + " Value " + end, Claim: value(statement, "planted.to", end)}, false, false},
		{"that short value outside the title and the value", show.Proof{Fact: period + " Value " + end + "; " + short, Claim: value(statement, "planted.to", end)}, true, false},
		{"a masked attribute's value given raw as the value", show.Proof{Fact: depot + " Value " + raw, Claim: value(lagos, maskedAttr, raw)}, true, true},
		{"a hidden attribute's value given as the value", show.Proof{Fact: depot + " Value " + hidden, Claim: value(lagos, hiddenAttr, hidden)}, true, true},
		{"a thing named by a title that is not its page's", show.Proof{Fact: "Planted: Thing " + raw, Claim: link(misnamed)}, true, false},
		{"a title that is itself a raw masked value", show.Proof{Fact: "Planted: Thing " + raw, Claim: link(titledRaw)}, true, true},
	} {
		read := encode(t, show.Page{Thing: show.Thing{ID: "planted", Type: "planted", Title: "planted"},
			Links: []show.LinkGroup{{Link: "planted", Rows: []show.Link{{Ends: []show.End{}, Proof: &c.proof}}}}})
		got := p.leaks(read, false)
		inLine := slices.ContainsFunc(got, func(leak string) bool {
			return strings.HasPrefix(leak, ".links[0].rows[0].proof.fact ") || strings.HasPrefix(leak, ".links[0].rows[0].proof.ruled_out[0].line ")
		})
		inData := slices.ContainsFunc(got, func(leak string) bool { return strings.HasPrefix(leak, ".links[0].rows[0].proof.claim.") })
		if inLine != c.found || inData != c.inData || !c.found && len(got) > 0 {
			t.Errorf("%s: found in the line %v, want %v; in the data %v, want %v: %q", c.name, inLine, c.found, inData, c.inData, got)
		}
	}
}
