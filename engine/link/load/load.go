// Package load puts a format 3 run into the engine: each document is one
// source record, replaced whole, and a document the run no longer holds is
// removed with everything only it gave. It stores what format 3 says and
// decides nothing: no key is joined to another and no tier is raised.
package load

import (
	"context"
	"fmt"
	"maps"
	"os"
	"path/filepath"
	"strings"

	"github.com/sanket2904/sifs/engine/extract"
	"github.com/sanket2904/sifs/engine/link/pack"

	"github.com/sanket2904/ontology-framework/interpreter"
	"github.com/sanket2904/ontology-framework/ontology"
	"github.com/sanket2904/ontology-framework/store"
	"github.com/sanket2904/ontology-framework/store/sqlite"
)

// StoreFile is the engine's store inside a data dir.
const StoreFile = "engine.db"

// Source names the records the loader writes; a record's id is its
// document's id.
const (
	SourceSystem   = "sifs-files"
	SourceInstance = "local"
)

// Engine is the engine opened with the files pack.
type Engine struct {
	interp *interpreter.Interpreter
	close  func() error
}

// Open opens, creating if needed, the SQLite store in dataDir, with the
// files pack as its schema.
func Open(ctx context.Context, dataDir string) (*Engine, error) {
	if dataDir == "" {
		return nil, fmt.Errorf("no data dir")
	}
	if err := os.MkdirAll(dataDir, 0o755); err != nil {
		return nil, err
	}
	return openSQLite(ctx, filepath.Join(dataDir, StoreFile), true)
}

// OpenExisting opens the store in dataDir to read it: it fails when there
// is none and leaves its schema as it is.
func OpenExisting(ctx context.Context, dataDir string) (*Engine, error) {
	path := filepath.Join(dataDir, StoreFile)
	if _, err := os.Stat(path); err != nil {
		return nil, fmt.Errorf("no store: %w", err)
	}
	return openSQLite(ctx, path, false)
}

func openSQLite(ctx context.Context, path string, publish bool) (*Engine, error) {
	st, err := sqlite.Open(path)
	if err != nil {
		return nil, err
	}
	schemas, err := st.SchemaStore(ctx)
	if err == nil {
		var e *Engine
		if e, err = open(ctx, schemas, st, st.Close, publish); err == nil {
			return e, nil
		}
	}
	st.Close()
	return nil, err
}

// OpenMemory opens the engine with the files pack on its memory store.
func OpenMemory(ctx context.Context) (*Engine, error) {
	return open(ctx, store.NewMemorySchemaStore(), store.NewMemoryStore(), func() error { return nil }, true)
}

func open(ctx context.Context, schemas store.SchemaStore, objects store.Store, closer func() error, publish bool) (*Engine, error) {
	// The files are the person's own and the tool runs as them.
	interp, err := interpreter.New(ctx, schemas, objects, interpreter.WithAnonymousWrites(), interpreter.WithUnverifiedSourceClaims())
	if err != nil || !publish {
		return &Engine{interp: interp, close: closer}, err
	}
	box, err := pack.Compile()
	if err != nil {
		return nil, err
	}
	stored, version, err := interp.SchemaSnapshot(ctx)
	if err != nil {
		return nil, err
	}
	// A store filled under an earlier pack may hold values this one types
	// or names otherwise. Load replaces every document's record whole, so
	// none of them outlives the next load.
	if err := interp.PublishSchema(interpreter.AllowBreakingSchemaChange(ctx, "the files pack of this sifs-load; the next load replaces every record"), box, version); err != nil {
		return nil, fmt.Errorf("publish the files pack: %w", err)
	}
	// A store filled under a pack that declared other names is looked up
	// by the old ones until it is indexed again.
	if !maps.Equal(names(stored), names(box)) {
		if err := interp.RebuildNameIndex(ctx); err != nil {
			return nil, fmt.Errorf("index the names of the files pack: %w", err)
		}
	}
	return &Engine{interp: interp, close: closer}, nil
}

// names is the set of attributes a schema declares as names.
func names(box *ontology.TBox) map[string]bool {
	out := map[string]bool{}
	if box == nil {
		return out
	}
	for id, at := range box.AttributeTypes {
		if at != nil && at.Alias {
			out[id] = true
		}
	}
	return out
}

// Close closes the store.
func (e *Engine) Close() error { return e.close() }

// Report is what a load did, by document, and the rows of the run it
// left out.
type Report struct {
	Written    int        `json:"written"`
	Unchanged  int        `json:"unchanged"`
	Removed    int        `json:"removed"`
	TurnedDown []TurnDown `json:"turned_down"`
	LeftOut    LeftOut    `json:"left_out"`
}

