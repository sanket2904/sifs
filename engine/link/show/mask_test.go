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
	titles         []string
}

func holds(t *testing.T, interp *interpreter.Interpreter) held {
	t.Helper()
	ctx := context.Background()
	h := held{box: interp.TBox(), things: map[string]*ontology.Entity{}, schema: map[string]bool{}}
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
//     things it names, each as that thing's own page gives it and only when
//     that title passes as a title, and around them text with no value kept
//     from a person in it;
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
	line := func(path, s string) {
		for _, v := range kept {
			text, isText := v.(string)
			if isText && text != "" && h.outsideTitles(s, text) || !isText && s == fmt.Sprint(v) {
				out = append(out, fmt.Sprintf("%s %q gives %q", path, s, fmt.Sprint(v)))
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
					walk(p, sub, line)
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

// excuse lets a proof line name the thing id by title, the one its page
// gives it, when that title passes as a title: a value of the thing's own
// that is shown as it is, or text with no value kept from a person in it.
func (h *held) excuse(id, title string, reveal bool) {
	kept := h.hidden
	if !reveal {
		kept = slices.Concat(h.masked, h.hidden)
	}
	own := slices.ContainsFunc(h.shownAsIs(id, reveal), func(v any) bool { return gives(title, v) })
	free := !slices.ContainsFunc(kept, func(v any) bool {
		text, isText := v.(string)
		return isText && text != "" && holdsWhole(title, text) || !isText && title == fmt.Sprint(v)
	})
	if title != "" && (own || free) {
		h.titles = append(h.titles, title)
	}
}

// outsideTitles reports whether v stands whole in s somewhere that no
// whole title a proof line may name covers.
func (h held) outsideTitles(s, v string) bool {
	for _, at := range wholeAt(s, v) {
		covered := slices.ContainsFunc(h.titles, func(title string) bool {
			return slices.ContainsFunc(wholeAt(s, title), func(t int) bool { return t <= at && at+len(v) <= t+len(title) })
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
			h.excuse(page.ID, page.Title, reveal)
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
// things: one named by a made-up period whose title holds a short masked
// value whole, and one titled by the raw masked value raw. Each is found
// where a value kept from a person stands outside a title a line may name,
// and only there.
func plantedLines(t *testing.T, h held, raw, hidden string) {
	t.Helper()
	const period, short = "2031-07-01 to 2031-07-31", "07"
	p := h
	p.masked = append(slices.Clone(h.masked), short)
	p.things = maps.Clone(h.things)
	p.things["planted:period"] = &ontology.Entity{ID: "planted:period", TypeID: "planted", Attributes: map[string][]any{"planted.period": {period}}}
	p.things["planted:raw"] = &ontology.Entity{ID: "planted:raw", TypeID: "planted"}
	p.excuse("planted:period", period, false)
	p.excuse("planted:raw", raw, false)
	for _, c := range []struct {
		name  string
		proof show.Proof
		found bool
	}{
		{"a masked value in a fact", show.Proof{Fact: "Statement account: Account " + raw}, true},
		{"a masked value in a ruled-out line", show.Proof{RuledOut: []show.RuledOut{{Line: "and no: a match (for " + raw + ", Statement " + period + ")"}}}, true},
		{"a hidden value in a fact", show.Proof{Fact: "Held at: Key " + hidden}, true},
		{"a short masked value inside a title the line names", show.Proof{Fact: "Latest statement: Statement " + period}, false},
		{"the same value beside that title", show.Proof{Fact: "Latest statement: Statement " + period + "; Key " + short}, true},
		{"a thing titled by a raw masked value, named", show.Proof{RuledOut: []show.RuledOut{{Line: "and no: Older statements (for " + raw + ")"}}}, true},
	} {
		read := encode(t, show.Page{Thing: show.Thing{ID: "planted", Type: "planted", Title: "planted"},
			Links: []show.LinkGroup{{Link: "planted", Rows: []show.Link{{Ends: []show.End{}, Proof: &c.proof}}}}})
		got := p.leaks(read, false)
		inProof := slices.ContainsFunc(got, func(leak string) bool { return strings.Contains(leak, ".proof.") })
		if inProof != c.found || !c.found && len(got) > 0 {
			t.Errorf("%s: found %v, want %v: %q", c.name, inProof, c.found, got)
		}
	}
}
