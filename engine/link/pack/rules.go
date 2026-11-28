package pack

import "github.com/sanket2904/ontology-framework/schema"

// Derived links: the loader writes none of them; each comes from a rule
// below.
const (
	LatestStatement         = module + ".LatestStatement"
	ProbablyLatestStatement = module + ".ProbablyLatestStatement"
)

func rules(m *schema.ModuleDef) {}
