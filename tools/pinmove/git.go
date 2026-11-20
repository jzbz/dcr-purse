// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"bufio"
	"bytes"
	"fmt"
	"io"
	"os"
	"os/exec"
	"sort"
	"strconv"
	"strings"
)

// Git runs read-only git commands in one repository.  Every command is
// plumbing or is given its options explicitly, so that a user's diff and
// log configuration cannot change what the tool reads.
type Git struct {
	Dir string
}

// git runs git with the given arguments and returns its standard output.
func (g Git) git(stdin io.Reader, args ...string) ([]byte, error) {
	full := append([]string{"-C", g.Dir, "-c", "core.quotepath=off", "--no-pager"}, args...)
	cmd := exec.Command("git", full...)
	cmd.Stdin = stdin
	cmd.Env = append(os.Environ(), "GIT_TERMINAL_PROMPT=0", "LC_ALL=C")
	var out, errb bytes.Buffer
	cmd.Stdout = &out
	cmd.Stderr = &errb
	if err := cmd.Run(); err != nil {
		return nil, fmt.Errorf("git %s: %v: %s", strings.Join(args, " "), err, strings.TrimSpace(errb.String()))
	}
	return out.Bytes(), nil
}

// Commit resolves a revision to a full commit hash.
func (g Git) Commit(rev string) (string, error) {
	out, err := g.git(nil, "rev-parse", "--verify", "--quiet", "--end-of-options", rev+"^{commit}")
	if err != nil {
		return "", fmt.Errorf("%s is not a commit in %s", rev, g.Dir)
	}
	return strings.TrimSpace(string(out)), nil
}

// Files lists the files (blobs) of a commit's tree.
func (g Git) Files(commit string) ([]string, error) {
	out, err := g.git(nil, "ls-tree", "-r", "-z", "--full-tree", commit)
	if err != nil {
		return nil, err
	}
	var files []string
	for _, rec := range bytes.Split(out, []byte{0}) {
		if len(rec) == 0 {
			continue
		}
		// "<mode> SP <type> SP <object> TAB <path>"
		meta, name, ok := bytes.Cut(rec, []byte{'\t'})
		if !ok || !bytes.Contains(meta, []byte(" blob ")) {
			continue
		}
		files = append(files, string(name))
	}
	return files, nil
}

// Show returns a file's content at a commit, or ok false if it is absent.
func (g Git) Show(commit, path string) (data []byte, ok bool, err error) {
	out, err := g.git(strings.NewReader(commit+":"+path+"\n"), "cat-file", "--batch")
	if err != nil {
		return nil, false, err
	}
	br := bufio.NewReader(bytes.NewReader(out))
	hdr, err := br.ReadString('\n')
	if err != nil {
		return nil, false, err
	}
	if strings.HasSuffix(strings.TrimSpace(hdr), " missing") {
		return nil, false, nil
	}
	f := strings.Fields(hdr)
	if len(f) != 3 {
		return nil, false, fmt.Errorf("cat-file: unexpected header %q", hdr)
	}
	n, err := strconv.Atoi(f[2])
	if err != nil {
		return nil, false, err
	}
	data = make([]byte, n)
	if _, err := io.ReadFull(br, data); err != nil {
		return nil, false, err
	}
	return data, true, nil
}

// Blobs returns the content of each path at a commit; paths absent at the
// commit are left out.
func (g Git) Blobs(commit string, paths []string) (map[string][]byte, error) {
	var ok []string
	for _, p := range paths {
		// cat-file refuses a path outside the tree and stops; such a path
		// is simply absent.
		if p == "" || strings.HasPrefix(p, "/") || p == ".." || strings.HasPrefix(p, "../") ||
			strings.Contains(p, "/../") || strings.ContainsAny(p, "\n\r") {
			continue
		}
		ok = append(ok, p)
	}
	paths = ok
	var in bytes.Buffer
	for _, p := range paths {
		in.WriteString(commit + ":" + p + "\n")
	}
	out, err := g.git(&in, "cat-file", "--batch")
	if err != nil {
		return nil, err
	}
	blobs := make(map[string][]byte, len(paths))
	br := bufio.NewReader(bytes.NewReader(out))
	for _, p := range paths {
		hdr, err := br.ReadString('\n')
		if err != nil {
			return nil, err
		}
		if strings.HasSuffix(strings.TrimSpace(hdr), " missing") {
			continue
		}
		f := strings.Fields(hdr)
		if len(f) != 3 {
			return nil, fmt.Errorf("cat-file: unexpected header %q", hdr)
		}
		n, err := strconv.Atoi(f[2])
		if err != nil {
			return nil, err
		}
		data := make([]byte, n+1) // the content and its trailing LF
		if _, err := io.ReadFull(br, data); err != nil {
			return nil, err
		}
		blobs[p] = data[:n]
	}
	return blobs, nil
}

