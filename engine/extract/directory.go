package extract

import (
	"encoding/json"
	"os"
	"strings"
	"unicode"
)

// Directory is the optional bank directory: a hint that adds a public id to
// an organisation the document already shows. Lookups answer only when
// exactly one entry holds the key.
type Directory struct {
	entries map[string]DirEntry
	byName  map[string][]string // match key -> entry ids
	byCode  map[string][]string // code key -> entry ids
}

// DirEntry is one directory entry, reduced to what the extractor reads.
type DirEntry struct {
	ID    string `json:"id"`
	Name  string `json:"name"`
	Names []struct {
		Text string `json:"text"`
		Key  string `json:"key"`
	} `json:"names"`
	Codes struct {
		BIC          []string `json:"bic"`
		IFSCPrefix   []string `json:"ifsc_prefix"`
		IBANBankCode []struct {
			Country string `json:"country"`
			Code    string `json:"code"`
		} `json:"iban_bank_code"`
	} `json:"codes"`
}

// LoadDirectory reads directory/banks.json.
func LoadDirectory(path string) (*Directory, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var f struct {
		Entries []DirEntry `json:"entries"`
	}
	if err := json.Unmarshal(raw, &f); err != nil {
		return nil, err
	}
	return NewDirectory(f.Entries), nil
}

// NewDirectory indexes entries by match key and by institution code.
func NewDirectory(entries []DirEntry) *Directory {
	d := &Directory{entries: map[string]DirEntry{}, byName: map[string][]string{}, byCode: map[string][]string{}}
	add := func(m map[string][]string, k, id string) {
		for _, have := range m[k] {
			if have == id {
				return
			}
		}
		m[k] = append(m[k], id)
	}
	for _, e := range entries {
		d.entries[e.ID] = e
		for _, n := range e.Names {
			add(d.byName, n.Key, e.ID)
		}
		for _, b := range e.Codes.BIC {
			if len(b) >= 8 {
				add(d.byCode, "BIC "+b[:8], e.ID)
			}
		}
		for _, p := range e.Codes.IFSCPrefix {
			add(d.byCode, "IFSC "+p, e.ID)
		}
		for _, c := range e.Codes.IBANBankCode {
			add(d.byCode, "IBAN "+c.Country+" "+c.Code, e.ID)
		}
	}
	return d
}

// lookupName gives the one entry whose names hold the key, if exactly one does.
func (d *Directory) lookupName(text string) (string, bool) {
	return only(d.byName[matchKey(text)])
}

// lookupCode gives the one entry holding an institution code, if exactly one does.
func (d *Directory) lookupCode(code string) (string, bool) { return only(d.byCode[code]) }

// holdsCode reports whether any entry holds an institution code exactly.
func (d *Directory) holdsCode(code string) bool { return len(d.byCode[code]) > 0 }

// lookupKey gives the one entry a key's own spelling looks up to: a code
// key's code, or a name key's match key.
func (d *Directory) lookupKey(key string) (string, bool) {
	if k, ok := strings.CutPrefix(key, "name:"); ok {
		return only(d.byName[k])
	}
	return d.lookupCode(key)
}

// settle adds to one document what the directory proves or settles: an
// acronym name that is exactly one entry's name, the code identifiers it
// holds, the code links whose card it gives, and the joins of a printed
// name and a code that look up to one entry, the same one.
func (d *Directory) settle(r *DocResult) {
	for i := range r.Mentions {
		m := &r.Mentions[i]
		if m.Key == "" && m.kind == acronymName {
			if _, ok := d.lookupName(m.Text); ok {
				m.Key, m.SettledBy = "name:"+matchKey(m.Text), "directory"
			}
		}
	}
	for i := range r.Identifiers {
		if id := &r.Identifiers[i]; id.isCode() && id.Code != "" && d.holdsCode(id.Code) {
			id.proof = directoryCode
		}
	}
	for i := range r.Links {
		l := &r.Links[i]
		if (l.Reason == ReasonIFSCPrefix || l.Reason == ReasonBICInstitution) && l.Key != "" {
			if _, ok := d.lookupCode(l.Key); ok {
				l.proof = directoryCode
			}
		}
	}
	names := map[string][]string{} // name key -> texts of its mentions here
	for _, m := range r.Mentions {
		if strings.HasPrefix(m.Key, "name:") {
			names[m.Key] = append(names[m.Key], m.Text)
		}
	}
	codes := map[string]bool{} // code keys linked here by their own code
	for _, l := range r.Links {
		if l.Reason == ReasonIFSCPrefix || l.Reason == ReasonBICInstitution || l.Reason == ReasonIBANBankCode {
			codes[l.Key] = true
		}
	}
	for n, texts := range names {
		id, ok := d.lookupName(texts[0])
		for _, t := range texts[1:] {
			other, found := d.lookupName(t)
			ok = ok && found && other == id
		}
		if !ok {
			continue
		}
		for c := range codes {
			if e, found := d.lookupCode(c); found && e == id {
				r.Joins = append(r.Joins, Join{A: n, B: c, Reason: JoinDirectory, proof: directoryJoin})
			}
		}
	}
}

func only(ids []string) (string, bool) {
	if len(ids) == 1 {
		return ids[0], true
	}
	return "", false
}

var keyStopWords = map[string]bool{"the": true, "ltd": true, "limited": true, "plc": true, "inc": true, "ag": true, "sa": true, "nv": true}

// matchKey is the directory's match key: compatibility-folded, lower case,
// 株式会社 removed, split on everything that is not a letter, mark or digit,
// the legal-form words dropped, and the rest joined.
func matchKey(s string) string {
	s = strings.ReplaceAll(strings.ToLower(fold(s)), "株式会社", "")
	var b strings.Builder
	for _, w := range strings.FieldsFunc(s, func(r rune) bool {
		return !unicode.IsLetter(r) && !unicode.IsMark(r) && !unicode.IsDigit(r)
	}) {
		if !keyStopWords[w] {
			b.WriteString(w)
		}
	}
	return b.String()
}

// fold is the part of Unicode compatibility folding (NFKC) the sample's
// scripts need. Go's standard library has no normaliser: full-width ASCII
// becomes ASCII, the ideographic space a space, and the Devanagari letters
// that NFKC decomposes (क़ .. य़) become letter plus nukta.
func fold(s string) string {
	var b strings.Builder
	for _, r := range s {
		switch {
		case r >= 0xFF01 && r <= 0xFF5E:
			b.WriteRune(r - 0xFEE0)
		case r == 0x3000:
			b.WriteRune(' ')
		case r >= 0x0958 && r <= 0x095F:
			b.WriteRune([]rune("कखगजडढफय")[r-0x0958])
			b.WriteRune(0x093C)
		default:
			b.WriteRune(r)
		}
	}
	return b.String()
}
