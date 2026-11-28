package load

import (
	"context"
	"maps"
	"slices"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/show"
)

// pathOf puts a document's folder and file name back together with one "/"
// between them. The folder of a file directly under the root is "/", which
// is that "/" already.
func pathOf(folder, name string) string {
	if strings.HasSuffix(folder, "/") {
		return folder + name
	}
	return folder + "/" + name
}

// A run of made-up files whose file is the full path the app's index holds:
// folders with a space, an accent (decomposed and composed), a "#", a "%"
// and quotes, a deep folder, a file directly under a volume and one directly
// under the root, a name holding a backslash, and two files of one name in
// two folders. Each loads, is titled by its file name with its folder as
// subtitle, is found by its file name, and its folder and file name lead
// back to its path byte for byte. Mutation: drop the folder in Facts, and
// the subtitles are empty.
func TestDocumentsWithFullPaths(t *testing.T) {
	cases := []struct{ id, file, title, subtitle string }{
		{"return", "/Users/ana/Tax Papers/2031/return.pdf", "return.pdf", "/Users/ana/Tax Papers/2031"},
		{"lease-decomposed", "/Users/ana/Genéve/bail.pdf", "bail.pdf", "/Users/ana/Genéve"},
		{"lease-composed", "/Users/ana/Genève/bail.pdf", "bail.pdf", "/Users/ana/Genève"},
		{"till", "/Users/ana/Receipts #12/till.txt", "till.txt", "/Users/ana/Receipts #12"},
		{"notes", "/Users/ana/100% done/notes.txt", "notes.txt", "/Users/ana/100% done"},
		{"list", "/Users/ana/Ana's \"best\" files/list.txt", "list.txt", "/Users/ana/Ana's \"best\" files"},
		{"deep", "/Users/ana/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/deep.txt", "deep.txt", "/Users/ana/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p"},
		{"scan", "/Volumes/Backup/scan 2031.pdf", "scan 2031.pdf", "/Volumes/Backup"},
		{"backslash", "/Users/ana/Inbox/back\\slash.txt", "back\\slash.txt", "/Users/ana/Inbox"},
		{"top", "/top.txt", "top.txt", "/"},
		{"statement-banks", "/Users/ana/Banks/statement.pdf", "statement.pdf", "/Users/ana/Banks"},
		{"statement-old-banks", "/Users/ana/Old Banks/statement.pdf", "statement.pdf", "/Users/ana/Old Banks"},
	}
	run := &extract.Output{Format: 3, Organisations: map[string]extract.Card{}}
	for _, c := range cases {
		run.Documents = append(run.Documents, extract.DocOut{ID: c.id, File: c.file})
	}
	stores(t, func(t *testing.T, ctx context.Context, reopen func() *Engine) {
		eq(t, "turned down", mustLoad(t, ctx, reopen(), run).TurnedDown, []TurnDown{})
		r := show.New(reopen().interp, false)
		for _, c := range cases {
			p := page(t, ctx, r, DocumentPrefix+c.id)
			eq(t, c.id+" title", p.Title, c.title)
			eq(t, c.id+" subtitle", p.Subtitle, c.subtitle)
			if got := pathOf(p.Subtitle, p.Title); got != c.file {
				t.Errorf("%s: folder %q and file name %q lead to %q, want %q", c.id, p.Subtitle, p.Title, got, c.file)
			}
			if got := foundDocuments(t, ctx, r, c.title)[DocumentPrefix+c.id]; got != c.title {
				t.Errorf("%q finds %s by %q, want by its file name", c.title, c.id, got)
			}
		}
		same := foundDocuments(t, ctx, r, "statement.pdf")
		eq(t, "statement.pdf finds", slices.Sorted(maps.Keys(same)), []string{DocumentPrefix + "statement-banks", DocumentPrefix + "statement-old-banks"})
	})
}
