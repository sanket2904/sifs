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
	pages          map[string]map[string]any
}

func holds(t *testing.T, interp *interpreter.Interpreter) held {
	t.Helper()
	ctx := context.Background()
	h := held{box: interp.TBox(), things: map[string]*ontology.Entity{}, schema: map[string]bool{}, pages: map[string]map[string]any{}}
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
//   - a proof's fact and each line of what it ruled out: exactly what that
//     node's own data makes, each part judged where it stands (see why); a
//     line that is not is reported, and searched whole;
//   - a claim on a hidden attribute: that it is hidden, and nothing else
//     about that attribute;
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
	// why judges the why line node[key], at path, by the parts its data
	// makes. A title must be the title its thing's page gives and pass
	// as a title; a fact's value must be what the page of its thing (its
	// link's row) shows for its attribute, in the line and in the data;
	// a condition's value must not be of a hidden attribute, and is
	// searched with the fixed words and labels. A part that fails is
	// reported and searched too.
	why := func(path string, node map[string]any, key string) {
		var parts []part
		var err error
		if key == "fact" {
			parts, err = factParts(object(node["claim"]))
		} else {
			parts = ruledOutParts(node)
		}
		line, at := text(node[key]), path+"."+key
		made := err == nil && joined(parts) == line
		if !made {
			out = append(out, fmt.Sprintf("%s %q is not what its data makes, %q (%v)", at, line, joined(parts), err))
			free(at, line)
		}
		var searched strings.Builder
		for _, p := range parts {
			wrong := ""
			switch p.kind {
			case titleOf:
				if !h.passesAsTitle(p.of, kept, reveal) {
					wrong = fmt.Sprintf("names %s by a title its page does not give, or that gives a value kept from a person", text(p.of["id"]))
				}
			case factValue:
				if !factShown(h.pages, p.of) {
					wrong = fmt.Sprintf("gives %s %q, which its page does not show", text(p.of["attribute"]), p.text)
					out = append(out, fmt.Sprintf("%s %q %s", path+p.at, p.text, wrong))
				}
			case conditionValue:
				if attr := text(p.of["attribute"]); h.display(attr).Hidden {
					wrong = "gives a value of hidden " + attr
				}
				free(path+p.at, p.text)
			}
			if made && wrong != "" {
				out = append(out, fmt.Sprintf("%s %q %s", at, line, wrong))
			}
			if p.kind == said || p.kind == conditionValue || wrong != "" {
				searched.WriteString(p.text)
			} else {
				searched.WriteString("\x00")
			}
		}
		if made {
			free(at, searched.String())
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
			if t["hidden"] == true {
				for _, k := range []string{"attribute", "label", "value", "plus"} {
					if sub, ok := t[k]; ok {
						out = append(out, fmt.Sprintf("%s.%s %q is said of a hidden attribute", path, k, fmt.Sprint(sub)))
					}
				}
			}
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
					why(path, t, k)
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

// passesAsTitle reports whether thing's title is the title its own page
// gives it and passes as a title: a value of the thing's own that is shown
// as it is, or text with no value kept from a person in it.
func (h held) passesAsTitle(thing map[string]any, kept []any, reveal bool) bool {
	id, title := text(thing["id"]), text(thing["title"])
	own := slices.ContainsFunc(h.shownAsIs(id, reveal), func(v any) bool { return gives(title, v) })
	page := h.pages[id]
	return page != nil && page["title"] == title && (own || !holdsAny(title, kept))
}

// holdsAny reports whether s gives any of kept: a text whole, any other
// value by being it.
func holdsAny(s string, kept []any) bool {
	return slices.ContainsFunc(kept, func(v any) bool {
		text, isText := v.(string)
		return isText && text != "" && holdsWhole(s, text) || !isText && s == fmt.Sprint(v)
	})
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
// thing: a why line names a thing by the title its page gives it, and
// gives a value as its page shows it.
func reportLeaks(t *testing.T, h held, reads [][]byte, reveal bool) {
	t.Helper()
	for _, read := range reads {
		var page map[string]any
		if json.Unmarshal(read, &page) == nil && text(page["id"]) != "" {
			h.pages[text(page["id"])] = page
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

// What a report on a planted why line says.
const (
	notMade  = " is not what its data makes"
	found    = " gives "
	badTitle = " by a title its page does not give"
	offPage  = " which its page does not show"
)

// plantedLines are proof lines planted in a page beside a store's own
// things, each with the report it must get: in the line (none, or one that
// says what is wrong) and in its data.
func plantedLines(t *testing.T, h held, raw, hidden string) {
	t.Helper()
	const period, short, end, depot, osakaDepot, clinic = "2031-07-01 to 2031-07-31", "07", "2031-07-31", "Lagos Depot", "Osaka Depot", "Clinica Lima"
	p := h
	p.masked = append(slices.Clone(h.masked), short)
	p.things = maps.Clone(h.things)
	p.pages = maps.Clone(h.pages)
	plant := func(id, title string, attrs map[string][]any) show.Thing {
		p.things[id] = &ontology.Entity{ID: id, TypeID: "planted", Attributes: attrs}
		values := []any{}
		for _, attr := range sortedIDs(attrs) {
			values = append(values, map[string]any{"attribute": attr, "values": attrs[attr]})
		}
		p.pages[id] = map[string]any{"id": id, "type": "planted", "title": title, "values": values, "links": []any{}}
		return show.Thing{ID: id, Type: "planted", Title: title}
	}
	var maskedAttr, hiddenAttr string
	var public []string
	for _, id := range sortedIDs(h.box.AttributeTypes) {
		switch d := h.display(id); {
		case d.Mask && maskedAttr == "":
			maskedAttr = id
		case d.Hidden && hiddenAttr == "":
			hiddenAttr = id
		case !d.Mask && !d.Hidden:
			public = append(public, id)
		}
	}
	name, code := public[0], public[1]
	statement := plant("planted:period", period, map[string][]any{name: {period}, code: {end}})
	titledRaw := plant("planted:raw", raw, nil)
	twin := plant("planted:twin", raw, map[string][]any{name: {raw}})
	lagos := plant("planted:depot", depot, map[string][]any{name: {depot}, code: {raw}})
	printed := plant("planted:clinic", clinic, map[string][]any{name: {clinic}, code: {raw}})
	osaka := plant("planted:osaka", osakaDepot, map[string][]any{name: {osakaDepot}})
	p.pages[osaka.ID]["links"] = []any{map[string]any{"link": "planted", "rows": []any{map[string]any{
		"ends":       []any{map[string]any{"thing": map[string]any{"id": lagos.ID, "title": depot}}},
		"attributes": []any{map[string]any{"attribute": code, "values": []any{"2031-07-01"}}},
	}}}}
	nowhere := show.Thing{ID: "planted:nowhere", Type: "planted", Title: "Nowhere"}
	misnamed := lagos
	misnamed.Title = raw
	hiddenLabel := h.box.AttributeTypes[hiddenAttr].Label
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
	since := func(v string) *show.Claim {
		c := link(osaka, lagos)
		c.Test, c.Attribute, c.Label, c.Value = "is", code, "Since", v
		return c
	}
	notShown := func(c show.Claim) *show.Claim {
		c.Test, c.Thing, c.Hidden = "has", &osaka, true
		return &c
	}
	condition := func(attr, v string) *show.Claim {
		return &show.Claim{Test: "is", Attribute: attr, Label: "Value", Value: v}
	}
	deep := &show.Claim{Test: "all_of", Of: []show.Claim{{Test: "any_of", Of: []show.Claim{{Test: "link", Link: "planted", LinkLabel: "Planted", With: condition(maskedAttr, raw)}}}}}
	sinceLine := "Planted: Thing " + osakaDepot + ", Thing " + depot + "; Since: "
	hiddenLine := osakaDepot + " has a value that is not shown"
	for _, c := range []struct {
		name   string
		proof  show.Proof
		line   string
		inData bool
	}{
		{"a masked value in a fact", show.Proof{Fact: "Statement account: Account " + raw}, found, false},
		{"a masked value in a ruled-out line", show.Proof{RuledOut: []show.RuledOut{{Line: "and no: a match (for " + raw + ", Statement " + period + ")"}}}, found, false},
		{"a hidden value in a fact", show.Proof{Fact: "Held at: Key " + hidden}, found, false},
		{"a short masked value inside a title the line names", show.Proof{Fact: "Planted: Thing " + period, Claim: link(statement)}, "", false},
		{"the same value beside that title", show.Proof{Fact: "Planted: Thing " + period + "; Key " + short, Claim: link(statement)}, found, false},
		{"a thing titled by a raw masked value, named", show.Proof{RuledOut: []show.RuledOut{{Line: "and no: Older statements (for " + raw + ")"}}}, found, false},
		{"a masked value raw, the same text as the public title of a thing the data does not name", show.Proof{Fact: "Planted: Thing " + depot + ", Thing " + raw, Claim: link(lagos)}, found, false},
		{"the same text, that thing named", show.Proof{Fact: "Planted: Thing " + raw, Claim: link(twin)}, "", false},
		{"a masked value raw before the titles", show.Proof{Fact: raw + " Planted: Thing " + depot + ", Thing " + period, Claim: link(lagos, statement)}, found, false},
		{"a masked value raw after the titles", show.Proof{Fact: "Planted: Thing " + depot + ", Thing " + period + " " + raw, Claim: link(lagos, statement)}, found, false},
		{"a masked value raw between the titles", show.Proof{Fact: "Planted: Thing " + depot + ", " + raw + ", Thing " + period, Claim: link(lagos, statement)}, found, false},
		{"a public date given as the value, a short masked value inside it", show.Proof{Fact: period + " Value " + end, Claim: value(statement, code, end)}, "", false},
		{"that short value outside the title and the value", show.Proof{Fact: period + " Value " + end + "; " + short, Claim: value(statement, code, end)}, found, false},
		{"a masked attribute's value given raw as the value", show.Proof{Fact: depot + " Value " + raw, Claim: value(lagos, maskedAttr, raw)}, found, true},
		{"a hidden attribute's value given as the value", show.Proof{Fact: depot + " Value " + hidden, Claim: value(lagos, hiddenAttr, hidden)}, found, true},
		{"a thing named by a title that is not its page's", show.Proof{Fact: "Planted: Thing " + raw, Claim: link(misnamed)}, found, false},
		{"a title that is itself a raw masked value", show.Proof{Fact: "Planted: Thing " + raw, Claim: link(titledRaw)}, found, true},

		{"a public value given, then repeated outside its place", show.Proof{Fact: clinic + " Value " + raw + "; secret " + raw, Claim: value(printed, code, raw)}, found, false},
		{"that public value in its place only", show.Proof{Fact: clinic + " Value " + raw, Claim: value(printed, code, raw)}, "", false},
		{"a masked value given raw under a public attribute's id", show.Proof{Fact: osakaDepot + " Value " + raw, Claim: value(osaka, name, raw)}, offPage, true},
		{"a value the page shows for another attribute of the same thing", show.Proof{Fact: depot + " Value " + raw, Claim: value(lagos, name, raw)}, offPage, true},
		{"a value of a thing the store does not hold", show.Proof{Fact: "Nowhere Value Lima", Claim: value(nowhere, name, "Lima")}, offPage, true},
		{"a link value that is not the link's row", show.Proof{Fact: sinceLine + "2031-08-15", Claim: since("2031-08-15")}, offPage, true},
		{"a link value that is the link's row", show.Proof{Fact: sinceLine + "2031-07-01", Claim: since("2031-07-01")}, "", false},
		{"a hidden claim that says only that", show.Proof{Fact: hiddenLine, Claim: notShown(show.Claim{})}, "", false},
		{"a hidden claim with a label", show.Proof{Fact: hiddenLine, Claim: notShown(show.Claim{Label: hiddenLabel})}, "", true},
		{"a hidden claim with an attribute id", show.Proof{Fact: hiddenLine, Claim: notShown(show.Claim{Attribute: hiddenAttr})}, "", true},
		{"a hidden claim with a value", show.Proof{Fact: hiddenLine, Claim: notShown(show.Claim{Value: "Lima"})}, "", true},
		{"a hidden claim with a plus", show.Proof{Fact: hiddenLine, Claim: notShown(show.Claim{Plus: "2 days"})}, "", true},
		{"a title repeated outside its place", show.Proof{Fact: "Planted: Thing " + raw + "; " + raw, Claim: link(twin)}, found, false},
		{"a thing the data names and the line does not, a masked value raw in the line", show.Proof{Fact: "secret " + raw, Claim: link(twin)}, found, false},
		{"a line with one word more than its data", show.Proof{Fact: "Planted: Thing " + osakaDepot + " now", Claim: link(osaka)}, notMade, false},
		{"a line with another thing's title", show.Proof{Fact: "Planted: Thing " + depot, Claim: link(osaka)}, notMade, false},
		{"a thing given another thing's title", show.Proof{Fact: "Planted: Thing " + depot, Claim: link(show.Thing{ID: osaka.ID, Type: "planted", Title: depot})}, badTitle, false},
		{"a condition's constant, a masked value given raw", show.Proof{RuledOut: []show.RuledOut{{Line: "and no: Value: " + raw, Claim: condition(maskedAttr, raw), For: []show.Thing{}}}}, found, true},
		{"a condition's constant, a masked value given raw under a public attribute's id", show.Proof{RuledOut: []show.RuledOut{{Line: "and no: Value: " + raw, Claim: condition(name, raw), For: []show.Thing{}}}}, found, true},
		{"a masked value raw three levels down, in the data only", show.Proof{RuledOut: []show.RuledOut{{Line: "and no: Planted, one with Value: ••••9221", Claim: deep, For: []show.Thing{}}}}, notMade, true},
	} {
		read := encode(t, show.Page{Thing: show.Thing{ID: "planted", Type: "planted", Title: "planted"},
			Links: []show.LinkGroup{{Link: "planted", Rows: []show.Link{{Ends: []show.End{}, Proof: &c.proof}}}}})
		got := p.leaks(read, false)
		var inLine []string
		for _, leak := range got {
			if strings.HasPrefix(leak, ".links[0].rows[0].proof.fact ") || strings.HasPrefix(leak, ".links[0].rows[0].proof.ruled_out[0].line ") {
				inLine = append(inLine, leak)
			}
		}
		inData := slices.ContainsFunc(got, func(leak string) bool {
			return strings.HasPrefix(leak, ".links[0].rows[0].proof.claim.") || strings.HasPrefix(leak, ".links[0].rows[0].proof.ruled_out[0].claim.") ||
				strings.HasPrefix(leak, ".links[0].rows[0].proof.ruled_out[0].for")
		})
		lineRight := c.line == "" && len(inLine) == 0 || c.line != "" && slices.ContainsFunc(inLine, func(leak string) bool { return strings.Contains(leak, c.line) })
		if !lineRight || inData != c.inData || c.line == "" && !c.inData && len(got) > 0 {
			t.Errorf("%s: in the line %q, want %q; in the data %v, want %v: %q", c.name, inLine, c.line, inData, c.inData, got)
		}
	}
}
