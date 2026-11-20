// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// Class is what a move does to one cited line or range.
type Class uint8

// Classes.  Same, Shifted and Touched are ADR-0016's (Context, "Measured");
// Invalid marks a citation that was already wrong at the old revision.
const (
	Same    Class = iota // every line unchanged, at the same number
	Shifted              // every line unchanged and still contiguous, at new numbers
	Touched              // a line inside changed, or lines were inserted inside the range
	Gone                 // the file does not exist at the new revision, or another kind of object took its path
	Invalid              // the citation runs past the end of the file at the old revision
)

var classNames = [...]string{"same", "shifted", "touched", "gone", "invalid"}

func (c Class) String() string { return classNames[c] }

// mapLine maps old line n through hunks sorted by OldStart.  changed is set
// when the line was deleted or replaced.
func mapLine(hunks []Hunk, n int) (newLine int, changed bool) {
	delta := 0
	for _, h := range hunks {
		if h.OldCount == 0 {
			// Insertion after old line OldStart.
			if h.OldStart < n {
				delta += h.NewCount
				continue
			}
			break
		}
		last := h.OldStart + h.OldCount - 1
		if n < h.OldStart {
			break
		}
		if n <= last {
			return 0, true
		}
		delta += h.NewCount - h.OldCount
	}
	return n + delta, false
}

// Mapping is the outcome for one item.
type Mapping struct {
	Class      Class
	NewStart   int
	NewEnd     int
	Hunks      []Hunk // the hunks that make a touched item touched
	RenamedTo  string
	TypeChange bool // for Gone: another kind of object took the path
	OldLineEnd int  // the file's line count at the old revision, for Invalid
	// For Shifted and Same: why the item needs a human, its enclosing
	// declaration being another at the new revision (declReview); cite
	// rewrites only shifted items without one, and lists same items with
	// one for re-reading.
	Review string `json:",omitempty"`
}

// classify maps the old range [start, end] of a file.  fd is nil when the
// file did not change; oldLines is its line count at the old revision.
//
// A range is touched when any line in it changed or when lines were
// inserted between two of its lines (every old line still maps, but the
// range is no longer contiguous); it is never judged by its two ends alone.
func classify(fd *FileDiff, oldLines, start, end int) Mapping {
	if end > oldLines {
		return Mapping{Class: Invalid, OldLineEnd: oldLines}
	}
	if fd == nil {
		return Mapping{Class: Same, NewStart: start, NewEnd: end}
	}
	switch {
	case fd.Status == 'D':
		return Mapping{Class: Gone, RenamedTo: fd.Renamed, TypeChange: fd.TypeChange}
	case fd.Status != 'M':
		// An added file has no old lines to map; a cited path is a file at
		// the old revision, so this is reached only if a type change was
		// not paired with its deletion.  Never map through it.
		return Mapping{Class: Gone}
	case fd.Binary:
		return Mapping{Class: Touched}
	}
	var hit []Hunk
	for _, h := range fd.Hunks {
		if h.OldCount == 0 {
			if h.OldStart >= start && h.OldStart < end {
				hit = append(hit, h)
			}
			continue
		}
		last := h.OldStart + h.OldCount - 1
		if h.OldStart <= end && last >= start {
			hit = append(hit, h)
		}
	}
	if len(hit) > 0 {
		return Mapping{Class: Touched, Hunks: hit}
	}
	ns, _ := mapLine(fd.Hunks, start)
	ne := ns + (end - start)
	if ns == start {
		return Mapping{Class: Same, NewStart: ns, NewEnd: ne}
	}
	return Mapping{Class: Shifted, NewStart: ns, NewEnd: ne}
}

// MarshalText writes the class's name, for the JSON report.
func (c Class) MarshalText() ([]byte, error) { return []byte(c.String()), nil }
