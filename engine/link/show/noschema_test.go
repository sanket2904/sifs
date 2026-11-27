package show_test

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"

	"github.com/sanket2904/sifs/engine/link/pack"
)

// TestKnowsNoType fails when the reader's source names the files pack or
// any type, link, role, attribute or collection it declares. Mutation: name
// files.Identifier in show.go, and it fails on that line.
func TestKnowsNoType(t *testing.T) {
	box, err := pack.Compile()
	if err != nil {
		t.Fatal(err)
	}
	var ids []string
	for id := range box.EntityTypes {
		ids = append(ids, id)
	}
	for id := range box.RelationTypes {
		ids = append(ids, id)
	}
	for id := range box.RoleTypes {
		ids = append(ids, id)
	}
	for id := range box.AttributeTypes {
		ids = append(ids, id)
	}
	var names []*regexp.Regexp
	for _, id := range ids {
		names = append(names, regexp.MustCompile(regexp.QuoteMeta(id)))
		short := id[strings.LastIndexAny(id, ".:")+1:]
		// A short name is a word of English too ("page", "value"), so it
		// counts only as a string.
		names = append(names, regexp.MustCompile(`"`+regexp.QuoteMeta(short)+`"`))
		if short[0] >= 'A' && short[0] <= 'Z' {
			names = append(names, regexp.MustCompile(`\b`+regexp.QuoteMeta(short)+`\b`))
		}
	}
	for _, et := range box.EntityTypes {
		if et.Display != nil && et.Display.Collection != "" {
			names = append(names, regexp.MustCompile(`"`+string(et.Display.Collection)+`"`))
		}
	}
	names = append(names,
		regexp.MustCompile(`engine/link/pack`), regexp.MustCompile(`engine/link/load`),
		regexp.MustCompile(`"files`), regexp.MustCompile(`Collection(Documents|People|Organisations|Accounts)`))

	files, err := filepath.Glob("*.go")
	if err != nil {
		t.Fatal(err)
	}
	read := 0
	for _, file := range files {
		if strings.HasSuffix(file, "_test.go") {
			continue
		}
		src, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		read++
		for _, name := range names {
			if loc := name.FindIndex(src); loc != nil {
				line := strings.Count(string(src[:loc[0]]), "\n") + 1
				t.Errorf("%s:%d names %s of the files schema", file, line, name)
			}
		}
	}
	if read == 0 {
		t.Fatal("no source read")
	}
}
