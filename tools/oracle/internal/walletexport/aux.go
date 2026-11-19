// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package walletexport

// Sections [agendas], [treasury] and [vsp]: the top-level buckets added by
// the later upgrades, decoded per wallet/udb/agendas.go, treasury.go and
// vsp.go.

import "strconv"

func (e *exporter) aux() {
	// [agendas] (agendas.go:23-62): key BE32 version ‖ agenda ID, value the
	// choice ID; per-ticket preferences in one nested bucket per ticket.
	apath := []string{"agendaprefs"}
	agendaKey := func(path []string, k, v []byte) (string, string, bool) {
		if len(k) < 4 {
			e.bad(path, k, "key", v)
			return "", "", false
		}
		return vU(uint64(be32(k[:4]))), vS(k[4:]), true
	}
	e.each(apath, e.bucket(apath...), func(k, v []byte) {
		if ver, id, ok := agendaKey(apath, k, v); ok {
			e.sec("agendas").add("agenda", ver, id, vS(v))
		}
	})
	tpath := []string{"ticketsagendaprefs"}
	if tb := e.bucket(tpath...); tb != nil {
		c := tb.Cursor()
		for k, v := c.First(); k != nil; k, v = c.Next() {
			if v != nil {
				continue // left unclaimed: [unknown]
			}
			e.claims.claim(tpath, k)
			npath := append(append([]string(nil), tpath...), string(k))
			nb := tb.Bucket(k)
			if len(k) != 32 {
				e.each(npath, nb, func(kk, vv []byte) { e.bad(npath, kk, "key", vv) })
				continue
			}
			e.each(npath, nb, func(kk, vv []byte) {
				if ver, id, ok := agendaKey(npath, kk, vv); ok {
					e.sec("agendas").add("ticketagenda", vH(k), ver, id, vS(vv))
				}
			})
		}
	}

	// [treasury] (treasury.go:33-264): one vote byte per row.
	vote := func(path []string, k, v []byte) (string, bool) {
		if len(v) != 1 {
			e.bad(path, k, "len", v)
			return "", false
		}
		return vU(uint64(v[0])), true
	}
	tsp := []string{"tspendpolicy"}
	e.each(tsp, e.bucket(tsp...), func(k, v []byte) {
		if len(k) != 32 {
			e.bad(tsp, k, "key", v)
			return
		}
		if vt, ok := vote(tsp, k, v); ok {
			e.sec("treasury").add("tspendpolicy", vH(k), vt)
		}
	})
	vtsp := []string{"vsptspendpolicy"}
	e.each(vtsp, e.bucket(vtsp...), func(k, v []byte) {
		if len(k) != 64 {
			e.bad(vtsp, k, "key", v)
			return
		}
		if vt, ok := vote(vtsp, k, v); ok {
			e.sec("treasury").add("vsptspendpolicy", vH(k[:32]), vH(k[32:]), vt)
		}
	})
	trp := []string{"treasurypolicy"}
	e.each(trp, e.bucket(trp...), func(k, v []byte) {
		if vt, ok := vote(trp, k, v); ok {
			e.sec("treasury").add("treasurypolicy", vX(k), vt)
		}
	})
	vtrp := []string{"vsptreasurypolicy"}
	e.each(vtrp, e.bucket(vtrp...), func(k, v []byte) {
		if len(k) < 32 {
			e.bad(vtrp, k, "key", v)
			return
		}
		if vt, ok := vote(vtrp, k, v); ok {
			e.sec("treasury").add("vsptreasurypolicy", vH(k[:32]), vX(k[32:]), vt)
		}
	})

	e.flush("agendas")
	e.flush("treasury")
	e.vsp()
	e.flush("vsp")
}