// LineCounts returns the number of lines of each path at a commit; paths
// absent at the commit are left out.
func (g Git) LineCounts(commit string, paths []string) (map[string]int, error) {
	blobs, err := g.Blobs(commit, paths)
	if err != nil {
		return nil, err
	}
	counts := make(map[string]int, len(blobs))
	for p, data := range blobs {
		counts[p] = lineCount(data)
	}
	return counts, nil
}

// lineCount counts lines as git does: a last line without a newline
// counts.
func lineCount(data []byte) int {
	n := bytes.Count(data, []byte{'\n'})
	if len(data) > 0 && data[len(data)-1] != '\n' {
		n++
	}
	return n
}

// Hunk is one hunk of a zero-context diff.  For OldCount 0 the hunk inserts
// after old line OldStart; for NewCount 0 it deletes.
type Hunk struct {
	OldStart, OldCount int
	NewStart, NewCount int
}

func (h Hunk) String() string {
	return fmt.Sprintf("-%d,%d +%d,%d", h.OldStart, h.OldCount, h.NewStart, h.NewCount)
}

// FileStatus is 'M' (modified), 'A' (added) or 'D' (deleted).
type FileStatus byte

// MarshalText writes the status letter, for the JSON report.
func (s FileStatus) MarshalText() ([]byte, error) { return []byte{byte(s)}, nil }

// FileDiff is the diff of one file between two commits.
type FileDiff struct {
	Path    string
	Status  FileStatus
	Binary  bool
	Hunks   []Hunk
	Renamed string // for 'D': the new path when rename detection pairs it
	// For 'D': an object of another type (a symbolic link, a submodule or
	// a regular file in place of either) takes the path at the new
	// revision.  git prints such a type change as a deletion followed by
	// an addition of the same path; the deletion is what a citation of the
	// old file sees.
	TypeChange bool `json:",omitempty"`
}

// Diff returns the changed files between two commits, keyed by old path
// (new path for added files), with zero-context hunks from the Myers
// algorithm.  Renames are not followed in the hunks; a deleted file that
// rename detection pairs with an added one carries the new path as a hint.
func (g Git) Diff(from, to string) (map[string]*FileDiff, error) {
	out, err := g.git(nil, "diff-tree", "-r", "-p", "-U0", "--no-color", "--no-ext-diff",
		"--no-textconv", "--no-renames", "--diff-algorithm=myers", "--src-prefix=a/", "--dst-prefix=b/",
		from, to)
	if err != nil {
		return nil, err
	}
	files, err := parsePatch(out)
	if err != nil {
		return nil, err
	}
	ns, err := g.git(nil, "diff-tree", "-r", "-z", "-M", "--name-status", from, to)
	if err != nil {
		return nil, err
	}
	f := bytes.Split(ns, []byte{0})
	for i := 0; i < len(f); i++ {
		st := string(f[i])
		if strings.HasPrefix(st, "R") && i+2 < len(f) {
			if fd := files[string(f[i+1])]; fd != nil {
				fd.Renamed = string(f[i+2])
			}
			i += 2
		} else if st != "" && i+1 < len(f) {
			i++
		}
	}
	return files, nil
}

func unquotePath(s string) string {
	if strings.HasPrefix(s, "\"") {
		if u, err := strconv.Unquote(s); err == nil {
			return u
		}
	}
	return s
}

