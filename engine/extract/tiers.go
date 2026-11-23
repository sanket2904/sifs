package extract

// Tier says how a person may see a claim (sample/FORMAT.md, "Tiers"): a
// certain claim as a fact, a likely one as "may be", with its reason.
type Tier string

const (
	Certain Tier = "certain"
	Likely  Tier = "likely"
)

// proof is why the extractor concludes a claim. The zero proof is reading:
// layout, headings, nearness or wording, which never makes a claim certain.
type proof uint8

const (
	read proof = iota
	// ibanRegistered: the IBAN's country is in iban-registry.json, it has
	// that country's length, and its check digits pass.
	ibanRegistered
	// zoneChecked: a passport or visa zone whose check digits all pass.
	zoneChecked
	// directoryCode: the directory holds the identifier's institution code
	// exactly. For a link, exactly one entry holds it: the key's card.
	directoryCode
	// zoneSays: a passport fact is what the page's passport zone says, and
	// that zone is itself zoneChecked.
	zoneSays
	// directoryJoin: in a run with the directory on, every printed name on
	// one key and the code on the other each look up to exactly one entry,
	// the same one.
	directoryJoin
)

// claim names what a row claims: an identifier's kind, a link's reason, a
// statement or passport fact, or a same_org row's reason.
type claim string

func identClaim(kind string) claim    { return claim("identifier " + kind) }
func linkClaim(reason string) claim   { return claim("link " + reason) }
func passportClaim(fact string) claim { return claim("passport " + fact) }
func stmtClaim(fact string) claim     { return claim("statement " + fact) }
func joinClaim(reason string) claim   { return claim("same_org " + reason) }

// certainBy mirrors FORMAT.md's tables: the one proof that makes each claim
// certain. A claim not listed here, such as any statement fact, passport
// current, or a link read from layout, is never certain.
var certainBy = map[claim]proof{
	identClaim(KindIBAN): ibanRegistered,
	identClaim(KindMRZ):  zoneChecked,
	identClaim(KindIFSC): directoryCode,
	identClaim(KindBIC):  directoryCode,

	linkClaim(ReasonIBANBankCode):   ibanRegistered,
	linkClaim(ReasonIFSCPrefix):     directoryCode,
	linkClaim(ReasonBICInstitution): directoryCode,

	passportClaim("holder"): zoneSays,
	passportClaim("number"): zoneSays,
	passportClaim("expiry"): zoneSays,

	joinClaim(JoinDirectory): directoryJoin,
}

// tierOf is the only way the extractor gives a tier.
func tierOf(c claim, p proof) Tier {
	if want, ok := certainBy[c]; ok && p == want {
		return Certain
	}
	return Likely
}
