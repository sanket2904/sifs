package show

import (
	"fmt"
	"slices"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/sanket2904/ontology-framework/ontology"
)

// shown decides, for the whole reader, what of a value a person is given: a
// value of a hidden attribute is never given, a value of a masked attribute
// keeps its last four characters unless the reader reveals, and a time of
// an attribute shown as a date is its day in UTC. Every value, title,
// subtitle and matched name the reader returns passes through it.
type shown struct {
	box    *ontology.TBox
	reveal bool
}

func (s shown) display(attrID string) *ontology.AttributeDisplay {
	if at := s.box.GetAttributeType(attrID); at != nil && at.Display != nil {
		return at.Display
	}
	return &ontology.AttributeDisplay{}
}

func (s shown) hidden(attrID string) bool { return s.display(attrID).Hidden }

// value is one value of an attribute as a person reads it.
func (s shown) value(attrID string, v any) string {
	d := s.display(attrID)
	text := plain(v, d.Kind)
	if d.Mask && !s.reveal {
		return mask(text)
	}
	return text
}

// values are an attribute's values in a stable order: the memory store and
// SQLite give a repeated value's values in different orders.
func (s shown) values(attrID string, vs []any) []string {
	out := make([]string, 0, len(vs))
	for _, v := range vs {
		out = append(out, s.value(attrID, v))
	}
	slices.Sort(out)
	return slices.Compact(out)
}

// first is the first of attrIDs with a value that is not hidden, as shown.
func (s shown) first(e *ontology.Entity, attrIDs []string) string {
	for _, attrID := range attrIDs {
		if s.hidden(attrID) {
			continue
		}
		if vs := s.values(attrID, e.Attributes[attrID]); len(vs) > 0 {
			return vs[0]
		}
	}
	return ""
}

func mask(text string) string {
	n := utf8.RuneCountInString(text)
	if n <= 4 {
		return strings.Repeat("•", n)
	}
	runes := []rune(text)
	return strings.Repeat("•", n-4) + string(runes[n-4:])
}

func plain(v any, kind ontology.DisplayKind) string {
	switch t := v.(type) {
	case string:
		return t
	case time.Time:
		if kind == ontology.DisplayDate {
			return t.UTC().Format(time.DateOnly)
		}
		return t.UTC().Format(time.RFC3339)
	case float64:
		if t == float64(int64(t)) {
			return fmt.Sprint(int64(t))
		}
	}
	return fmt.Sprint(v)
}
