// Package extract finds organisations, identifiers, the ties between them,
// and statement and passport facts in a document's page text, by rules only.
package extract

import (
	"strings"
	"unicode/utf8"
)

// Page is one page of text as the text set gives it.
type Page struct {
	Text    string `json:"text"`
	Scanned bool   `json:"scanned"`
}

// Document is one file of a text set.
type Document struct {
	ID    string `json:"id"`
	Lang  string `json:"lang"`
	Pages []Page `json:"pages"`
}

// Span points at text on a page. Start and End count Unicode scalar values.
type Span struct {
	Page  int    `json:"page"`
	Start int    `json:"start"`
	End   int    `json:"end"`
	Text  string `json:"text"`
}

// Line is one line of a page with its offset in the page, in scalar values.
type Line struct {
	Page    int // from 1
	Index   int // from 0 within the page
	Start   int
	Text    string
	Scanned bool
}

// span returns the span of the byte range [b0, b1) of the line.
func (l Line) span(b0, b1 int) Span {
	s := l.Start + utf8.RuneCountInString(l.Text[:b0])
	return Span{Page: l.Page, Start: s, End: s + utf8.RuneCountInString(l.Text[b0:b1]), Text: l.Text[b0:b1]}
}

// splitLines cuts every page into lines, keeping each line's scalar offset.
func splitLines(doc Document) []Line {
	var out []Line
	for pi, p := range doc.Pages {
		off := 0
		for i, t := range strings.Split(p.Text, "\n") {
			out = append(out, Line{Page: pi + 1, Index: i, Start: off, Text: t, Scanned: p.Scanned})
			off += utf8.RuneCountInString(t) + 1
		}
	}
	return out
}

// before reports whether a comes before b in reading order.
func (a Span) before(b Span) bool {
	if a.Page != b.Page {
		return a.Page < b.Page
	}
	return a.Start < b.Start
}

func (a Span) overlaps(b Span) bool {
	return a.Page == b.Page && a.Start < b.End && b.Start < a.End
}