// parsePatch parses the output of diff-tree -p -U0.  Each file section is
// stored once it is complete, keyed by its old path (new path for an added
// file).  A type change is printed as two sections for one path, a
// deletion and an addition; the deletion is kept, marked TypeChange, since
// the hunks of the addition do not apply to the old file's lines.
func parsePatch(out []byte) (map[string]*FileDiff, error) {
	files := make(map[string]*FileDiff)
	var cur *FileDiff
	store := func() {
		if cur == nil {
			return
		}
		prev := files[cur.Path]
		switch {
		case prev != nil && prev.Status == 'D' && cur.Status == 'A':
			prev.TypeChange = true
		case prev != nil && prev.Status == 'A' && cur.Status == 'D':
			cur.TypeChange = true
			files[cur.Path] = cur
		default:
			files[cur.Path] = cur
		}
		cur = nil
	}
	var newName string
	sc := bufio.NewScanner(bytes.NewReader(out))
	sc.Buffer(make([]byte, 1<<20), 64<<20)
	for sc.Scan() {
		line := sc.Text()
		switch {
		case strings.HasPrefix(line, "diff --git "):
			store()
			cur = &FileDiff{Status: 'M'}
			newName = ""
			// The header's names; a/ and b/ name the same path, since
			// renames are not detected in the patch.
			rest := strings.TrimPrefix(line, "diff --git ")
			if a, b, ok := strings.Cut(rest, " b/"); ok {
				cur.Path = strings.TrimPrefix(unquotePath(a), "a/")
				newName = unquotePath(b)
			}
		case cur == nil:
			continue
		case strings.HasPrefix(line, "new file mode"):
			cur.Status = 'A'
			cur.Path = newName
		case strings.HasPrefix(line, "deleted file mode"):
			cur.Status = 'D'
		case strings.HasPrefix(line, "Binary files"):
			cur.Binary = true
		case strings.HasPrefix(line, "@@ "):
			h, err := parseHunk(line)
			if err != nil {
				return nil, err
			}
			cur.Hunks = append(cur.Hunks, h)
		}
	}
	if err := sc.Err(); err != nil {
		return nil, err
	}
	store()
	for _, fd := range files {
		sort.Slice(fd.Hunks, func(i, j int) bool { return fd.Hunks[i].OldStart < fd.Hunks[j].OldStart })
	}
	return files, nil
}

// parseHunk parses "@@ -a[,b] +c[,d] @@ ...".
func parseHunk(line string) (Hunk, error) {
	f := strings.Fields(line)
	if len(f) < 4 || f[0] != "@@" || f[3] != "@@" {
		return Hunk{}, fmt.Errorf("bad hunk header %q", line)
	}
	parse := func(s string, sign byte) (int, int, error) {
		if len(s) < 2 || s[0] != sign {
			return 0, 0, fmt.Errorf("bad hunk range %q", s)
		}
		a, b, hasCount := strings.Cut(s[1:], ",")
		start, err := strconv.Atoi(a)
		if err != nil {
			return 0, 0, err
		}
		count := 1
		if hasCount {
			if count, err = strconv.Atoi(b); err != nil {
				return 0, 0, err
			}
		}
		return start, count, nil
	}
	var h Hunk
	var err error
	if h.OldStart, h.OldCount, err = parse(f[1], '-'); err != nil {
		return h, err
	}
	if h.NewStart, h.NewCount, err = parse(f[2], '+'); err != nil {
		return h, err
	}
	return h, nil
}

// LogEntry is one commit of a range.
type LogEntry struct {
	Hash    string
	Side    string // "+" reachable from the new revision only, "-" from the old only
	Date    string
	Subject string
	Merge   bool
	Files   []string
}

// Log lists the commits of the symmetric difference of two revisions,
// newest first, with the files each changes (first-parent diff for a
// merge, so its files are what it brought in).
func (g Git) Log(from, to string) ([]LogEntry, error) {
	out, err := g.git(nil, "log", "--no-color", "--left-right", "--no-renames", "--name-only",
		"--diff-merges=first-parent", "--format=%x1e%m%x1f%H%x1f%cs%x1f%P%x1f%s", from+"..."+to, "--")
	if err != nil {
		return nil, err
	}
	var entries []LogEntry
	for _, rec := range strings.Split(string(out), "\x1e") {
		if strings.TrimSpace(rec) == "" {
			continue
		}
		hdr, files, _ := strings.Cut(rec, "\n")
		f := strings.SplitN(hdr, "\x1f", 5)
		if len(f) != 5 {
			return nil, fmt.Errorf("git log: unexpected record %q", hdr)
		}
		side := "+"
		if f[0] == "<" {
			side = "-"
		}
		e := LogEntry{Side: side, Hash: f[1], Date: f[2], Merge: len(strings.Fields(f[3])) > 1, Subject: f[4]}
		for _, fl := range strings.Split(files, "\n") {
			if fl = strings.TrimSpace(fl); fl != "" {
				e.Files = append(e.Files, fl)
			}
		}
		entries = append(entries, e)
	}
	return entries, nil
}

// CommitDate returns a commit's committer date (YYYY-MM-DD).
func (g Git) CommitDate(commit string) (string, error) {
	out, err := g.git(nil, "show", "-s", "--no-color", "--format=%cs", commit)
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}

// Describe names a commit by a tag that points at it, if any.
func (g Git) Describe(commit string) string {
	out, err := g.git(nil, "tag", "--points-at", commit)
	if err != nil {
		return ""
	}
	tags := strings.Fields(string(out))
	return strings.Join(tags, ", ")
}
