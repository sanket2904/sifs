package show_test

import (
	"context"
	"testing"
	"time"

	"github.com/sanket2904/sifs/engine/link/show"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/schema"
)

// libraryOnMemory is a made-up lending library, unrelated to the files
// schema: a loan is due on a day, declared shown as a date and used as its
// subtitle, and was logged at a time, a timestamp with no such
// declaration. A member's two-digit locker is masked, and its digits are
// also digits of the day a loan is due.
func libraryOnMemory(t *testing.T) *interpreter.Interpreter {
	t.Helper()
	m := schema.NewModule("library", "1")
	label := schema.Label
	m.Entity("Loan").Label("Loan").
		Attribute("book", "string", label("Book"), schema.Alias()).
		Attribute("due", "timestamp", label("Due"), schema.Shown(ontology.DisplayDate)).
		Attribute("logged", "timestamp", label("Logged")).
		Attribute("locker", "string", label("Locker"), schema.Masked()).
		Display(schema.Title("book"), schema.Subtitle("due"), schema.Collection(ontology.CollectionDocuments))
	box, err := schema.CompileModules(m)
	if err != nil {
		t.Fatal(err)
	}
	loan := func(id, book string, due, logged time.Time, locker string) interpreter.RecordFacts {
		return interpreter.RecordFacts{Entities: []ontology.ObservedEntity{{ID: id, TypeID: "library.Loan", Attributes: map[string]any{
			"library.book": book, "library.due": due, "library.logged": logged, "library.locker": locker}}}}
	}
	return memoryStore(t, box, map[string]interpreter.RecordFacts{
		"osaka": loan("loan:osaka", "Kokoro", time.Date(2026, 8, 31, 0, 0, 0, 0, time.UTC), time.Date(2026, 8, 30, 23, 30, 0, 0, time.UTC), "08"),
		"nairobi": loan("loan:nairobi", "Petals of Blood", time.Date(2026, 9, 6, 0, 0, 0, 0, time.UTC),
			time.Date(2026, 9, 6, 9, 15, 0, 0, time.FixedZone("EAT", 3*3600)), "06"),
	}, "library")
}

// A time of an attribute declared shown as a date is given as its day in
// UTC, in a value and in a subtitle, wherever the reader runs; a timestamp
// with no such declaration is given whole. Mutations: give every time in
// full in plain, and the due days read T00:00:00Z; give a date's day in
// time.Local, and the days move in UTC-10.
func TestDateShownAsDate(t *testing.T) {
	defer func(local *time.Location) { time.Local = local }(time.Local)
	for _, zone := range []*time.Location{time.FixedZone("UTC-10", -10*3600), time.FixedZone("UTC+14", 14*3600)} {
		time.Local = zone
		t.Run(zone.String(), func(t *testing.T) {
			r := show.New(libraryOnMemory(t), false)
			for id, want := range map[string][2]string{
				"loan:osaka":   {"2026-08-31", "2026-08-30T23:30:00Z"},
				"loan:nairobi": {"2026-09-06", "2026-09-06T06:15:00Z"},
			} {
				p, err := r.Page(context.Background(), id)
				if err != nil {
					t.Fatal(err)
				}
				due, logged := valuesOf(p, "Due"), valuesOf(p, "Logged")
				if p.Subtitle != want[0] || len(due) != 1 || due[0] != want[0] || len(logged) != 1 || logged[0] != want[1] {
					t.Errorf("%s: subtitle %q, due %q, logged %q; want %q, %q, %q", id, p.Subtitle, due, logged, want[0], want[0], want[1])
				}
			}
		})
	}
}

// A day shown by declaration as a subtitle is no leak, although a masked
// locker's two digits are digits of it; the same digits given as a title
// of their own are. Mutation: decide a title by its text alone in leaks, as
// before, and the due days are called leaks of the lockers.
func TestDateSubtitleIsNoLeak(t *testing.T) {
	interp := libraryOnMemory(t)
	h := holds(t, interp)
	reportLeaks(t, h, everything(t, show.New(interp, false), h), false)
	if got := h.leaks(encode(t, show.Thing{ID: "loan:osaka", Type: "library.Loan", Title: "Kokoro", Subtitle: "08"}), false); len(got) != 1 {
		t.Errorf("a locker given as a subtitle: %q", got)
	}
}

func valuesOf(p show.Page, label string) []string {
	for _, v := range p.Values {
		if v.Label == label {
			return v.Values
		}
	}
	return nil
}
