// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package walletexport

// The lexical layer of the purse-export 2 format (ADR-0001 sections 2 to 8):
// the schema table, the enumerated values, value renderers, the record
// order, masking, the digests and the streaming writer. Nothing here knows
// about dcrwallet.

import (
	"bufio"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"hash"
	"io"
	"sort"
	"strconv"
	"strings"
)

// formatVersion is the number in the magic line.
const formatVersion = 2

// magic is the first line of every export.
var magic = "purse-export " + strconv.Itoa(formatVersion)

// absent is the value of an optional field that has no value.
const absent = "-"

// emptyBytes is the rendering of an empty byte string (type x and L).
const emptyBytes = "_"

type field struct {
	name     string
	typ      byte // u i b h x s L e
	optional bool // '?': the value may be "-"
	volatile bool // '~': random, host or udb-internal clock; masked by masked
	clock    bool // '@': a clock value the caller injects; masked by maskedclock
	written  bool // '^': write-only at the pin
}

type recType struct {
	name   string
	keys   int
	fields []field
}

type sectionDef struct {
	name    string
	version int
	// compared is false for the one section the eof digests leave out
	// ([residue], ADR-0001 section 4).
	compared bool
	types    []*recType
}

// rt parses a record type declaration such as "account 1 num:u name:s?".
func rt(decl string) *recType {
	parts := strings.Fields(decl)
	keys, err := strconv.Atoi(parts[1])
	if err != nil {
		panic(decl)
	}
	t := &recType{name: parts[0], keys: keys}
	for _, p := range parts[2:] {
		name, spec, ok := strings.Cut(p, ":")
		if !ok || len(spec) == 0 {
			panic(decl)
		}
		f := field{name: name, typ: spec[0]}
		flags := spec[1:]
		take := func(c byte) bool {
			if len(flags) > 0 && flags[0] == c {
				flags = flags[1:]
				return true
			}
			return false
		}
		f.optional = take('?')
		f.volatile = take('~')
		if !f.volatile {
			f.clock = take('@')
		}
		f.written = take('^')
		if flags != "" {
			panic(decl)
		}
		t.fields = append(t.fields, f)
	}
	if keys > len(t.fields) {
		panic(decl)
	}
	for _, f := range t.fields[:keys] {
		if f.optional || f.volatile || f.clock {
			panic("key field may not be optional, volatile or a clock value: " + decl)
		}
	}
	return t
}

func (t *recType) decl() string {
	var b strings.Builder
	fmt.Fprintf(&b, "=type %s %d", t.name, t.keys)
	for _, f := range t.fields {
		b.WriteByte(' ')
		b.WriteString(f.name)
		b.WriteByte(':')
		b.WriteByte(f.typ)
		if f.optional {
			b.WriteByte('?')
		}
		if f.volatile {
			b.WriteByte('~')
		}
		if f.clock {
			b.WriteByte('@')
		}
		if f.written {
			b.WriteByte('^')
		}
	}
	return b.String()
}

