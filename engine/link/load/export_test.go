package load

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/sanket2904/sifs/engine/extract"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/schema"
	"github.com/sanket2904/ontology-framework/store/sqlite"
)

// FillAtBase fills the SQLite store in dataDir with run the way sifs-load
// did at 6c6f1d6, for the tests outside the package.
func FillAtBase(t *testing.T, dataDir string, run *extract.Output) {
	t.Helper()
	fillSQLite(t, dataDir, baseModule(), run, func(doc extract.DocOut) interpreter.RecordFacts { return asBase(Facts(doc, run.Organisations)) })
}

// FillBeforeFiles fills the SQLite store in dataDir with run the way
// sifs-load did at 2afc7bb, before a document had a file name.
func FillBeforeFiles(t *testing.T, dataDir string, run *extract.Output) {
	t.Helper()
	fillSQLite(t, dataDir, moduleAt2afc7bb(), run, beforeFiles(run))
}

func fillSQLite(t *testing.T, dataDir string, module *schema.ModuleDef, run *extract.Output, base func(extract.DocOut) interpreter.RecordFacts) {
	t.Helper()
	ctx := context.Background()
	st, err := sqlite.Open(filepath.Join(dataDir, StoreFile))
	if err != nil {
		t.Fatal(err)
	}
	schemas, err := st.SchemaStore(ctx)
	if err != nil {
		t.Fatal(err)
	}
	fillAt(t, ctx, schemas, st, module, run, base)
	if err := st.Close(); err != nil {
		t.Fatal(err)
	}
}
