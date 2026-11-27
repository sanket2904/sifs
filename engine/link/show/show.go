// Package show shows what the engine knows, for any schema: the kinds of
// thing a store holds, the things a typed word finds, and the page of one
// thing. It knows no type. Every label, title and grouping it gives comes
// from the schema the store holds, and every thing and fact from the
// engine's own calls. It reads and writes nothing.
package show

import (
	"cmp"
	"context"
	"errors"
	"fmt"
	"slices"
	"strings"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/store"
)

// Reader reads one engine store.
type Reader struct {
	interp *interpreter.Interpreter
	box    *ontology.TBox
	shown  shown
}

// New reads through interp with the schema it holds. With reveal, masked
// values are given raw.
func New(interp *interpreter.Interpreter, reveal bool) *Reader {
	box := interp.TBox()
	return &Reader{interp: interp, box: box, shown: shown{box: box, reveal: reveal}}
}

// ErrNoThing is the error for a page of an id the store does not hold.
var ErrNoThing = errors.New("no such thing")

// Thing names one thing: its id, its type, and its title and subtitle as
// its type declares them. A thing with no title of its own is titled by
// its id.
type Thing struct {
	ID       string `json:"id"`
	Type     string `json:"type"`
	Title    string `json:"title"`
	Subtitle string `json:"subtitle,omitempty"`
}

func (r *Reader) thing(e *ontology.Entity) Thing {
	t := Thing{ID: e.ID, Type: e.TypeID}
	if d := r.typeDisplay(e.TypeID); d != nil {
		t.Title = r.shown.first(e, d.Title)
		t.Subtitle = r.shown.first(e, d.Subtitle)
	}
	if t.Title == "" {
		t.Title = t.ID
	}
	return t
}

// typeDisplay is the display a type declares, or the one its nearest
// supertype declares.
func (r *Reader) typeDisplay(typeID string) *ontology.TypeDisplay {
	seen := map[string]bool{}
	for et := r.box.GetEntityType(typeID); et != nil && !seen[et.ID]; et = r.box.GetEntityType(et.SuperTypeID) {
		if et.Display != nil {
			return et.Display
		}
		seen[et.ID] = true
	}
	return nil
}

// collection is the collection a type declares, or "things".
func (r *Reader) collection(typeID string) string {
	if d := r.typeDisplay(typeID); d != nil && d.Collection != "" {
		return string(d.Collection)
	}
	return string(ontology.CollectionThings)
}

// label is the label the schema declares for a type, a link, a role or an
// attribute, or, without one, its id after the module.
func (r *Reader) label(id string) string {
	var label string
	switch {
	case r.box.GetEntityType(id) != nil:
		label = r.box.GetEntityType(id).Label
	case r.box.GetRelationType(id) != nil:
		label = r.box.GetRelationType(id).Label
	case r.box.GetRoleType(id) != nil:
		label = r.box.GetRoleType(id).Label
	case r.box.GetAttributeType(id) != nil:
		label = r.box.GetAttributeType(id).Label
	}
	if label != "" {
		return label
	}
	return id[strings.LastIndexAny(id, ".:")+1:]
}

// Kinds are the kinds of thing the schema declares and how many of each the
// store holds, by collection.
type Kinds struct {
	Kinds []Kind `json:"kinds"`
}

type Kind struct {
	Type       string `json:"type"`
	Label      string `json:"label"`
	Collection string `json:"collection"`
	Count      int    `json:"count"`
}

// Kinds lists every type of thing the schema declares that can be made.
func (r *Reader) Kinds(ctx context.Context) (Kinds, error) {
	out := Kinds{Kinds: []Kind{}}
	for _, id := range sortedKeys(r.box.EntityTypes) {
		if r.box.EntityTypes[id].IsAbstract {
			continue
		}
		things, err := r.interp.QueryEntities(ctx, store.EntityPattern{TypeID: id})
		if err != nil {
			return out, err
		}
		out.Kinds = append(out.Kinds, Kind{Type: id, Label: r.label(id), Collection: r.collection(id), Count: len(things)})
	}
	slices.SortFunc(out.Kinds, func(a, b Kind) int {
		return cmp.Or(cmp.Compare(a.Collection, b.Collection), cmp.Compare(a.Label, b.Label), cmp.Compare(a.Type, b.Type))
	})
	return out, nil
}

// Found is what a typed word or phrase finds, grouped by collection and
// type.
type Found struct {
	Groups []Group `json:"groups"`
}

type Group struct {
	Collection string  `json:"collection"`
	Type       string  `json:"type"`
	Label      string  `json:"label"`
	Things     []Match `json:"things"`
}

// Match is one thing found and the name of it that matched.
type Match struct {
	Thing
	Matched Name `json:"matched"`
}

type Name struct {
	Attribute string `json:"attribute"`
	Label     string `json:"label"`
	Value     string `json:"value,omitempty"`
	Exact     bool   `json:"exact"`
}

// findLimit caps how many things one find gives.
const findLimit = 200

// Find gives the things the engine's name lookup returns for words. Blank
// words find nothing.
func (r *Reader) Find(ctx context.Context, words string) (Found, error) {
	out := Found{Groups: []Group{}}
	if strings.TrimSpace(words) == "" {
		return out, nil
	}
	found, err := r.interp.Resolve(ctx, words, interpreter.ResolveOptions{Limit: findLimit})
	if err != nil {
		return out, err
	}
	groups := map[string]*Group{}
	for _, c := range found {
		typeID := c.Entity.TypeID
		g := groups[typeID]
		if g == nil {
			g = &Group{Collection: r.collection(typeID), Type: typeID, Label: r.label(typeID)}
			groups[typeID] = g
		}
		name := Name{Attribute: c.AttrID, Label: r.label(c.AttrID), Exact: c.Exact}
		if !r.shown.hidden(c.AttrID) {
			name.Value = r.shown.value(c.AttrID, c.Name)
		}
		g.Things = append(g.Things, Match{Thing: r.thing(c.Entity), Matched: name})
	}
	for _, g := range groups {
		slices.SortFunc(g.Things, func(a, b Match) int {
			return cmp.Or(cmp.Compare(a.Title, b.Title), cmp.Compare(a.ID, b.ID), cmp.Compare(a.Subtitle, b.Subtitle),
				cmp.Compare(a.Matched.Label, b.Matched.Label), cmp.Compare(a.Matched.Value, b.Matched.Value))
		})
		out.Groups = append(out.Groups, *g)
	}
	slices.SortFunc(out.Groups, func(a, b Group) int {
		return cmp.Or(cmp.Compare(a.Collection, b.Collection), cmp.Compare(a.Label, b.Label), cmp.Compare(a.Type, b.Type))
	})
	return out, nil
}

func sortedKeys[V any](m map[string]V) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	slices.Sort(keys)
	return keys
}

func errNoThing(id string) error { return fmt.Errorf("%w: %s", ErrNoThing, id) }