// schema is the section registry of format 2, in file order (ADR-0001
// section 6). Every section is always present.
var schema = []*sectionDef{
	{"wallet", 1, true, []*recType{
		rt("wallet 0 watchonly:b? lastaccount:u? lastimported:u? mgrcreated:u?~^ txcreated:i?~^ minedbal:i?^"),
	}},
	{"masterkeys", 1, true, []*recType{
		rt("masterkey 1 which:e salt:x~ digest:x~ n:u r:u p:u"),
	}},
	{"cryptokeys", 1, true, []*recType{
		rt("cryptokey 1 which:e sealed:x~"),
	}},
	{"cointypes", 1, true, []*recType{
		rt("cointype 1 which:e encpub:x?~ encpriv:x?~ xpub:s?"),
		rt("slip0044acct0 0 encpub:x~ encpriv:x~ nextext:u nextint:u name:s xpub:s?"),
	}},
	{"accounts", 1, true, []*recType{
		rt("account 1 num:u type:u name:s? idname:s? encpub:x~ encpriv:x~ kdfsalt:x?~ kdftime:u? kdfmem:u? kdfthreads:u?~ extused:u? intused:u? extret:u? intret:u? xpub:s?"),
	}},
	{"accountnames", 1, true, []*recType{
		rt("nameindex 1 name:s num:u?"),
	}},
	{"addresses", 1, true, []*recType{
		rt("address 1 key:x id:x? addr:s? type:e account:u idxacct:u? inset:b branch:u? index:u? encpub:x?~ encpriv:x?~ enchash:x?~ script:x? pubkey:x? added:u~^"),
	}},
	{"chain", 2, true, []*recType{
		rt("tip 0 hash:h"),
		rt("processed 0 hash:h"),
		rt("havecfilters 0 raw:x?"),
		rt("birth 0 set:b fromheight:b? fromtime:b? hash:h? height:u? time:i?@"),
	}},
	{"blocks", 2, true, []*recType{
		rt("blockseg 1 seg:u n:u sha256:x"),
		rt("block 1 height:u hash:h time:u votebits:u inv:b txs:L"),
	}},
	{"headers", 1, true, []*recType{
		rt("hdrseg 1 seg:u n:u sha256:x"),
		rt("header 1 height:u hash:h raw:x"),
		rt("hdrmissing 1 height:u hash:h"),
		rt("sideheader 1 hash:h raw:x"),
	}},
	{"cfilters", 1, true, []*recType{
		rt("cfseg 1 seg:u n:u sha256:x"),
		rt("cfilter 1 height:u hash:h key:x filter:x"),
		rt("cfmissing 1 height:u hash:h"),
		rt("sidecfilter 1 hash:h key:x filter:x"),
	}},
	{"txs", 2, true, []*recType{
		rt("tx 3 hash:h height:u block:h received:i@ raw:x"),
		rt("utx 1 hash:h received:i@ raw:x"),
	}},
	{"unpublished", 1, true, []*recType{
		rt("unpublished 1 hash:h"),
	}},
	{"credits", 1, true, []*recType{
		rt("credit 5 tx:h height:u block:h index:u inv:b amount:i change:b optag:u coinbase:b expiry:b spent:b sptx:h? spheight:u? spblock:h? spindex:u? stype:u?^ scriptoff:u? scriptlen:u? account:u? utxo:b"),
	}},
	{"debits", 1, true, []*recType{
		rt("debit 5 tx:h height:u block:h input:u inv:b amount:i crtx:h crheight:u crblock:h crindex:u"),
	}},
	{"ucredits", 1, true, []*recType{
		rt("ucredit 2 tx:h index:u amount:i change:b optag:u coinbase:b expiry:b spentbit:b stype:u? scriptoff:u? scriptlen:u? account:u?"),
	}},
	{"uinputs", 1, true, []*recType{
		rt("uinput 2 tx:h index:u spender:h"),
	}},
	{"tickets", 1, true, []*recType{
		rt("ticket 1 hash:h picked:i?^"),
	}},
	{"commitments", 1, true, []*recType{
		rt("commitment 2 tx:h index:u amount:i account:u? unspent:b unminedspent:b?"),
	}},
	{"multisig", 1, true, []*recType{
		rt("multisig 2 tx:h index:u scripthash:x m:u n:u spent:b stake:b block:h height:u amount:i sptx:h? spindex:u? txhash:h utxo:b"),
	}},
	{"agendas", 1, true, []*recType{
		rt("agenda 2 version:u agenda:s choice:s"),
		rt("ticketagenda 3 ticket:h version:u agenda:s choice:s"),
	}},
	{"treasury", 1, true, []*recType{
		rt("tspendpolicy 1 tspend:h vote:u"),
		rt("vsptspendpolicy 2 ticket:h tspend:h vote:u"),
		rt("treasurypolicy 1 key:x vote:u"),
		rt("vsptreasurypolicy 2 ticket:h key:x vote:u"),
	}},
	{"vsp", 2, true, []*recType{
		rt("vspfuture 1 offset:u host:s?"),
		rt("vsphost 1 host:s pubkey:x idhost:s? future:u?"),
		rt("vspticket 1 ticket:h feetx:h status:u host:s pubkey:x? future:u?"),
	}},
	{"orphans", 2, true, []*recType{
		rt("orphan 3 index:e ref:s val:s"),
	}},
	{"bad", 2, true, []*recType{
		rt("bad 3 path:s key:x reason:e value:x?"),
	}},
	{"unknown", 1, true, []*recType{
		rt("unknown 2 path:s key:x value:x?"),
	}},
	{"derived", 1, true, []*recType{
		rt("coin 0 type:u?"),
		rt("tipheight 0 height:i"),
		rt("missingcf 0 v:b"),
		rt("utxocount 0 n:u"),
		rt("acctprops 1 num:u name:s type:u extused:u intused:u extret:u intret:u importedkeys:u encrypted:b"),
		rt("balance 2 num:u minconf:i immcoinbase:i immstakegen:i lockedtickets:i spendable:i total:i votingauth:i unconfirmed:i"),
		rt("unspentticket 1 hash:h"),
		rt("failed 1 item:e"),
	}},
	{"privcheck", 2, true, []*recType{
		rt("mkeyok 0 ok:e"),
		rt("ctok 1 which:e ok:e"),
		rt("slip0044acct0ok 0 ok:e"),
		rt("acctok 1 num:u ok:e via:e"),
		rt("addrok 1 key:x ok:e"),
	}},
	{"residue", 1, false, []*recType{
		rt("residue 1 path:s entries:u sha256:x"),
	}},
}

