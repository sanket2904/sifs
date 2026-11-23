package extract

import (
	"encoding/json"
	"os"

	"github.com/sanket2904/sifs/engine/extract/orgkey"
)

// Directory is the optional bank directory: a hint that adds a public id to
// an organisation the document already shows. Lookups answer only when
// exactly one entry holds the key.
type Directory struct {
	entries map[string]DirEntry
	byName  map[string][]string     // match key -> entry ids
	byCode  map[orgkey.Key][]string // code key -> entry ids
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
	d := &Directory{entries: map[string]DirEntry{}, byName: map[string][]string{}, byCode: map[orgkey.Key][]string{}}
	for _, e := range entries {
		d.entries[e.ID] = e
		for _, n := range e.Names {
			addOnce(d.byName, n.Key, e.ID)
		}
		for _, b := range e.Codes.BIC {
			if len(b) >= 8 {
				addOnce(d.byCode, orgkey.BIC(b), e.ID)
			}
		}
		for _, p := range e.Codes.IFSCPrefix {
			if len(p) == 4 {
				addOnce(d.byCode, orgkey.IFSC(p), e.ID)
			}
		}
		for _, c := range e.Codes.IBANBankCode {
			addOnce(d.byCode, orgkey.IBAN(c.Country, c.Code), e.ID)
		}
	}
	return d
}

func addOnce[K comparable](m map[K][]string, k K, id string) {
	for _, have := range m[k] {
		if have == id {
			return
		}
	}
	m[k] = append(m[k], id)
}

// lookupName gives the one entry whose names hold the key, if exactly one does.
func (d *Directory) lookupName(text string) (string, bool) {
	return only(d.byName[orgkey.MatchKey(text)])
}

// lookupCode gives the one entry holding an institution code, if exactly one does.
func (d *Directory) lookupCode(code orgkey.Key) (string, bool) { return only(d.byCode[code]) }

// holdsCode reports whether any entry holds an institution code exactly.
func (d *Directory) holdsCode(code orgkey.Key) bool { return len(d.byCode[code]) > 0 }

// lookupKey gives the one entry a key's own spelling looks up to: a code
// key's code, or a document key's name part.
func (d *Directory) lookupKey(key orgkey.Key) (string, bool) {
	if key.IsName() {
		return only(d.byName[key.Part()])
	}
	return d.lookupCode(key)
}

// settle adds to one document what the directory proves or suggests: an
// acronym name that is exactly one entry's name, the code identifiers it
// holds, the code links whose card it gives, and the joins of a printed
// name and a code that look up to one entry, the same one. Such a join is
// likely: the directory proves a code, never who prints a name.
func (d *Directory) settle(r *DocResult) {
	for i := range r.Mentions {
		m := &r.Mentions[i]
		if m.Key.IsZero() && m.kind == acronymName {
			if _, ok := d.lookupName(m.Text); ok {
				if k, ok := orgkey.Name(r.ID, m.Text); ok {
					m.Key, m.SettledBy = k, "directory"
				}
			}
		}
	}
	for i := range r.Identifiers {
		if id := &r.Identifiers[i]; id.isCode() && !id.Code.IsZero() && d.holdsCode(id.Code) {
			id.proof = directoryCode
		}
	}
	for i := range r.Links {
		l := &r.Links[i]
		if (l.reason == ReasonIFSCPrefix || l.reason == ReasonBICInstitution) && !l.key.IsZero() {
			if _, ok := d.lookupCode(l.key); ok {
				l.proof = directoryCode
			}
		}
	}
	names := map[orgkey.Key][]string{} // document key -> texts of its mentions
	for _, m := range r.Mentions {
		if m.Key.IsName() {
			names[m.Key] = append(names[m.Key], m.Text)
		}
	}
	codes := map[orgkey.Key]bool{} // code keys linked here by their own code
	for _, l := range r.Links {
		if l.reason == ReasonIFSCPrefix || l.reason == ReasonBICInstitution || l.reason == ReasonIBANBankCode {
			codes[l.key] = true
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
				r.Joins = append(r.Joins, Join{A: n, B: c, Reason: JoinDirectory})
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
