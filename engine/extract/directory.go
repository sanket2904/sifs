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
	byCode  map[string][]string // institution code -> entry ids
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
			if len(b) >= 6 {
				add(d.byCode, "bic:"+b[:6], e.ID)
			}
		}
		for _, p := range e.Codes.IFSCPrefix {
			add(d.byCode, "ifsc:"+p, e.ID)
		}
		for _, c := range e.Codes.IBANBankCode {
			add(d.byCode, "iban:"+c.Country+":"+c.Code, e.ID)
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