// verdicts are the values of every [privcheck] ok field.
var verdicts = []string{"yes", "no", "nopass", "error"}

// enumValues lists, for every field of type e, the only values it may take
// (ADR-0001 section 3, "Enumerated values"). The writer refuses any other.
var enumValues = map[string][]string{
	"masterkey.which":    {"pub", "priv"},
	"cryptokey.which":    {"pub", "priv"},
	"cointype.which":     {"legacy", "slip0044"},
	"address.type":       {"chain", "key", "script"},
	"orphan.index":       {"addracct", "acctaddrs", "acctvars", "acctidname", "utxo", "multisigutxo", "cmtunspent", "vsphostid"},
	"bad.reason":         {"len", "key", "layout", "type", "value", "missing", "trail", "counter", "decrypt", "unreached", "api"},
	"failed.item":        {"coin", "tipheight", "missingcf", "utxocount", "acctprops", "balance", "unspentticket"},
	"mkeyok.ok":          verdicts,
	"ctok.which":         {"legacy", "slip0044"},
	"ctok.ok":            verdicts,
	"slip0044acct0ok.ok": verdicts,
	"acctok.ok":          verdicts,
	"acctok.via":         {"wallet", "account"},
	"addrok.ok":          verdicts,
}

// comparedMetas are the metas the eof digests cover, in file order
// (ADR-0001 sections 4 and 5).
var comparedMetas = []string{"network", "chain", "decrypt", "derived", "privcheck"}

func init() {
	// Every e field has a value list and every list names an e field.
	seen := make(map[string]bool)
	for _, s := range schema {
		for _, t := range s.types {
			for _, f := range t.fields {
				if f.typ != 'e' {
					continue
				}
				name := t.name + "." + f.name
				if len(enumValues[name]) == 0 {
					panic("no enumerated values for " + name)
				}
				seen[name] = true
			}
		}
	}
	for name := range enumValues {
		if !seen[name] {
			panic("enumerated values for a field that is not of type e: " + name)
		}
	}
}

func enumOK(t *recType, f field, v string) bool {
	for _, a := range enumValues[t.name+"."+f.name] {
		if a == v {
			return true
		}
	}
	return false
}