// vsp exports the VSP tables without their surrogate host IDs
// (vsp.go:47-289): hosts by name with the pubkey SetVSPTicket reuses, and
// tickets with host and pubkey resolved exactly as GetVSPTicket resolves
// them. An ID at or below the counter vsphost/vsphostindex is never handed
// out again and is not exported (P2); an ID above it is, as its offset from
// the counter, because SetVSPTicket gives the next new hosts counter+1,
// counter+2, ... without checking whether the slot is taken (vsp.go:47-90):
// one vspfuture record per such ID that a vsphost key, a vsppubkey row or a
// vsp ticket names, and the offset on the vsphost and vspticket records
// that name it.
func (e *exporter) vsp() {
	hpath := []string{"vsphost"}
	hb := e.bucket(hpath...)
	ppath := []string{"vsppubkey"}
	pb := e.bucket(ppath...)
	vpath := []string{"vsp"}
	vb := e.bucket(vpath...)
	sec := e.sec("vsp")

	// vsphost: BE32 id -> BE32 len ‖ host, plus the counter key.
	hosts := make(map[uint32][]byte)
	var counter []byte
	e.each(hpath, hb, func(k, v []byte) {
		if string(k) == "vsphostindex" {
			counter = v
			if len(v) != 4 {
				e.bad(hpath, k, "len", v)
			}
			return
		}
		if len(k) != 4 {
			e.bad(hpath, k, "key", v)
			return
		}
		h, _, ok := lenPrefixedBE(v)
		if !ok {
			e.bad(hpath, k, "layout", v)
			return
		}
		hosts[be32(k)] = h
	})
	if counter == nil && hb != nil {
		e.bad(hpath, []byte("vsphostindex"), "missing", nil)
	}
	// Without a usable counter SetVSPTicket panics on a new host
	// (vsp.go:53-55, a [bad] record above); offsets are then taken from 0.
	var base uint32
	if len(counter) == 4 {
		base = be32(counter)
	}
	future := make(map[uint32]bool)
	offset := func(id uint32) string {
		if id <= base {
			return absent
		}
		future[id] = true
		return vU(uint64(id - base))
	}
	for id := range hosts {
		offset(id)
	}

	// vsppubkey: host -> BE32 id ‖ BE32 len ‖ pubkey.
	type pubkeyRow struct {
		id     uint32
		pubkey []byte
	}
	pubkeys := make(map[string]pubkeyRow)
	e.each(ppath, pb, func(k, v []byte) {
		if len(v) < 8 {
			e.bad(ppath, k, "len", v)
			return
		}
		pk, _, ok := lenPrefixedBE(v[4:])
		if !ok {
			e.bad(ppath, k, "layout", v)
			return
		}
		id := be32(v[:4])
		pubkeys[string(k)] = pubkeyRow{id, pk}
		idhost := absent
		if h, ok := hosts[id]; ok {
			idhost = vS(h)
		}
		sec.add("vsphost", vS(k), vX(pk), idhost, offset(id))
	})

	// Host IDs whose host has no pubkey row naming that ID (other than the
	// seed row 0 with an empty host).
	conflicts := make(map[string]int)
	for id, h := range hosts {
		if id == 0 && len(h) == 0 {
			continue
		}
		if p, ok := pubkeys[string(h)]; !ok || p.id != id {
			conflicts[string(h)]++
		}
	}
	for h, n := range conflicts {
		e.orphan("vsphostid", h, strconv.Itoa(n))
	}

	// vsp: ticket -> fee hash(32) ‖ BE32 status ‖ BE32 host ID.
	e.each(vpath, vb, func(k, v []byte) {
		if len(k) != 32 {
			e.bad(vpath, k, "key", v)
			return
		}
		if len(v) < 40 {
			e.bad(vpath, k, "len", v)
			return
		}
		id := be32(v[36:40])
		host := []byte{}
		pubkey := absent
		if h, ok := hosts[id]; ok && len(h) > 0 {
			host = h
			if p, ok := pubkeys[string(h)]; ok {
				pubkey = vX(p.pubkey)
			}
		}
		sec.add("vspticket", vH(k), vH(v[:32]), vU(uint64(be32(v[32:36]))), vS(host), pubkey, offset(id))
	})

	for id := range future {
		h := absent
		if hv, ok := hosts[id]; ok {
			h = vS(hv)
		}
		sec.add("vspfuture", vU(uint64(id-base)), h)
	}
	// The counter is behind an ID in use: the next new host overwrites a
	// slot something already names.
	if len(future) > 0 && len(counter) == 4 {
		e.bad(hpath, []byte("vsphostindex"), "counter", counter)
	}
}

// lenPrefixedBE splits <BE32 len><bytes> off the front of b.
func lenPrefixedBE(b []byte) (field, rest []byte, ok bool) {
	if len(b) < 4 {
		return nil, nil, false
	}
	n := uint64(be32(b))
	if uint64(len(b)-4) < n {
		return nil, nil, false
	}
	return b[4 : 4+n], b[4+n:], true
}
