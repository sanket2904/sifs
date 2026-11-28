package show_test

import (
	"fmt"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
)

// The fixed words a why line holds besides the schema's labels, the titles
// of the things it names and the values it gives, as sample/README.md lists
// them.
const (
	wordIs       = " is "
	wordHas      = " has "
	wordNotShown = "a value that is not shown"
	wordAndNo    = "and no: "
	wordAMatch   = "a match"
	wordFor      = " (for "
	wordOneWith  = ", one with "
	wordPlus     = " plus "
)

var testWords = map[string]string{
	"is": ": ", "more_than": " more than ", "less_than": " less than ", "at_least": " at least ", "at_most": " at most ",
	"in_the_past": " in the past", "in_the_future": " in the future",
}

func text(v any) string { s, _ := v.(string); return s }

func object(v any) map[string]any { m, _ := v.(map[string]any); return m }

// A part is one piece of a why line as its data makes it, and at is where
// its text stands in the node's data. Of is the thing whose title it is, or
// the claim whose value it is.
type part struct {
	kind partKind
	text string
	at   string
	of   map[string]any
}

type partKind int

const (
	said           partKind = iota // fixed words and the schema's labels
	titleOf                        // the title of a thing the data names
	factValue                      // the value a fact gives, of a thing or a link
	conditionValue                 // the value a rule's condition tests for
)

func words(s string) part { return part{kind: said, text: s} }

func joined(parts []part) string {
	var b strings.Builder
	for _, p := range parts {
		b.WriteString(p.text)
	}
	return b.String()
}

// The shapes a why's data takes, as sample/README.md describes a proof. Each
// object in a node's data is exactly one shape of the place it stands in: it
// makes one of that shape's tests, has every field the shape must have, and
// no field but those the shape may have. Every field is then said by the
// line or judged where it stands: a thing by its page, a label by its id, a
// value by its subject's page or as a condition's constant, with and of as
// claims of their own.
//
//	in         shape                tests                         fields besides test
//	fact       kind fact            type                          thing type label
//	fact       value fact           is                            thing attribute label value
//	fact       has fact             has                           thing attribute label
//	fact       not shown fact       has                           thing hidden
//	fact       link fact            link                          link link_label ends
//	fact       link value fact      is                            link link_label ends attribute label value
//	fact       link has fact        has                           link link_label ends attribute label
//	fact       link not shown fact  has                           link link_label ends hidden
//	condition  kind                 type                          type label
//	condition  link                 link                          link link_label
//	condition  link one with        link                          link link_label with
//	condition  has                  has                           attribute label
//	condition  value                is more_than less_than        attribute label value
//	                                at_least at_most
//	condition  time                 in_the_past in_the_future     attribute label, may plus
//	condition  not shown            the tests of has, value, time hidden
//	condition  link has             has                           link link_label attribute label
//	condition  link value           is                            link link_label attribute label value
//	condition  link time            in_the_past in_the_future     link link_label attribute label, may plus
//	condition  link not shown       has is in_the_past            link link_label hidden
//	                                in_the_future
//	condition  any or all           any_of all_of                 of (one condition or more)
//	with       with has             has                           attribute label
//	with       with value           is                            attribute label value
//	with       with not shown       has is                        hidden
//	thing      thing                                              id type title, may subtitle
//	end        end                                                role label thing
//	proof      proof                                              fact status claim, may rule cut sources because ruled_out
//	ruled out  ruled out                                          line for, may claim
type shape struct {
	in, name  string
	tests     []string
	must, may []string
}