var sectionByName = func() map[string]*sectionDef {
	m := make(map[string]*sectionDef)
	for _, s := range schema {
		m[s.name] = s
	}
	return m
}()

// ---------------------------------------------------------------- values

func vU(v uint64) string { return strconv.FormatUint(v, 10) }
func vI(v int64) string  { return strconv.FormatInt(v, 10) }

func vB(v bool) string {
	if v {
		return "1"
	}
	return "0"
}

// vH renders a chainhash stored in its internal byte order in display order.
func vH(stored []byte) string {
	if len(stored) != 32 {
		panic(fmt.Sprintf("hash of %d bytes", len(stored)))
	}
	var r [32]byte
	for i := range r {
		r[i] = stored[31-i]
	}
	return hex.EncodeToString(r[:])
}

func vX(b []byte) string {
	if len(b) == 0 {
		return emptyBytes
	}
	return hex.EncodeToString(b)
}

// vS renders a byte string: printable ASCII except '"' and '\' as itself,
// everything else as \xHH, between double quotes.
func vS(b []byte) string {
	var sb strings.Builder
	sb.Grow(len(b) + 2)
	sb.WriteByte('"')
	for _, c := range b {
		if c >= 0x21 && c <= 0x7e && c != '"' && c != '\\' {
			sb.WriteByte(c)
		} else {
			fmt.Fprintf(&sb, "\\x%02x", c)
		}
	}
	sb.WriteByte('"')
	return sb.String()
}

// vL renders a list of stored hashes.
func vL(hashes [][]byte) string {
	if len(hashes) == 0 {
		return emptyBytes
	}
	parts := make([]string, len(hashes))
	for i, h := range hashes {
		parts[i] = vH(h)
	}
	return strings.Join(parts, ",")
}

// unS reverses vS (for ordering only; the input is always vS output).
func unS(s string) []byte {
	s = s[1 : len(s)-1]
	out := make([]byte, 0, len(s))
	for i := 0; i < len(s); i++ {
		if s[i] == '\\' {
			b, _ := hex.DecodeString(s[i+2 : i+4])
			out = append(out, b[0])
			i += 3
			continue
		}
		out = append(out, s[i])
	}
	return out
}

// ---------------------------------------------------------------- order

// cmpValue orders two rendered values of one field (ADR-0001 section 7):
// numbers numerically, s by its bytes, everything else by its rendering,
// with "_" (empty) before and "-" (absent) after every other value.
func cmpValue(typ byte, a, b string) int {
	if a == b {
		return 0
	}
	switch {
	case a == absent:
		return 1
	case b == absent:
		return -1
	case a == emptyBytes:
		return -1
	case b == emptyBytes:
		return 1
	}
	switch typ {
	case 'u', 'b':
		if len(a) != len(b) {
			if len(a) < len(b) {
				return -1
			}
			return 1
		}
		return strings.Compare(a, b)
	case 'i':
		x, _ := strconv.ParseInt(a, 10, 64)
		y, _ := strconv.ParseInt(b, 10, 64)
		switch {
		case x < y:
			return -1
		case x > y:
			return 1
		}
		return 0
	case 's':
		return strings.Compare(string(unS(a)), string(unS(b)))
	}
	return strings.Compare(a, b)
}

// cmpKeys orders two records of one type by their key fields.
func cmpKeys(t *recType, a, b []string) int {
	for k := 0; k < t.keys; k++ {
		if c := cmpValue(t.fields[k].typ, a[k], b[k]); c != 0 {
			return c
		}
	}
	return 0
}

// ---------------------------------------------------------------- masking

