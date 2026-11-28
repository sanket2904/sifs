package show_test

import (
	"fmt"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/interpreter"
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

// factParts is a proof's fact as its claim, at .claim, says it.
func factParts(c map[string]any) ([]part, error) {
	if thing := object(c["thing"]); thing != nil {
		title := part{kind: titleOf, text: text(thing["title"]), at: ".claim.thing.title", of: thing}
		switch {
		case c["hidden"] == true:
			return []part{title, words(wordHas + wordNotShown)}, nil
		case c["test"] == "type":
			return []part{title, words(wordIs + text(c["label"]))}, nil
		case c["test"] == "has":
			return []part{title, words(wordHas + text(c["label"]))}, nil
		case c["test"] == "is":
			return []part{title, words(" " + text(c["label"]) + " "), {kind: factValue, text: text(c["value"]), at: ".claim.value", of: c}}, nil
		}
		return nil, fmt.Errorf("a claim about a thing that tests %v", c["test"])
	}
	ends := list(c["ends"])
	if c["link"] == nil || len(ends) == 0 {
		return nil, fmt.Errorf("a claim that names no thing: %v", c)
	}
	out := []part{words(text(c["link_label"]) + ": ")}
	for i, e := range ends {
		if i > 0 {
			out = append(out, words(", "))
		}
		if role := text(e["label"]); role != "" {
			out = append(out, words(role+" "))
		}
		thing := object(e["thing"])
		out = append(out, part{kind: titleOf, text: text(thing["title"]), at: fmt.Sprintf(".claim.ends[%d].thing.title", i), of: thing})
	}
	switch {
	case c["test"] == "link":
		return out, nil
	case c["hidden"] == true:
		return append(out, words("; has "+wordNotShown)), nil
	case c["test"] == "has":
		return append(out, words("; has "+text(c["label"]))), nil
	case c["test"] == "is":
		return append(out, words("; "+text(c["label"])+": "), part{kind: factValue, text: text(c["value"]), at: ".claim.value", of: c}), nil
	}
	return nil, fmt.Errorf("a link claim that tests %v", c["test"])
}

// conditionParts is a condition a rule checked, at at, as its claim says it.
func conditionParts(c map[string]any, at string) []part {
	switch c["test"] {
	case "type":
		return []part{words(text(c["label"]))}
	case "any_of", "all_of":
		var out []part
		for i, sub := range list(c["of"]) {
			if i > 0 {
				out = append(out, words(map[any]string{"any_of": " or ", "all_of": " and "}[c["test"]]))
			}
			out = append(out, conditionParts(sub, fmt.Sprintf("%s.of[%d]", at, i))...)
		}
		return out
	}
	with := object(c["with"])
	switch {
	case c["link"] == nil:
		return valueParts(c, at)
	case with != nil && with["hidden"] == true:
		return []part{words(text(c["link_label"]) + wordOneWith + wordNotShown)}
	case with != nil && with["test"] == "has":
		return []part{words(text(c["link_label"]) + wordOneWith + text(with["label"]))}
	case with != nil:
		return append([]part{words(text(c["link_label"]) + wordOneWith)}, valueParts(with, at+".with")...)
	case c["attribute"] != nil || c["hidden"] == true:
		return append([]part{words(text(c["link_label"]) + "; ")}, valueParts(c, at)...)
	}
	return []part{words(text(c["link_label"]))}
}

func valueParts(c map[string]any, at string) []part {
	if c["hidden"] == true {
		return []part{words(wordNotShown)}
	}
	if c["test"] == "has" {
		return []part{words("has " + text(c["label"]))}
	}
	label := text(c["label"])
	if plus := text(c["plus"]); plus != "" {
		label += wordPlus + plus
	}
	if c["test"] == "in_the_past" || c["test"] == "in_the_future" {
		return []part{words(label + testWords[text(c["test"])])}
	}
	return []part{words(label + testWords[text(c["test"])]), {kind: conditionValue, text: text(c["value"]), at: at + ".value", of: c}}
}

// ruledOutParts is a ruled-out entry's line as its claim and things say it.
func ruledOutParts(r map[string]any) []part {
	out := []part{words(wordAndNo + wordAMatch)}
	if c := object(r["claim"]); c != nil {
		out = append([]part{words(wordAndNo)}, conditionParts(c, ".claim")...)
	}
	for i, thing := range list(r["for"]) {
		out = append(out, words(map[bool]string{true: wordFor, false: ", "}[i == 0]))
		out = append(out, part{kind: titleOf, text: text(thing["title"]), at: fmt.Sprintf(".for[%d].title", i), of: thing})
	}
	if len(list(r["for"])) > 0 {
		out = append(out, words(")"))
	}
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

// factShown reports whether the value a fact's claim gives is what the
// page of its thing shows for its attribute, or, of a link, what that
// thing's row of the link to its other end shows.
func factShown(pages map[string]map[string]any, c map[string]any) bool {
	subject, link, other := object(c["thing"]), "", ""
	if ends := list(c["ends"]); len(ends) == 2 {
		subject, link, other = object(ends[0]["thing"]), text(c["link"]), text(object(ends[1]["thing"])["id"])
	}
	page := pages[text(subject["id"])]
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
					parts, err := factParts(c)
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
					if line := joined(ruledOutParts(ruled)); line != ruled["line"] {
						fail(where, "%q: its data makes %q", ruled["line"], line)
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