var (
	valueTests = []string{"is", "more_than", "less_than", "at_least", "at_most"}
	timeTests  = []string{"in_the_past", "in_the_future"}
	shapes     = []shape{
		{"fact", "kind fact", []string{"type"}, []string{"thing", "type", "label"}, nil},
		{"fact", "value fact", []string{"is"}, []string{"thing", "attribute", "label", "value"}, nil},
		{"fact", "has fact", []string{"has"}, []string{"thing", "attribute", "label"}, nil},
		{"fact", "not shown fact", []string{"has"}, []string{"thing", "hidden"}, nil},
		{"fact", "link fact", []string{"link"}, []string{"link", "link_label", "ends"}, nil},
		{"fact", "link value fact", []string{"is"}, []string{"link", "link_label", "ends", "attribute", "label", "value"}, nil},
		{"fact", "link has fact", []string{"has"}, []string{"link", "link_label", "ends", "attribute", "label"}, nil},
		{"fact", "link not shown fact", []string{"has"}, []string{"link", "link_label", "ends", "hidden"}, nil},
		{"condition", "kind", []string{"type"}, []string{"type", "label"}, nil},
		{"condition", "link", []string{"link"}, []string{"link", "link_label"}, nil},
		{"condition", "link one with", []string{"link"}, []string{"link", "link_label", "with"}, nil},
		{"condition", "has", []string{"has"}, []string{"attribute", "label"}, nil},
		{"condition", "value", valueTests, []string{"attribute", "label", "value"}, nil},
		{"condition", "time", timeTests, []string{"attribute", "label"}, []string{"plus"}},
		{"condition", "not shown", slices.Concat([]string{"has"}, valueTests, timeTests), []string{"hidden"}, nil},
		{"condition", "link has", []string{"has"}, []string{"link", "link_label", "attribute", "label"}, nil},
		{"condition", "link value", []string{"is"}, []string{"link", "link_label", "attribute", "label", "value"}, nil},
		{"condition", "link time", timeTests, []string{"link", "link_label", "attribute", "label"}, []string{"plus"}},
		{"condition", "link not shown", slices.Concat([]string{"has", "is"}, timeTests), []string{"link", "link_label", "hidden"}, nil},
		{"condition", "any or all", []string{"any_of", "all_of"}, []string{"of"}, nil},
		{"with", "with has", []string{"has"}, []string{"attribute", "label"}, nil},
		{"with", "with value", []string{"is"}, []string{"attribute", "label", "value"}, nil},
		{"with", "with not shown", []string{"has", "is"}, []string{"hidden"}, nil},
		{"thing", "thing", nil, []string{"id", "type", "title"}, []string{"subtitle"}},
		{"end", "end", nil, []string{"role", "label", "thing"}, nil},
		{"proof", "proof", nil, []string{"fact", "status", "claim"}, []string{"rule", "cut", "sources", "because", "ruled_out"}},
		{"ruled out", "ruled out", nil, []string{"line", "for"}, []string{"claim"}},
	}
)

// holdsField is what each field holds in JSON.
var holdsField = func() map[string]func(any) bool {
	isText := func(v any) bool { _, ok := v.(string); return ok }
	all := func(least int, each func(any) bool) func(any) bool {
		return func(v any) bool {
			vs, ok := v.([]any)
			return ok && len(vs) >= least && !slices.ContainsFunc(vs, func(v any) bool { return !each(v) })
		}
	}
	isObject := func(v any) bool { _, ok := v.(map[string]any); return ok }
	out := map[string]func(any) bool{
		"hidden": func(v any) bool { return v == true },
		"thing":  isObject, "with": isObject, "claim": isObject,
		"ends": all(1, isObject), "of": all(1, isObject),
		"for": all(0, isObject), "because": all(0, isObject), "ruled_out": all(0, isObject),
		"sources": all(0, isText),
	}
	for _, f := range []string{"test", "type", "link", "link_label", "attribute", "label", "value", "plus", "role", "id", "title", "subtitle", "fact", "status", "rule", "cut", "line"} {
		out[f] = isText
	}
	return out
}()

// fits reports whether data has exactly the fields of shape s, each holding
// what that field holds.
func fits(data map[string]any, s shape) bool {
	must := s.must
	if s.tests != nil {
		must = append([]string{"test"}, must...)
	}
	for _, f := range must {
		if _, ok := data[f]; !ok {
			return false
		}
	}
	for f, v := range data {
		if !slices.Contains(must, f) && !slices.Contains(s.may, f) || holdsField[f] == nil || !holdsField[f](v) {
			return false
		}
	}
	return true
}

// labelOf is the label the schema gives id as the kind of name field holds,
// and whether it declares id so.
func labelOf(box *ontology.TBox, field, id string) (string, bool) {
	switch field {
	case "type":
		if et := box.GetEntityType(id); et != nil {
			return et.Label, true
		}
	case "link":
		if rt := box.GetRelationType(id); rt != nil {
			return rt.Label, true
		}
	case "role":
		if rt := box.GetRoleType(id); rt != nil {
			return rt.Label, true
		}
	case "attribute":
		if at := box.GetAttributeType(id); at != nil {
			return at.Label, true
		}
	}
	return "", false
}

// A rebuilder makes a why line from its node's data, object by object, each
// of one shape, and counts in seen, when it has one, the shapes it meets.
type rebuilder struct {
	box  *ontology.TBox
	seen map[string]int
}