// render writes the record line of t with values, masked: a masked x value
// becomes "*" and its byte length, any other masked value "*"; "-" and "_"
// are never masked. maskVolatile masks '~' fields (the masked profile);
// maskClock also masks '@' fields (the maskedclock projection).
func render(b *strings.Builder, t *recType, values []string, maskVolatile, maskClock bool) {
	b.WriteString(t.name)
	for i, f := range t.fields {
		v := values[i]
		if ((maskVolatile && f.volatile) || (maskClock && f.clock)) && v != absent && v != emptyBytes {
			if f.typ == 'x' {
				v = "*" + strconv.Itoa(len(v)/2)
			} else {
				v = "*"
			}
		}
		b.WriteByte(' ')
		b.WriteString(f.name)
		b.WriteByte('=')
		b.WriteString(v)
	}
	b.WriteByte('\n')
}

// ---------------------------------------------------------------- writer

// exportFault is an exporter bug or a refused state found while writing
// (a record out of order, a duplicate key, a value outside its field's
// set). It is raised as a panic inside the writer and recovered by export.
type exportFault struct{ err error }

func fault(format string, args ...any) {
	panic(exportFault{fmt.Errorf(format, args...)})
}

// SectionSummary is what the export command reports per section.
type SectionSummary struct {
	Name    string `json:"name"`
	Records int    `json:"records"`
	SHA256  string `json:"sha256"`
	MSHA256 string `json:"msha256"`
}

// exportWriter streams an export: sections are opened in registry order,
// each record is checked, rendered, hashed and written as it is added, and
// nothing but the previous record's values is kept.
type exportWriter struct {
	bw                *bufio.Writer
	next              int
	sums              []SectionSummary
	eofRaw, eofMasked hash.Hash
	open              *secWriter
	line              strings.Builder
}

func newExportWriter(w io.Writer, comments []string, metas [][2]string) *exportWriter {
	ew := &exportWriter{
		bw:        bufio.NewWriterSize(w, 1<<20),
		eofRaw:    sha256.New(),
		eofMasked: sha256.New(),
	}
	ew.bw.WriteString(magic + "\n")
	for _, c := range comments {
		fmt.Fprintf(ew.bw, "# %s\n", c)
	}
	for _, m := range metas {
		fmt.Fprintf(ew.bw, "@%s=%s\n", m[0], m[1])
	}
	// The compared metas open both eof digests (ADR-0001 section 4).
	for _, want := range comparedMetas {
		for _, m := range metas {
			if m[0] == want {
				fmt.Fprintf(ew.eofRaw, "@%s=%s\n", m[0], m[1])
				fmt.Fprintf(ew.eofMasked, "@%s=%s\n", m[0], m[1])
			}
		}
	}
	return ew
}

// secWriter writes the records of one section.
type secWriter struct {
	ew          *exportWriter
	def         *sectionDef
	raw, masked hash.Hash
	n           int
	rank        int
	prev        []string
}

// section opens the next section, which must be name.
func (ew *exportWriter) section(name string) *secWriter {
	if ew.open != nil {
		fault("section %s opened while %s is open", name, ew.open.def.name)
	}
	if ew.next >= len(schema) || schema[ew.next].name != name {
		fault("section %s out of registry order", name)
	}
	def := schema[ew.next]
	ew.next++
	fmt.Fprintf(ew.bw, "=section %s %d\n", def.name, def.version)
	for _, t := range def.types {
		ew.bw.WriteString(t.decl() + "\n")
	}
	s := &secWriter{ew: ew, def: def, raw: sha256.New(), masked: sha256.New(), rank: -1}
	ew.open = s
	return s
}

