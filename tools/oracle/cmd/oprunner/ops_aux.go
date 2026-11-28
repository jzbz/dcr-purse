// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// Operations on udb's VSP, treasury-policy and agenda-preference records.

import (
	"bytes"
	"encoding/hex"
	"regexp"
	"sort"
	"strings"

	"github.com/decred/dcrd/blockchain/stake/v5"
	"github.com/decred/dcrd/chaincfg/chainhash"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/udb"
)

func decodeHex(s string) ([]byte, error) { return hex.DecodeString(s) }

// pointerText matches a Go pointer printed with %v.
var pointerText = regexp.MustCompile(`0x[0-9a-f]+`)

// canonPointer rewrites the one error text of udb that prints a heap
// address: GetVSPHost formats "no VSP host info for id %v" with &id
// (wallet/udb/vsp.go:203), so the text differs from run to run. The
// address becomes "<pointer>"; the kind is kept, and the error is
// wrapped so that a group member's "tolerate" still matches it.
func canonPointer(err error) error {
	text := err.Error()
	const marker = "no VSP host info for id 0x"
	i := strings.Index(text, marker)
	if i < 0 {
		return err
	}
	fixed := text[:i] + "no VSP host info for id " +
		pointerText.ReplaceAllString(text[i+len(marker)-2:], "<pointer>")
	v := errorValue(err)
	return &canonError{kind: v["kind"].(string), text: fixed, err: err}
}

func vspTicketValue(t *udb.VSPTicket) map[string]any {
	return map[string]any{
		"feehash": t.FeeHash.String(), "feetxstatus": t.FeeTxStatus,
		"vsphostid": t.VSPHostID, "host": t.Host, "pubkey": hexStr(t.PubKey),
	}
}

func policyArg(a *opArgs) (stake.TreasuryVoteT, error) {
	p, err := a.ranged("policy", 0, 255)
	return stake.TreasuryVoteT(p), err
}

func init() {
	for name, def := range auxOps {
		ops[name] = def
	}
}