// shape is the shape of data, which stands in in, whose every label is its
// id's label and which names no hidden attribute.
func (b rebuilder) shape(in string, data map[string]any) (shape, error) {
	i := slices.IndexFunc(shapes, func(s shape) bool {
		return s.in == in && (s.tests == nil || slices.Contains(s.tests, text(data["test"]))) && fits(data, s)
	})
	if i < 0 {
		return shape{}, fmt.Errorf("%s data of no shape a why gives: %v", in, data)
	}
	for _, l := range []struct{ id, label string }{{"type", "label"}, {"attribute", "label"}, {"role", "label"}, {"link", "link_label"}} {
		id, hasID := data[l.id]
		label, hasLabel := data[l.label]
		if !hasID || !hasLabel {
			continue
		}
		want, declared := labelOf(b.box, l.id, text(id))
		if !declared || label != want {
			return shape{}, fmt.Errorf("%s data labels %s %v %q, which is not the label the schema gives it", in, l.id, id, label)
		}
		if l.id == "attribute" && b.box.GetAttributeType(text(id)).Display != nil && b.box.GetAttributeType(text(id)).Display.Hidden {
			return shape{}, fmt.Errorf("%s data names hidden attribute %v", in, id)
		}
	}
	if b.seen != nil {
		b.seen[in+": "+shapes[i].name]++
	}
	return shapes[i], nil
}

// subject is the thing a fact's claim gives its value of, and, of a link,
// the link and the id of the thing at its other end: the thing the line
// names, or the first thing a link's line names. Its page, or its row of
// that link, must show the value.
func subject(c map[string]any) (thing map[string]any, link, other string) {
	if thing := object(c["thing"]); thing != nil {
		return thing, "", ""
	}
	ends := list(c["ends"])
	if len(ends) == 0 {
		return nil, "", ""
	}
	if len(ends) == 2 {
		other = text(object(ends[1]["thing"])["id"])
	}
	return object(ends[0]["thing"]), text(c["link"]), other
}

// thing is a thing's title in a node's data, at at.
func (b rebuilder) thing(thing map[string]any, at string) (part, error) {
	if _, err := b.shape("thing", thing); err != nil {
		return part{}, err
	}
	return part{kind: titleOf, text: text(thing["title"]), at: at + ".title", of: thing}, nil
}

// fact is a proof node's line as its data says it.
func (b rebuilder) fact(node map[string]any) ([]part, error) {
	if _, err := b.shape("proof", node); err != nil {
		return nil, err
	}
	c := object(node["claim"])
	s, err := b.shape("fact", c)
	if err != nil {
		return nil, err
	}
	label := text(c["label"])
	value := part{kind: factValue, text: text(c["value"]), at: ".claim.value", of: c}
	if thing, link, _ := subject(c); link == "" {
		title, err := b.thing(thing, ".claim.thing")
		return append([]part{title}, map[string][]part{
			"kind fact":      {words(wordIs + label)},
			"value fact":     {words(" " + label + " "), value},
			"has fact":       {words(wordHas + label)},
			"not shown fact": {words(wordHas + wordNotShown)},
		}[s.name]...), err
	}
	out := []part{words(text(c["link_label"]) + ": ")}
	for i, e := range list(c["ends"]) {
		if _, err := b.shape("end", e); err != nil {
			return nil, err
		}
		if i > 0 {
			out = append(out, words(", "))
		}
		if role := text(e["label"]); role != "" {
			out = append(out, words(role+" "))
		}
		title, err := b.thing(object(e["thing"]), fmt.Sprintf(".claim.ends[%d].thing", i))
		if err != nil {
			return nil, err
		}
		out = append(out, title)
	}
	return append(out, map[string][]part{
		"link value fact":     {words("; " + label + ": "), value},
		"link has fact":       {words("; has " + label)},
		"link not shown fact": {words("; has " + wordNotShown)},
	}[s.name]...), nil
}

