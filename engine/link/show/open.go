package show

import (
	"context"
	"fmt"
	"os"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/store/sqlite"
)

// Open opens the engine's SQLite store at path to read it with the schema
// it holds. It fails when there is no store and publishes nothing.
func Open(ctx context.Context, path string) (*interpreter.Interpreter, func() error, error) {
	if _, err := os.Stat(path); err != nil {
		return nil, nil, fmt.Errorf("no store: %w", err)
	}
	st, err := sqlite.Open(path)
	if err != nil {
		return nil, nil, err
	}
	schemas, err := st.SchemaStore(ctx)
	if err != nil {
		st.Close()
		return nil, nil, err
	}
	interp, err := interpreter.New(ctx, schemas, st)
	if err != nil {
		st.Close()
		return nil, nil, err
	}
	return interp, st.Close, nil
}