// add checks, renders, hashes and writes one record. Records must arrive
// in the order of ADR-0001 section 7, and no two records of one type may
// have equal keys.
func (s *secWriter) add(typeName string, values ...string) {
	rank := -1
	for i, t := range s.def.types {
		if t.name == typeName {
			rank = i
			break
		}
	}
	if rank < 0 {
		fault("no record type %s in section %s", typeName, s.def.name)
	}
	t := s.def.types[rank]
	if len(values) != len(t.fields) {
		fault("%s.%s: %d values for %d fields", s.def.name, typeName, len(values), len(t.fields))
	}
	for i, f := range t.fields {
		v := values[i]
		if v == absent {
			if !f.optional {
				fault("%s.%s.%s may not be absent", s.def.name, typeName, f.name)
			}
			continue
		}
		if f.typ == 'e' && !enumOK(t, f, v) {
			fault("%s.%s: %q is not a value of the field", typeName, f.name, v)
		}
	}
	switch {
	case rank < s.rank:
		fault("%s record after a later type in section %s", typeName, s.def.name)
	case rank == s.rank:
		if t.keys == 0 {
			fault("more than one %s record", typeName)
		}
		switch c := cmpKeys(t, s.prev, values); {
		case c == 0:
			fault("two %s records with the key %v", typeName, values[:t.keys])
		case c > 0:
			fault("%s records out of order at %v", typeName, values[:t.keys])
		}
	}
	s.rank, s.prev = rank, values
	s.n++

	b := &s.ew.line
	b.Reset()
	render(b, t, values, false, false)
	line := b.String()
	s.ew.bw.WriteString(line)
	s.raw.Write([]byte(line))
	b.Reset()
	render(b, t, values, true, false)
	s.masked.Write([]byte(b.String()))
}

// close writes the section trailer and folds a compared section into the
// eof digests.
func (s *secWriter) close() {
	sum := SectionSummary{
		Name:    s.def.name,
		Records: s.n,
		SHA256:  hex.EncodeToString(s.raw.Sum(nil)),
		MSHA256: hex.EncodeToString(s.masked.Sum(nil)),
	}
	fmt.Fprintf(s.ew.bw, "=end %s records=%d sha256=%s msha256=%s\n", sum.Name, sum.Records, sum.SHA256, sum.MSHA256)
	if s.def.compared {
		fmt.Fprintf(s.ew.eofRaw, "%s %s\n", sum.Name, sum.SHA256)
		fmt.Fprintf(s.ew.eofMasked, "%s %s\n", sum.Name, sum.MSHA256)
	}
	s.ew.sums = append(s.ew.sums, sum)
	s.ew.open = nil
}

// finish writes the eof line and flushes.
func (ew *exportWriter) finish() (sums []SectionSummary, sha, msha string, err error) {
	if ew.open != nil || ew.next != len(schema) {
		fault("export finished with %d of %d sections written", ew.next, len(schema))
	}
	sha, msha = hex.EncodeToString(ew.eofRaw.Sum(nil)), hex.EncodeToString(ew.eofMasked.Sum(nil))
	fmt.Fprintf(ew.bw, "=eof sections=%d sha256=%s msha256=%s\n", len(schema), sha, msha)
	return ew.sums, sha, msha, ew.bw.Flush()
}

// sectionBuf collects the values of the records of a small section, whose
// producers do not emit in order, until it is written.
type sectionBuf struct {
	def     *sectionDef
	records []bufRecord
}

type bufRecord struct {
	rank   int
	values []string
}

func (s *sectionBuf) add(typeName string, values ...string) {
	for rank, t := range s.def.types {
		if t.name == typeName {
			if len(values) != len(t.fields) {
				fault("%s.%s: %d values for %d fields", s.def.name, typeName, len(values), len(t.fields))
			}
			s.records = append(s.records, bufRecord{rank, values})
			return
		}
	}
	fault("no record type %s in section %s", typeName, s.def.name)
}

// write sorts the records and writes them as section def through ew.
func (s *sectionBuf) write(ew *exportWriter) {
	sort.SliceStable(s.records, func(i, j int) bool {
		a, b := s.records[i], s.records[j]
		if a.rank != b.rank {
			return a.rank < b.rank
		}
		return cmpKeys(s.def.types[a.rank], a.values, b.values) < 0
	})
	w := ew.section(s.def.name)
	for _, r := range s.records {
		w.add(s.def.types[r.rank].name, r.values...)
	}
	w.close()
	s.records = nil
}
