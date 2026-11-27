// Package card answers questions from the engine's store that the loader
// filled: the organisation card for a typed word or phrase, and when each
// passport expires. It reads the store and writes nothing, adds no rule and
// never raises a tier the store gives.
package card

import (
	"context"
	"slices"
	"strings"

	"github.com/sanket2904/sifs/engine/link/load"
	"github.com/sanket2904/sifs/engine/link/pack"

	"github.com/sanket2904/ontology-framework/ontology"
)

// Tier is a claim's tier as the store gives it: certain or likely.
type Tier string

const (
	Certain Tier = "certain"
	Likely  Tier = "likely"
)

// lower is the lower of two tiers. Anything that is not certain counts as
// likely, so an unknown value can only lower a tier.
func lower(a, b Tier) Tier {
	if a == Certain && b == Certain {
		return Certain
	}
	return Likely
}

func higher(a, b Tier) Tier {
	if a == Certain || b == Certain {
		return Certain
	}
	return Likely
}

// snapshot is what the store holds of the types the answers read, by id.
type snapshot struct {
	things map[string]*ontology.Entity
	links  map[string][]*ontology.Relation
}

func read(ctx context.Context, e *load.Engine, things, links []string) (*snapshot, error) {
	s := &snapshot{things: map[string]*ontology.Entity{}, links: map[string][]*ontology.Relation{}}
	for _, t := range things {
		found, err := e.Things(ctx, t)
		if err != nil {
			return nil, err
		}
		for _, thing := range found {
			s.things[thing.ID] = thing
		}
	}
	for _, t := range links {
		found, err := e.Links(ctx, t)
		if err != nil {
			return nil, err
		}
		s.links[t] = found
	}
	return s, nil
}

// player is the id of the thing that plays a role in a link.
func player(r *ontology.Relation, role string) string {
	for _, p := range r.Players {
		if p.RoleTypeID == r.TypeID+":"+role {
			return p.PlayerID
		}
	}
	return ""
}

func document(r *ontology.Relation, role string) string {
	return strings.TrimPrefix(player(r, role), load.DocumentPrefix)
}

func str(attrs map[string][]any, name string) string {
	if v := attrs[name]; len(v) > 0 {
		if s, ok := v[0].(string); ok {
			return s
		}
	}
	return ""
}

// strs is a repeated value, sorted: the memory store and SQLite give its
// values in different orders.
func strs(attrs map[string][]any, name string) []string {
	out := []string{}
	for _, v := range attrs[name] {
		if s, ok := v.(string); ok {
			out = append(out, s)
		}
	}
	slices.Sort(out)
	return slices.Compact(out)
}

func integer(attrs map[string][]any, name string) (int, bool) {
	v := attrs[name]
	if len(v) == 0 {
		return 0, false
	}
	switch n := v[0].(type) {
	case int64:
		return int(n), true
	case int:
		return n, true
	case float64:
		return int(n), true
	}
	return 0, false
}

func boolean(attrs map[string][]any, name string) *bool {
	if v := attrs[name]; len(v) > 0 {
		if b, ok := v[0].(bool); ok {
			return &b
		}
	}
	return nil
}

func tier(attrs map[string][]any, name string) Tier { return Tier(str(attrs, name)) }

func tierOf(r *ontology.Relation) Tier { return tier(r.Attributes, pack.Tier) }
