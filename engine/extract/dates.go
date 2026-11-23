package extract

import (
	"fmt"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/sanket2904/sifs/engine/extract/orgkey"
)

// Date is a calendar date.
type Date struct{ Y, M, D int }

func (d Date) String() string { return fmt.Sprintf("%04d-%02d-%02d", d.Y, d.M, d.D) }

func (d Date) before(e Date) bool { return d.String() < e.String() }

func makeDate(y, m, d int) (Date, bool) {
	t := time.Date(y, time.Month(m), d, 0, 0, 0, 0, time.UTC)
	if m < 1 || m > 12 || t.Day() != d || t.Month() != time.Month(m) {
		return Date{}, false
	}
	return Date{y, m, d}, true
}

// ParseDate reads YYYY-MM-DD.
func ParseDate(s string) (Date, bool) {
	t, err := time.Parse("2006-01-02", s)
	if err != nil {
		return Date{}, false
	}
	return Date{t.Year(), int(t.Month()), t.Day()}, true
}

// months holds month names in the sample's languages, folded to lower case.
var months = func() map[string]int {
	m := map[string]int{}
	for i, names := range [][]string{
		{"january", "jan", "januar", "janvier", "janv", "जनवरी"},
		{"february", "feb", "februar", "février", "fevrier", "févr", "fév", "फ़रवरी", "फरवरी"},
		{"march", "mar", "märz", "maerz", "mrz", "mär", "mars", "मार्च"},
		{"april", "apr", "avril", "avr", "अप्रैल"},
		{"may", "mai", "मई"},
		{"june", "jun", "juni", "juin", "जून"},
		{"july", "jul", "juli", "juillet", "juil", "जुलाई"},
		{"august", "aug", "août", "aout", "अगस्त"},
		{"september", "sep", "sept", "septembre", "सितंबर", "सितम्बर"},
		{"october", "oct", "oktober", "okt", "octobre", "अक्टूबर"},
		{"november", "nov", "novembre", "नवंबर", "नवम्बर"},
		{"december", "dec", "dezember", "dez", "décembre", "decembre", "déc", "दिसंबर", "दिसम्बर"},
	} {
		for _, n := range names {
			m[strings.ToLower(orgkey.Fold(n))] = i + 1
		}
	}
	return m
}()

// datePattern is one way of writing a date. read gives the date read day
// first and the date read month first; only a numeric date can differ.
type datePattern struct {
	re   *regexp.Regexp
	read func(g []string) [2]*Date
}

func atoi(s string) int { n, _ := strconv.Atoi(s); return n }

func both(y, m, d int) [2]*Date {
	if t, ok := makeDate(y, m, d); ok {
		return [2]*Date{&t, &t}
	}
	return [2]*Date{}
}

var datePatterns = []datePattern{
	{regexp.MustCompile(`([0-9]{4})年([0-9]{1,2})月([0-9]{1,2})日`), func(g []string) [2]*Date { return both(atoi(g[1]), atoi(g[2]), atoi(g[3])) }},
	{regexp.MustCompile(`\b([0-9]{4})-([0-9]{2})-([0-9]{2})\b`), func(g []string) [2]*Date { return both(atoi(g[1]), atoi(g[2]), atoi(g[3])) }},
	{regexp.MustCompile(`\b([0-9]{1,2})\.?\s+(\pL[\pL\pM]*)\.?,?\s+([0-9]{4})\b`), func(g []string) [2]*Date {
		return both(atoi(g[3]), months[strings.ToLower(orgkey.Fold(g[2]))], atoi(g[1]))
	}},
	{regexp.MustCompile(`\b(\pL+)\.?\s+([0-9]{1,2}),?\s+([0-9]{4})\b`), func(g []string) [2]*Date {
		return both(atoi(g[3]), months[strings.ToLower(orgkey.Fold(g[1]))], atoi(g[2]))
	}},
	{regexp.MustCompile(`\b([0-9]{1,2})[./-]([0-9]{1,2})[./-]([0-9]{4})\b`), func(g []string) [2]*Date {
		a, b, y := atoi(g[1]), atoi(g[2]), atoi(g[3])
		var out [2]*Date
		if t, ok := makeDate(y, b, a); ok {
			out[0] = &t
		}
		if t, ok := makeDate(y, a, b); ok {
			out[1] = &t
		}
		return out
	}},
}

// dateAt is a date written in text, read day first and month first.
type dateAt struct {
	b0, b1 int
	dates  [2]*Date
}

// findDates finds the dates written in s, in order, without overlaps.
func findDates(s string) []dateAt {
	var out []dateAt
	for _, p := range datePatterns {
		for _, m := range p.re.FindAllStringSubmatchIndex(s, -1) {
			g := make([]string, len(m)/2)
			for i := range g {
				if m[2*i] >= 0 {
					g[i] = s[m[2*i]:m[2*i+1]]
				}
			}
			ds := p.read(g)
			if ds[0] == nil && ds[1] == nil {
				continue
			}
			clash := false
			for _, o := range out {
				if m[0] < o.b1 && o.b0 < m[1] {
					clash = true
				}
			}
			if !clash {
				out = append(out, dateAt{m[0], m[1], ds})
			}
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].b0 < out[j].b0 })
	return out
}

// readPeriod reads a period written as two dates. Both are read in one
// order, day first or month first, and exactly one reading may give two
// dates with the first not after the second.
func readPeriod(s string) (from, to Date, ok bool) {
	ds := findDates(s)
	if len(ds) != 2 {
		return Date{}, Date{}, false
	}
	var got [][2]Date
	for o := 0; o < 2; o++ {
		a, b := ds[0].dates[o], ds[1].dates[o]
		if a != nil && b != nil && !b.before(*a) && (len(got) == 0 || got[0] != [2]Date{*a, *b}) {
			got = append(got, [2]Date{*a, *b})
		}
	}
	if len(got) != 1 {
		return Date{}, Date{}, false
	}
	return got[0][0], got[0][1], true
}

// readDate reads the first date in s; it fails when s holds none or the
// date reads differently day first and month first.
func readDate(s string) (Date, int, int, bool) {
	ds := findDates(s)
	if len(ds) == 0 {
		return Date{}, 0, 0, false
	}
	d, m := ds[0].dates[0], ds[0].dates[1]
	switch {
	case d != nil && m != nil && *d != *m:
		return Date{}, 0, 0, false
	case d == nil:
		d = m
	}
	return *d, ds[0].b0, ds[0].b1, true
}