var auxOps = map[string]opDef{
	"udb.set_vsp_ticket": {writeTx, "SetVSPTicket", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "ticket")
		if err != nil {
			return nil, err
		}
		rec := &udb.VSPTicket{}
		if rec.FeeHash, err = r.argHash(a, "fee_hash"); err != nil {
			return nil, err
		}
		if rec.FeeTxStatus, err = a.u32("fee_tx_status"); err != nil {
			return nil, err
		}
		if rec.VSPHostID, err = a.optU32("vsp_host_id", 0); err != nil {
			return nil, err
		}
		if rec.Host, err = a.str("host"); err != nil {
			return nil, err
		}
		if rec.PubKey, err = a.hexBytes("pubkey"); err != nil {
			return nil, err
		}
		if err := udb.SetVSPTicket(c.wtx, &h, rec); err != nil {
			return nil, err
		}
		// SetVSPTicket assigns the host ID into the record it is given.
		return map[string]any{"vsphostid": rec.VSPHostID}, nil
	}},
	"udb.delete_vsp_ticket": {writeTx, "DeleteVSPTicket", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "ticket")
		if err != nil {
			return nil, err
		}
		return nil, udb.DeleteVSPTicket(c.wtx, h)
	}},
	"udb.get_vsp_ticket": {readTx, "GetVSPTicket", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "ticket")
		if err != nil {
			return nil, err
		}
		t, err := udb.GetVSPTicket(c.rtx, h)
		if err != nil {
			return nil, err
		}
		return vspTicketValue(t), nil
	}},
	"udb.get_vsp_tickets_by_fee_status": {readTx, "GetVSPTicketsByFeeStatus", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		st, err := a.ranged("status", -1<<31, 1<<31-1)
		if err != nil {
			return nil, err
		}
		m, err := udb.GetVSPTicketsByFeeStatus(c.rtx, int(st))
		if err != nil {
			return nil, err
		}
		// Go returns a map (random order): sorted by ticket hash here.
		keys := make([]chainhash.Hash, 0, len(m))
		for k := range m {
			keys = append(keys, k)
		}
		sortHashes(keys)
		out := []any{}
		for i := range keys {
			v := vspTicketValue(m[keys[i]])
			v["ticket"] = keys[i].String()
			out = append(out, v)
		}
		return out, nil
	}},
	"udb.set_vsp_host": {writeTx, "SetVSPHost", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		id, err := a.u32("id")
		if err != nil {
			return nil, err
		}
		host, err := a.str("host")
		if err != nil {
			return nil, err
		}
		return nil, udb.SetVSPHost(c.wtx, id, &udb.VSPHost{Host: []byte(host)})
	}},
	"udb.get_vsp_host": {readTx, "GetVSPHost", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		id, err := a.u32("id")
		if err != nil {
			return nil, err
		}
		h, err := udb.GetVSPHost(c.rtx, id)
		if err != nil {
			return nil, canonPointer(err)
		}
		return hexStr(h.Host), nil
	}},
	"udb.set_vsp_pub_key": {writeTx, "SetVSPPubKey", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		host, err := a.str("host")
		if err != nil {
			return nil, err
		}
		id, err := a.u32("id")
		if err != nil {
			return nil, err
		}
		pk, err := a.hexBytes("pubkey")
		if err != nil {
			return nil, err
		}
		return nil, udb.SetVSPPubKey(c.wtx, []byte(host), &udb.VSPPubKey{ID: id, PubKey: pk})
	}},
	"udb.get_vsp_pub_key": {readTx, "GetVSPPubKey", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		host, err := a.str("host")
		if err != nil {
			return nil, err
		}
		pk, err := udb.GetVSPPubKey(c.rtx, []byte(host))
		if err != nil {
			return nil, err
		}
		return map[string]any{"id": pk.ID, "pubkey": hexStr(pk.PubKey)}, nil
	}},

	"udb.set_tspend_policy": {writeTx, "SetTSpendPolicy", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "tspend")
		if err != nil {
			return nil, err
		}
		p, err := policyArg(a)
		if err != nil {
			return nil, err
		}
		return nil, udb.SetTSpendPolicy(c.wtx, &h, p)
	}},
	"udb.set_vsp_tspend_policy": {writeTx, "SetVSPTSpendPolicy", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		t, err := r.argHash(a, "ticket")
		if err != nil {
			return nil, err
		}
		h, err := r.argHash(a, "tspend")
		if err != nil {
			return nil, err
		}
		p, err := policyArg(a)
		if err != nil {
			return nil, err
		}
		return nil, udb.SetVSPTSpendPolicy(c.wtx, &t, &h, p)
	}},
	"udb.tspend_policy": {readTx, "TSpendPolicy", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "tspend")
		if err != nil {
			return nil, err
		}
		p, err := udb.TSpendPolicy(c.rtx, &h)
		return uint8(p), err
	}},
	"udb.vsp_tspend_policy": {readTx, "VSPTSpendPolicy", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		t, err := r.argHash(a, "ticket")
		if err != nil {
			return nil, err
		}
		h, err := r.argHash(a, "tspend")
		if err != nil {
			return nil, err
		}
		p, err := udb.VSPTSpendPolicy(c.rtx, &t, &h)
		return uint8(p), err
	}},
	"udb.tspend_policies": {readTx, "TSpendPolicies", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		m, err := udb.TSpendPolicies(c.rtx)
		if err != nil {
			return nil, err
		}
		keys := make([]chainhash.Hash, 0, len(m))
		for k := range m {
			keys = append(keys, k)
		}
		sortHashes(keys)
		out := []any{}
		for i := range keys {
			out = append(out, map[string]any{"tspend": keys[i].String(), "policy": uint8(m[keys[i]])})
		}
		return out, nil
	}},
	"udb.vsp_tspend_policies": {readTx, "VSPTSpendPolicies", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		m, err := udb.VSPTSpendPolicies(c.rtx)
		if err != nil {
			return nil, err
		}
		keys := make([]udb.VSPTSpend, 0, len(m))
		for k := range m {
			keys = append(keys, k)
		}
		sort.Slice(keys, func(i, j int) bool {
			if keys[i].Ticket != keys[j].Ticket {
				return lessHash(&keys[i].Ticket, &keys[j].Ticket)
			}
			return lessHash(&keys[i].TSpend, &keys[j].TSpend)
		})
		out := []any{}
		for _, k := range keys {
			out = append(out, map[string]any{
				"ticket": k.Ticket.String(), "tspend": k.TSpend.String(), "policy": uint8(m[k]),
			})
		}
		return out, nil
	}},
	"udb.set_treasury_key_policy": {writeTx, "SetTreasuryKeyPolicy", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		k, err := a.hexBytes("pikey")
		if err != nil {
			return nil, err
		}
		p, err := policyArg(a)
		if err != nil {
			return nil, err
		}
		return nil, udb.SetTreasuryKeyPolicy(c.wtx, k, p)
	}},
	"udb.set_vsp_treasury_key_policy": {writeTx, "SetVSPTreasuryKeyPolicy", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		t, err := r.argHash(a, "ticket")
		if err != nil {
			return nil, err
		}
		k, err := a.hexBytes("pikey")
		if err != nil {
			return nil, err
		}
		p, err := policyArg(a)
		if err != nil {
			return nil, err
		}
		return nil, udb.SetVSPTreasuryKeyPolicy(c.wtx, &t, k, p)
	}},
	"udb.treasury_key_policy": {readTx, "TreasuryKeyPolicy", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		k, err := a.hexBytes("pikey")
		if err != nil {
			return nil, err
		}
		p, err := udb.TreasuryKeyPolicy(c.rtx, k)
		return uint8(p), err
	}},
	"udb.vsp_treasury_key_policy": {readTx, "VSPTreasuryKeyPolicy", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		t, err := r.argHash(a, "ticket")
		if err != nil {
			return nil, err
		}
		k, err := a.hexBytes("pikey")
		if err != nil {
			return nil, err
		}
		p, err := udb.VSPTreasuryKeyPolicy(c.rtx, &t, k)
		return uint8(p), err
	}},
	"udb.treasury_key_policies": {readTx, "TreasuryKeyPolicies", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		m, err := udb.TreasuryKeyPolicies(c.rtx)
		if err != nil {
			return nil, err
		}
		out := []any{}
		for _, k := range sortedKeys(m) {
			out = append(out, map[string]any{"pikey": hexStr([]byte(k)), "policy": uint8(m[k])})
		}
		return out, nil
	}},
	"udb.vsp_treasury_key_policies": {readTx, "VSPTreasuryKeyPolicies", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		m, err := udb.VSPTreasuryKeyPolicies(c.rtx)
		if err != nil {
			return nil, err
		}
		keys := make([]udb.VSPTreasuryKey, 0, len(m))
		for k := range m {
			keys = append(keys, k)
		}
		sort.Slice(keys, func(i, j int) bool {
			if keys[i].Ticket != keys[j].Ticket {
				return lessHash(&keys[i].Ticket, &keys[j].Ticket)
			}
			return bytes.Compare([]byte(keys[i].TreasuryKey), []byte(keys[j].TreasuryKey)) < 0
		})
		out := []any{}
		for _, k := range keys {
			out = append(out, map[string]any{
				"ticket": k.Ticket.String(), "pikey": hexStr([]byte(k.TreasuryKey)), "policy": uint8(m[k]),
			})
		}
		return out, nil
	}},

	"udb.set_default_agenda_preference": {writeTx, "SetDefaultAgendaPreference", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		ver, err := a.u32("version")
		if err != nil {
			return nil, err
		}
		agenda, err := a.str("agenda")
		if err != nil {
			return nil, err
		}
		choice, err := a.str("choice")
		if err != nil {
			return nil, err
		}
		return nil, udb.SetDefaultAgendaPreference(c.wtx, ver, agenda, choice)
	}},
	"udb.set_ticket_agenda_preference": {writeTx, "SetTicketAgendaPreference", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		t, err := r.argHash(a, "ticket")
		if err != nil {
			return nil, err
		}
		ver, err := a.u32("version")
		if err != nil {
			return nil, err
		}
		agenda, err := a.str("agenda")
		if err != nil {
			return nil, err
		}
		choice, err := a.str("choice")
		if err != nil {
			return nil, err
		}
		return nil, udb.SetTicketAgendaPreference(c.wtx, &t, ver, agenda, choice)
	}},
	"udb.default_agenda_preference": {readTx, "DefaultAgendaPreference", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		ver, err := a.u32("version")
		if err != nil {
			return nil, err
		}
		agenda, err := a.str("agenda")
		if err != nil {
			return nil, err
		}
		return udb.DefaultAgendaPreference(c.rtx, ver, agenda), nil
	}},
	"udb.ticket_agenda_preference": {readTx, "TicketAgendaPreference", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		t, err := r.argHash(a, "ticket")
		if err != nil {
			return nil, err
		}
		ver, err := a.u32("version")
		if err != nil {
			return nil, err
		}
		agenda, err := a.str("agenda")
		if err != nil {
			return nil, err
		}
		return udb.TicketAgendaPreference(c.rtx, &t, ver, agenda), nil
	}},
}