// LeftOut counts, by kind, the rows of a run the loader does not store. It
// leaves them out by design, so they do not make a load fail; it counts
// them so that no one takes the run as loaded whole. An organisation card
// that no loaded row uses has nothing to stand on; a mention whose org is
// null names nothing to link to; the answers are judgements over many
// documents, which are not stored.
type LeftOut struct {
	UnusedOrganisations         int `json:"unused_organisations"`
	MentionsWithoutOrganisation int `json:"mentions_without_organisation"`
	Answers                     int `json:"answers"`
}

// TurnDown is a record the engine did not take, with the engine's message.
type TurnDown struct {
	Document string `json:"document"`
	Message  string `json:"message"`
}

func record(docID string) interpreter.SourceRecord {
	return interpreter.SourceRecord{SourceSystem: SourceSystem, SourceInstanceID: SourceInstance, RecordID: docID}
}

// Load makes the store hold exactly the run's documents. A record the
// engine turns down is reported and the others still load.
func (e *Engine) Load(ctx context.Context, run *extract.Output) (Report, error) {
	rep := Report{TurnedDown: []TurnDown{}}
	rep.LeftOut.Answers = len(run.Answers.LatestStatement) + len(run.Answers.CurrentPassport)
	usedOrgs := map[string]bool{}
	inRun := map[string]bool{}
	for _, doc := range run.Documents {
		inRun[doc.ID] = true
		facts := Facts(doc, run.Organisations)
		rep.LeftOut.MentionsWithoutOrganisation += len(doc.Orgs) - countType(facts.Relations, pack.Mentions)
		for _, thing := range facts.Entities {
			if thing.TypeID == pack.Organisation {
				usedOrgs[thing.Attributes[pack.Key].(string)] = true
			}
		}
		res, err := e.interp.ReplaceRecord(ctx, record(doc.ID), facts)
		switch {
		case !rep.taken(doc.ID, res, err):
		case res.Record.Unchanged():
			rep.Unchanged++
		default:
			rep.Written++
		}
	}
	for key := range run.Organisations {
		if !usedOrgs[key] {
			rep.LeftOut.UnusedOrganisations++
		}
	}
	stored, err := e.Documents(ctx)
	if err != nil {
		return rep, err
	}
	for _, id := range stored {
		if inRun[id] {
			continue
		}
		if res, err := e.interp.RemoveRecord(ctx, record(id)); rep.taken(id, res, err) {
			rep.Removed++
		}
	}
	return rep, nil
}

func countType(relations []ontology.ObservedRelation, typeID string) int {
	n := 0
	for _, r := range relations {
		if r.TypeID == typeID {
			n++
		}
	}
	return n
}

// taken reports whether the engine took a record write, and keeps its
// turn-down when it did not.
func (rep *Report) taken(docID string, res *interpreter.ObserveResult, err error) bool {
	switch {
	case err != nil:
		rep.TurnedDown = append(rep.TurnedDown, TurnDown{Document: docID, Message: err.Error()})
		return false
	case res == nil || res.Record == nil:
		rep.TurnedDown = append(rep.TurnedDown, TurnDown{Document: docID, Message: "the engine gave no record result"})
		return false
	}
	return true
}

// Documents lists the ids of the documents the store holds.
func (e *Engine) Documents(ctx context.Context) ([]string, error) {
	things, err := e.interp.QueryEntities(ctx, store.EntityPattern{TypeID: pack.Document})
	if err != nil {
		return nil, err
	}
	ids := make([]string, 0, len(things))
	for _, t := range things {
		ids = append(ids, strings.TrimPrefix(t.ID, DocumentPrefix))
	}
	return ids, nil
}

// Things reads every thing of one type the store holds, and Links every
// link of one type. They only read: nothing reachable through them writes
// to the store or publishes a schema.
func (e *Engine) Things(ctx context.Context, typeID string) ([]*ontology.Entity, error) {
	return e.interp.QueryEntities(ctx, store.EntityPattern{TypeID: typeID})
}

func (e *Engine) Links(ctx context.Context, typeID string) ([]*ontology.Relation, error) {
	return e.interp.QueryRelations(ctx, store.RelationPattern{TypeID: typeID})
}

// Counts is how many things and links of each kind the store holds, by the
// pack's type names without the module.
type Counts struct {
	Things map[string]int `json:"things"`
	Links  map[string]int `json:"links"`
}

// Count reads the store's counts.
func (e *Engine) Count(ctx context.Context) (Counts, error) {
	c := Counts{Things: map[string]int{}, Links: map[string]int{}}
	for _, t := range pack.Things {
		found, err := e.interp.QueryEntities(ctx, store.EntityPattern{TypeID: t})
		if err != nil {
			return c, err
		}
		c.Things[short(t)] = len(found)
	}
	for _, t := range pack.Links {
		found, err := e.interp.QueryRelations(ctx, store.RelationPattern{TypeID: t})
		if err != nil {
			return c, err
		}
		c.Links[short(t)] = len(found)
	}
	return c, nil
}

func short(typeID string) string { return typeID[strings.IndexByte(typeID, '.')+1:] }