// condition is a condition a rule checked, standing in in and at at, as its
// data says it.
func (b rebuilder) condition(in string, c map[string]any, at string) ([]part, error) {
	s, err := b.shape(in, c)
	if err != nil {
		return nil, err
	}
	test, label, link := text(c["test"]), text(c["label"]), text(c["link_label"])
	if plus, ok := c["plus"]; ok {
		label += wordPlus + text(plus)
	}
	value := part{kind: conditionValue, text: text(c["value"]), at: at + ".value", of: c}
	switch s.name {
	case "any or all":
		var out []part
		for i, sub := range list(c["of"]) {
			if i > 0 {
				out = append(out, words(map[string]string{"any_of": " or ", "all_of": " and "}[test]))
			}
			parts, err := b.condition("condition", sub, fmt.Sprintf("%s.of[%d]", at, i))
			if err != nil {
				return nil, err
			}
			out = append(out, parts...)
		}
		return out, nil
	case "link one with":
		parts, err := b.condition("with", object(c["with"]), at+".with")
		return append([]part{words(link + wordOneWith)}, parts...), err
	}
	return map[string][]part{
		"kind":           {words(label)},
		"link":           {words(link)},
		"has":            {words("has " + label)},
		"value":          {words(label + testWords[test]), value},
		"time":           {words(label + testWords[test])},
		"not shown":      {words(wordNotShown)},
		"link has":       {words(link + "; has " + label)},
		"link value":     {words(link + "; " + label + testWords[test]), value},
		"link time":      {words(link + "; " + label + testWords[test])},
		"link not shown": {words(link + "; " + wordNotShown)},
		"with has":       {words(label)},
		"with value":     {words(label + testWords[test]), value},
		"with not shown": {words(wordNotShown)},
	}[s.name], nil
}

// ruledOut is a ruled-out entry's line as its data says it.
func (b rebuilder) ruledOut(r map[string]any) ([]part, error) {
	if _, err := b.shape("ruled out", r); err != nil {
		return nil, err
	}
	out := []part{words(wordAndNo + wordAMatch)}
	if c, ok := r["claim"]; ok {
		parts, err := b.condition("condition", object(c), ".claim")
		if err != nil {
			return nil, err
		}
		out = append([]part{words(wordAndNo)}, parts...)
	}
	for i, thing := range list(r["for"]) {
		out = append(out, words(map[bool]string{true: wordFor, false: ", "}[i == 0]))
		title, err := b.thing(thing, fmt.Sprintf(".for[%d]", i))
		if err != nil {
			return nil, err
		}
		out = append(out, title)
	}
	if len(list(r["for"])) > 0 {
		out = append(out, words(")"))
	}
	return out, nil
}

// values are the values in a node's data that is of no shape, at any depth,
// each to be judged: the value of a fact's claim as a fact's value, every
// other as a condition's constant.
func values(node map[string]any, fact bool) []part {
	var out []part
	var walk func(v any, at string)
	walk = func(v any, at string) {
		switch t := v.(type) {
		case []any:
			for i, sub := range t {
				walk(sub, fmt.Sprintf("%s[%d]", at, i))
			}
		case map[string]any:
			for _, k := range sortedIDs(t) {
				if k == "value" {
					kind := conditionValue
					if fact && at == ".claim" {
						kind = factValue
					}
					out = append(out, part{kind: kind, text: fmt.Sprint(t[k]), at: at + ".value", of: t})
				} else {
					walk(t[k], at+"."+k)
				}
			}
		}
	}
	walk(node["claim"], ".claim")
	return out
}

// claimThings are the things a claim names, at any depth.
func claimThings(c map[string]any) []map[string]any {
	if c == nil {
		return nil
	}
	var out []map[string]any
	if thing := object(c["thing"]); thing != nil {
		out = append(out, thing)
	}
	for _, e := range list(c["ends"]) {
		out = append(out, object(e["thing"]))
	}
	out = append(out, claimThings(object(c["with"]))...)
	for _, part := range list(c["of"]) {
		out = append(out, claimThings(part)...)
	}
	return out
}

// proofsOf calls each on every proof a page gives, at its root.
func proofsOf(page map[string]any, each func(where string, proof map[string]any)) {
	for _, a := range list(page["also"]) {
		each("also "+text(a["label"]), object(a["proof"]))
	}
	for _, g := range list(page["links"]) {
		for i, row := range list(g["rows"]) {
			if proof := object(row["proof"]); proof != nil {
				each(fmt.Sprintf("%s row %d", text(g["label"]), i), proof)
			}
		}
	}
}

// shownValue reports whether page shows value for attribute: of the thing
// itself, or, with other, of its link of type link to other.
func shownValue(page map[string]any, link, other, attribute, value string) bool {
	has := func(values []map[string]any) bool {
		for _, v := range values {
			if v["attribute"] == attribute && slices.Contains(v["values"].([]any), any(value)) {
				return true
			}
		}
		return false
	}
	if link == "" {
		return has(list(page["values"]))
	}
	for _, g := range list(page["links"]) {
		if g["link"] != link {
			continue
		}
		for _, row := range list(g["rows"]) {
			for _, e := range list(row["ends"]) {
				if object(e["thing"])["id"] == other && has(list(row["attributes"])) {
					return true
				}
			}
		}
	}
	return false
}

