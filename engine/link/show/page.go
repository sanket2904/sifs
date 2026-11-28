package show

import (
	"cmp"
	"context"
	"fmt"
	"slices"
	"strings"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/store"
)

// Page is everything the engine knows of one thing.
type Page struct {
	Thing
	Label  string      `json:"label"`
	Also   []Also      `json:"also"`
	Values []Value     `json:"values"`
	Links  []LinkGroup `json:"links"`
}

// Value is one attribute of a thing or a link: its values, the status the
// engine gives them and the source records they come from.
type Value struct {
	Attribute string   `json:"attribute"`
	Label     string   `json:"label"`
	Values    []string `json:"values"`
	Status    string   `json:"status,omitempty"`
	Reason    string   `json:"reason,omitempty"`
	Sources   []string `json:"sources,omitempty"`
}

// LinkGroup is the links of one type in which the thing plays one role,
// under the label its role declares.
type LinkGroup struct {
	Link  string `json:"link"`
	Role  string `json:"role"`
	Label string `json:"label"`
	Rows  []Link `json:"rows"`
}

// Link is one link: the things on its other ends, its own attributes and
// the source records it comes from, or, when no source record states it,
// the engine's proof of it.
type Link struct {
	Ends       []End    `json:"ends"`
	Attributes []Value  `json:"attributes"`
	Sources    []string `json:"sources"`
	Proof      *Proof   `json:"proof,omitempty"`
	id         string
}

type End struct {
	Role  string `json:"role"`
	Label string `json:"label"`
	Thing Thing  `json:"thing"`
}

// Also is a type the thing has by derivation, with the engine's proof.
type Also struct {
	Type  string `json:"type"`
	Label string `json:"label"`
	Proof Proof  `json:"proof"`
}

// Page reads the page of the thing with id.
func (r *Reader) Page(ctx context.Context, id string) (Page, error) {
	e, err := r.interp.GetEntity(ctx, id)
	if err != nil {
		return Page{}, err
	}
	if e == nil {
		return Page{}, errNoThing(id)
	}
	p := Page{Thing: r.thing(e), Label: r.label(e.TypeID), Also: []Also{}}
	if p.Values, err = r.values(ctx, id); err != nil {
		return p, err
	}
	if p.Links, err = r.links(ctx, e); err != nil {
		return p, err
	}
	for _, typeID := range sortedKeys(e.DerivedTypes) {
		if typeID == e.TypeID {
			continue
		}
		proof, err := r.why(ctx, interpreter.TypeFact(id, typeID))
		if err != nil {
			return p, err
		}
		p.Also = append(p.Also, Also{Type: typeID, Label: r.label(typeID), Proof: proof})
	}
	return p, nil
}

func (r *Reader) values(ctx context.Context, id string) ([]Value, error) {
	desc, err := r.interp.DescribeObject(ctx, id)
	if err != nil {
		return nil, err
	}
	out := []Value{}
	for _, prop := range desc.Properties {
		attrID := prop.Property.Definition.ID
		if !prop.Present || r.shown.hidden(attrID) {
			continue
		}
		from, err := r.sources(ctx, store.AssertionQuery{IDs: prop.State.AssertionIDs})
		if err != nil {
			return nil, err
		}
		out = append(out, Value{
			Attribute: attrID, Label: r.label(attrID), Values: r.shown.values(attrID, prop.Values),
			Status: string(prop.State.Status), Reason: prop.State.Reason, Sources: from,
		})
	}
	sortValues(out)
	return out, nil
}

func (r *Reader) links(ctx context.Context, e *ontology.Entity) ([]LinkGroup, error) {
	rels, err := r.interp.GetRelationsForPlayer(ctx, e.ID, "")
	if err != nil {
		return nil, err
	}
	groups := map[[2]string]*LinkGroup{}
	for _, rel := range rels {
		for _, role := range rolesOf(rel, e.ID) {
			if d := r.box.GetRoleType(role); d != nil && d.Display != nil && d.Display.Hidden {
				continue
			}
			row, err := r.link(ctx, rel, e.ID, role)
			if err != nil {
				return nil, err
			}
			key := [2]string{rel.TypeID, role}
			g := groups[key]
			if g == nil {
				g = &LinkGroup{Link: rel.TypeID, Role: role, Label: r.roleLabel(rel.TypeID, role)}
				groups[key] = g
			}
			g.Rows = append(g.Rows, row)
		}
	}
	out := []LinkGroup{}
	for _, g := range groups {
		slices.SortFunc(g.Rows, func(a, b Link) int {
			return cmp.Or(cmp.Compare(rowKey(a), rowKey(b)), cmp.Compare(a.id, b.id))
		})
		out = append(out, *g)
	}
	slices.SortFunc(out, func(a, b LinkGroup) int {
		return cmp.Or(cmp.Compare(a.Label, b.Label), cmp.Compare(a.Link, b.Link), cmp.Compare(a.Role, b.Role))
	})
	return out, nil
}

// rolesOf are the roles id plays in rel, once each.
func rolesOf(rel *ontology.Relation, id string) []string {
	var roles []string
	for _, p := range rel.Players {
		if p.PlayerID == id && !slices.Contains(roles, p.RoleTypeID) {
			roles = append(roles, p.RoleTypeID)
		}
	}
	return roles
}

// roleLabel is how a link reads from the thing playing role, as the role
// declares it, or the link's label and the role's.
func (r *Reader) roleLabel(linkID, role string) string {
	if rt := r.box.GetRoleType(role); rt != nil && rt.Display != nil && rt.Display.Label != "" {
		return rt.Display.Label
	}
	return fmt.Sprintf("%s (%s)", r.label(linkID), r.label(role))
}

func (r *Reader) link(ctx context.Context, rel *ontology.Relation, self, role string) (Link, error) {
	row := Link{Ends: []End{}, Attributes: []Value{}, id: rel.ID}
	for _, p := range rel.Players {
		if p.PlayerID == self && p.RoleTypeID == role {
			continue
		}
		end := End{Role: p.RoleTypeID, Label: r.label(p.RoleTypeID)}
		other, err := r.interp.GetEntity(ctx, p.PlayerID)
		if err != nil {
			return row, err
		}
		if other != nil {
			end.Thing = r.thing(other)
		} else {
			end.Thing = Thing{ID: p.PlayerID, Title: p.PlayerID}
		}
		row.Ends = append(row.Ends, end)
	}
	slices.SortFunc(row.Ends, func(a, b End) int {
		return cmp.Or(cmp.Compare(a.Label, b.Label), cmp.Compare(a.Role, b.Role), cmp.Compare(a.Thing.Title, b.Thing.Title))
	})
	for _, attrID := range sortedKeys(rel.Attributes) {
		if r.shown.hidden(attrID) {
			continue
		}
		row.Attributes = append(row.Attributes, Value{Attribute: attrID, Label: r.label(attrID), Values: r.shown.values(attrID, rel.Attributes[attrID])})
	}
	sortValues(row.Attributes)
	var err error
	if row.Sources, err = r.sources(ctx, store.AssertionQuery{RelationID: rel.ID}); err != nil || len(row.Sources) > 0 {
		return row, err
	}
	if fact, ok := r.linkFact(rel); ok {
		proof, err := r.why(ctx, fact)
		row.Proof = &proof
		return row, err
	}
	return row, nil
}

// linkFact names a link of two things as the engine's proofs name it, its
// players in the order its type declares their roles.
func (r *Reader) linkFact(rel *ontology.Relation) (interpreter.ProofFact, bool) {
	rt := r.box.GetRelationType(rel.TypeID)
	if rt == nil || len(rt.Relates) != 2 || len(rel.Players) != 2 {
		return interpreter.ProofFact{}, false
	}
	player := map[string]string{}
	for _, p := range rel.Players {
		player[p.RoleTypeID] = p.PlayerID
	}
	from, to := player[rt.Relates[0].RoleTypeID], player[rt.Relates[1].RoleTypeID]
	return interpreter.LinkFact(rel.TypeID, from, to), from != "" && to != ""
}

func rowKey(l Link) string {
	var b strings.Builder
	for _, e := range l.Ends {
		b.WriteString(e.Label + "\x00" + e.Thing.Title + "\x00")
	}
	for _, a := range l.Attributes {
		b.WriteString(a.Label + "\x00" + strings.Join(a.Values, "\x00") + "\x00")
	}
	return b.String()
}

func sortValues(vs []Value) {
	slices.SortFunc(vs, func(a, b Value) int {
		return cmp.Or(cmp.Compare(a.Label, b.Label), cmp.Compare(a.Attribute, b.Attribute))
	})
}

// sources are the source records of the assertions q finds, once each.
func (r *Reader) sources(ctx context.Context, q store.AssertionQuery) ([]string, error) {
	if len(q.IDs) == 0 && q.RelationID == "" {
		return []string{}, nil
	}
	as, err := r.interp.QueryAssertions(ctx, q)
	if err != nil {
		return nil, err
	}
	out := []string{}
	for _, a := range as {
		out = append(out, a.Metadata.SourceRecordID)
	}
	slices.Sort(out)
	return slices.Compact(out), nil
}