// factShown reports whether the value a fact's claim gives is what the page
// of its subject shows for its attribute, or, of a link, what that thing's
// row of the link to its other end shows.
func factShown(pages map[string]map[string]any, c map[string]any) bool {
	thing, link, other := subject(c)
	page := pages[text(thing["id"])]
	return page != nil && shownValue(page, link, other, text(c["attribute"]), text(c["value"]))
}

// kindOf is the kind of fact a claim says, for the counts.
func kindOf(c map[string]any) string {
	kind := "a thing's"
	if c["link"] != nil {
		kind = "a link's"
	}
	switch {
	case c["hidden"] == true:
		return kind + " value not shown"
	case c["test"] == "link":
		return "a link"
	case c["test"] == "type":
		return "a thing's kind"
	}
	return kind + " " + text(c["test"])
}

// Every proof node of every page, on the sample store and on the shop and
// the lending library, gives as data each thing its line names, with the
// id, type and title that thing's own page gives, and the value it gives,
// equal to what the page shows for that attribute; and its line is what
// the fixed words make of that data and nothing else. Mutations: leave the
// thing out of a value line's data, or build a line with a word the data
// does not give, and the line is not its data's.
func TestWhyLinesGiveTheirParts(t *testing.T) {
	for _, s := range []struct {
		name string
		open func(*testing.T) *interpreter.Interpreter
	}{
		{"files on memory", filesOnMemory},
		{"files on sqlite", func(t *testing.T) *interpreter.Interpreter { return openSQLite(t, filesOnSQLite(t)) }},
		{"shop on memory", shopOnMemory},
		{"lending on memory", lendingOnMemory},
		{"lending on sqlite", lendingOnSQLite},
	} {
		t.Run(s.name, func(t *testing.T) {
			interp := s.open(t)
			r := show.New(interp, false)
			b := rebuilder{box: interp.TBox()}
			pages := map[string]map[string]any{}
			for _, id := range sortedIDs(holds(t, interp).things) {
				pages[id] = doc(t, r, id)
			}
			counts := map[string]int{}
			fail := func(where, format string, args ...any) {
				t.Helper()
				t.Errorf("%s: %s", where, fmt.Sprintf(format, args...))
			}
			named := func(where string, things []map[string]any) {
				for _, thing := range things {
					page := pages[text(thing["id"])]
					if page == nil || page["type"] != thing["type"] || page["title"] != thing["title"] {
						fail(where, "names %v, whose page is %v %v", thing, page["type"], page["title"])
					}
				}
			}
			var node func(where string, p map[string]any)
			node = func(where string, p map[string]any) {
				where += " > " + text(p["fact"])
				c := object(p["claim"])
				if c == nil {
					fail(where, "no data")
				} else {
					counts[kindOf(c)]++
					parts, err := b.fact(p)
					if line := joined(parts); err != nil || line != p["fact"] {
						fail(where, "the data %v makes %q (%v)", c, line, err)
					}
					named(where, claimThings(c))
					if value := text(c["value"]); value != "" && !factShown(pages, c) {
						fail(where, "gives %s %q, which its page does not show", c["attribute"], value)
					}
				}
				for _, ruled := range list(p["ruled_out"]) {
					counts["ruled out"]++
					if c := object(ruled["claim"]); c == nil && len(list(ruled["for"])) == 0 {
						fail(where, "%q has no data", ruled["line"])
					}
					if parts, err := b.ruledOut(ruled); err != nil || joined(parts) != ruled["line"] {
						fail(where, "%q: its data makes %q (%v)", ruled["line"], joined(parts), err)
					}
					named(where+" > "+text(ruled["line"]), slices.Concat(claimThings(object(ruled["claim"])), list(ruled["for"])))
				}
				for _, sub := range list(p["because"]) {
					node(where, sub)
				}
			}
			for _, id := range sortedIDs(pages) {
				proofsOf(pages[id], func(where string, proof map[string]any) { node(id+" "+where, proof) })
			}
			if len(counts) == 0 {
				t.Fatal("no proof node to check")
			}
			t.Logf("nodes checked, by kind of line: %v", counts)
		})
	}
}
