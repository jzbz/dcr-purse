// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

// Operations on udb.Store, the transaction store's package functions, and
// the runner's own operations (fixtures and the clock).

import (
	"bytes"
	"context"
	"sort"
	"time"

	"github.com/decred/dcrd/chaincfg/chainhash"
	"github.com/decred/dcrd/dcrutil/v4"
	"github.com/decred/dcrd/gcs/v4"
	"github.com/decred/dcrd/gcs/v4/blockcf2"
	"github.com/decred/dcrd/wire"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/seam"
	"github.com/jzbz/dcr-purse/tools/oracle/cmd/oprunner/internal/udb"
)

// ops is the vocabulary: operation name to definition. The "update" and
// "view" groups and the top-level-only "rt.checkpoint" and "rt.reopen"
// are handled by the runner itself.
var ops = map[string]opDef{
	// The runner's fixtures and clock.
	"tx.build": {readTx, "NewTxRecordFromMsgTx", opTxBuild},
	"tx.raw":   {noTx, "NewTxRecord", opTxRaw},
	"rt.clock_set": {noTx, "(the injected clock)", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		t, err := a.int64v("unix")
		if err != nil {
			return nil, err
		}
		seam.SetNow(t)
		return nil, nil
	}},
	"rt.clock_advance": {noTx, "(the injected clock)", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		d, err := a.int64v("seconds")
		if err != nil {
			return nil, err
		}
		seam.SetNow(seam.Unix() + d)
		return nil, nil
	}},

	// Chain, headers and filters.
	"store.main_chain_tip": {readTx, "Store.MainChainTip", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, height := r.store.MainChainTip(c.rtx)
		return map[string]any{"hash": h.String(), "height": height}, nil
	}},
	"store.extend_main_chain": {writeTx, "Store.ExtendMainChain", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		if _, ok := a.m["again"]; ok {
			return r.extendAgain(c, a)
		}
		label, err := a.optStr("label", "")
		if err != nil {
			return nil, err
		}
		if _, dup := r.blocks[label]; dup && label != "" {
			return nil, scriptErrorf("%s: block label %q already used", a.op, label)
		}
		header, f, err := r.buildHeader(c, a)
		if err != nil {
			return nil, err
		}
		hash := header.BlockHash()
		if err := r.store.ExtendMainChain(c.txmgrW(), header, &hash, f); err != nil {
			return nil, err
		}
		if label != "" {
			r.blocks[label] = hash
		}
		return map[string]any{"hash": hash.String(), "height": header.Height}, nil
	}},
	"store.processed_txs_block_marker": {readTx, "Store.ProcessedTxsBlockMarker", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		return r.store.ProcessedTxsBlockMarker(c.rtx).String(), nil
	}},
	"store.update_processed_txs_block_marker": {writeTx, "Store.UpdateProcessedTxsBlockMarker", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argBlockHash(c, a, "block")
		if err != nil {
			return nil, err
		}
		return nil, r.store.UpdateProcessedTxsBlockMarker(c.wtx, &h)
	}},
	"udb.set_birth_state": {writeTx, "SetBirthState", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		bs := &udb.BirthdayState{}
		if a.has("block") {
			h, err := r.argBlockHash(c, a, "block")
			if err != nil {
				return nil, err
			}
			bs.Hash = h
		}
		var err error
		if bs.Height, err = a.optU32("height", 0); err != nil {
			return nil, err
		}
		t, err := a.optInt64("time", 0)
		if err != nil {
			return nil, err
		}
		bs.Time = time.Unix(t, 0)
		if bs.SetFromHeight, err = a.optBool("from_height", false); err != nil {
			return nil, err
		}
		if bs.SetFromTime, err = a.optBool("from_time", false); err != nil {
			return nil, err
		}
		return nil, udb.SetBirthState(c.wtx, bs)
	}},
	"udb.birth_state": {readTx, "BirthState", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		bs := udb.BirthState(c.rtx)
		if bs == nil {
			return nil, nil
		}
		return map[string]any{
			"hash": bs.Hash.String(), "height": bs.Height, "time": bs.Time.Unix(),
			"from_height": bs.SetFromHeight, "from_time": bs.SetFromTime,
		}, nil
	}},
	"store.is_missing_main_chain_cfilters": {readTx, "Store.IsMissingMainChainCFilters", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		return r.store.IsMissingMainChainCFilters(c.rtx), nil
	}},
	"store.missing_cfilters_height": {readTx, "Store.MissingCFiltersHeight", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		return r.store.MissingCFiltersHeight(c.rtx)
	}},
	"store.insert_missing_cfilters": {writeTx, "Store.InsertMissingCFilters", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		var refs []jsonRaw
		if err := a.decode("blocks", &refs); err != nil {
			return nil, err
		}
		var hashes []*chainhash.Hash
		var filters []*gcs.FilterV2
		for _, ref := range refs {
			h, err := r.blockHash(c, ref.RawMessage, a.op+" blocks")
			if err != nil {
				return nil, err
			}
			header, err := r.store.GetBlockHeader(c.rtx, &h)
			if err != nil {
				return nil, resolveErr(a.op+" blocks", "GetBlockHeader", err)
			}
			f, err := gcs.NewFilterV2(blockcf2.B, blockcf2.M, blockcf2.Key(&header.MerkleRoot), nil)
			if err != nil {
				return nil, err
			}
			hashes = append(hashes, &h)
			filters = append(filters, f)
		}
		return nil, r.store.InsertMissingCFilters(c.wtx, hashes, filters)
	}},
	"store.get_main_chain_cfilters": {readTx, "Store.GetMainChainCFilters", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argBlockHash(c, a, "start")
		if err != nil {
			return nil, err
		}
		incl, err := a.boolean("inclusive")
		if err != nil {
			return nil, err
		}
		// The Go caller passes storage whose length bounds the result.
		n, err := a.ranged("count", 0, 100000)
		if err != nil {
			return nil, err
		}
		fs, err := r.store.GetMainChainCFilters(c.rtx, &h, incl, make([]*udb.BlockCFilter, n))
		if err != nil {
			return nil, err
		}
		out := []any{}
		for _, f := range fs {
			out = append(out, map[string]any{
				"block": f.BlockHash.String(), "key": hexStr(f.Key[:]), "filter": hexStr(f.FilterV2.Bytes()),
			})
		}
		return out, nil
	}},
	"store.cfilter_v2": {readTx, "Store.CFilterV2", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argBlockHash(c, a, "block")
		if err != nil {
			return nil, err
		}
		key, f, err := r.store.CFilterV2(c.rtx, &h)
		if err != nil {
			return nil, err
		}
		return map[string]any{"key": hexStr(key[:]), "filter": hexStr(f.Bytes())}, nil
	}},
	"store.import_cfilters_v2": {writeTx, "Store.ImportCFiltersV2", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		start, err := a.i32("start_height")
		if err != nil {
			return nil, err
		}
		var hexes []string
		if err := a.decode("filters", &hexes); err != nil {
			return nil, err
		}
		data := make([][]byte, 0, len(hexes))
		for _, x := range hexes {
			b, err := decodeHex(x)
			if err != nil {
				return nil, scriptErrorf("%s: filters: %v", a.op, err)
			}
			data = append(data, b)
		}
		return nil, r.store.ImportCFiltersV2(c.wtx, start, data)
	}},
	"udb.extract_block_header": {readTx, "Store.GetSerializedBlockHeader, ExtractBlockHeaderParentHash, ExtractBlockHeaderHeight, ExtractBlockHeaderTime, RawBlockHeader.Height", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argBlockHash(c, a, "block")
		if err != nil {
			return nil, err
		}
		b, err := r.store.GetSerializedBlockHeader(c.txmgr(), &h)
		if err != nil {
			return nil, err
		}
		var raw udb.RawBlockHeader
		copy(raw[:], b)
		return map[string]any{
			"parent":     hexStr(udb.ExtractBlockHeaderParentHash(b)),
			"height":     udb.ExtractBlockHeaderHeight(b),
			"time":       udb.ExtractBlockHeaderTime(b),
			"raw_height": raw.Height(),
		}, nil
	}},
	"store.get_main_chain_block_hash_for_height": {readTx, "Store.GetMainChainBlockHashForHeight", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		height, err := a.i32("height")
		if err != nil {
			return nil, err
		}
		h, err := r.store.GetMainChainBlockHashForHeight(c.txmgr(), height)
		if err != nil {
			return nil, err
		}
		return h.String(), nil
	}},
	"store.get_serialized_block_header": {readTx, "Store.GetSerializedBlockHeader", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argBlockHash(c, a, "block")
		if err != nil {
			return nil, err
		}
		b, err := r.store.GetSerializedBlockHeader(c.txmgr(), &h)
		if err != nil {
			return nil, err
		}
		return hexStr(b), nil
	}},
	"store.get_block_header_time": {readTx, "Store.GetBlockHeaderTime", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argBlockHash(c, a, "block")
		if err != nil {
			return nil, err
		}
		return r.store.GetBlockHeaderTime(c.rtx, &h)
	}},
	"store.get_block_header": {readTx, "Store.GetBlockHeader", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argBlockHash(c, a, "block")
		if err != nil {
			return nil, err
		}
		header, err := r.store.GetBlockHeader(c.rtx, &h)
		if err != nil {
			return nil, err
		}
		return hexStr(headerBytes(header)), nil
	}},
	"store.block_in_main_chain": {readTx, "Store.BlockInMainChain", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argBlockHash(c, a, "block")
		if err != nil {
			return nil, err
		}
		main, inv := r.store.BlockInMainChain(c.rtx, &h)
		return map[string]any{"main": main, "invalidated": inv}, nil
	}},
	"store.get_block_meta_for_hash": {readTx, "Store.GetBlockMetaForHash", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argBlockHash(c, a, "block")
		if err != nil {
			return nil, err
		}
		m, err := r.store.GetBlockMetaForHash(c.txmgr(), &h)
		if err != nil {
			return nil, err
		}
		return blockMetaValue(&m), nil
	}},
	"store.get_main_chain_block_hashes": {readTx, "Store.GetMainChainBlockHashes", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argBlockHash(c, a, "start")
		if err != nil {
			return nil, err
		}
		incl, err := a.boolean("inclusive")
		if err != nil {
			return nil, err
		}
		// The Go caller passes storage whose length bounds the result.
		n, err := a.ranged("count", 0, 100000)
		if err != nil {
			return nil, err
		}
		hs, err := r.store.GetMainChainBlockHashes(c.txmgr(), &h, incl, make([]chainhash.Hash, n))
		if err != nil {
			return nil, err
		}
		return hashList(hs), nil
	}},
	"store.rollback": {writeTx, "Store.Rollback", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		height, err := a.i32("height")
		if err != nil {
			return nil, err
		}
		m, err := r.store.Rollback(c.wtx, height)
		if err != nil {
			return nil, err
		}
		// Go returns a map (random order): sorted by block hash here.
		keys := make([]chainhash.Hash, 0, len(m))
		for k := range m {
			keys = append(keys, k)
		}
		sortHashes(keys)
		out := []any{}
		for i := range keys {
			txs := []any{}
			for _, tx := range m[keys[i]] {
				txs = append(txs, tx.TxHash().String())
			}
			out = append(out, map[string]any{"block": keys[i].String(), "txs": txs})
		}
		return out, nil
	}},

	// Recording transactions.
	"store.insert_mined_tx": {writeTx, "Store.InsertMinedTx", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		rec, err := r.argTx(a, "tx")
		if err != nil {
			return nil, err
		}
		h, err := r.argBlockHash(c, a, "block")
		if err != nil {
			return nil, err
		}
		return nil, r.store.InsertMinedTx(c.wtx, rec, &h)
	}},
	"store.insert_mem_pool_tx": {writeTx, "Store.InsertMemPoolTx", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		rec, err := r.argTx(a, "tx")
		if err != nil {
			return nil, err
		}
		return nil, r.store.InsertMemPoolTx(c.wtx, rec)
	}},
	"store.add_credit": {writeTx, "Store.AddCredit", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		rec, err := r.argTx(a, "tx")
		if err != nil {
			return nil, err
		}
		idx, err := a.u32("index")
		if err != nil {
			return nil, err
		}
		change, err := a.optBool("change", false)
		if err != nil {
			return nil, err
		}
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		// Resolved last: its failure is the operation's result, after
		// every argument was checked.
		block, err := r.argBlockMeta(c, a, "block")
		if err != nil {
			return nil, err
		}
		return nil, r.store.AddCredit(c.wtx, rec, block, idx, change, acct)
	}},
	"store.add_ticket_commitment": {writeTx, "Store.AddTicketCommitment", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		rec, err := r.argTx(a, "tx")
		if err != nil {
			return nil, err
		}
		idx, err := a.u32("index")
		if err != nil {
			return nil, err
		}
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		return nil, r.store.AddTicketCommitment(c.txmgrW(), rec, idx, acct)
	}},
	"store.redeem_ticket_commitments": {writeTx, "Store.RedeemTicketCommitments", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		rec, err := r.argTx(a, "tx")
		if err != nil {
			return nil, err
		}
		block, err := r.argBlockMeta(c, a, "block")
		if err != nil {
			return nil, err
		}
		return nil, r.store.RedeemTicketCommitments(c.txmgrW(), rec, block)
	}},
	"store.add_multisig_out": {writeTx, "Store.AddMultisigOut", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		rec, err := r.argTx(a, "tx")
		if err != nil {
			return nil, err
		}
		idx, err := a.u32("index")
		if err != nil {
			return nil, err
		}
		block, err := r.argBlockMeta(c, a, "block")
		if err != nil {
			return nil, err
		}
		return nil, r.store.AddMultisigOut(c.wtx, rec, block, idx)
	}},
	"store.spend_multisig_out": {writeTx, "Store.SpendMultisigOut", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		op, err := r.argOutpoint(a, "outpoint")
		if err != nil {
			return nil, err
		}
		sh, err := r.argHash(a, "spender")
		if err != nil {
			return nil, err
		}
		idx, err := a.u32("index")
		if err != nil {
			return nil, err
		}
		return nil, r.store.SpendMultisigOut(c.txmgrW(), &op, sh, idx)
	}},
	"store.set_published": {writeTx, "Store.SetPublished", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "tx")
		if err != nil {
			return nil, err
		}
		p, err := a.boolean("published")
		if err != nil {
			return nil, err
		}
		return nil, r.store.SetPublished(c.wtx, &h, p)
	}},
	"store.remove_unconfirmed": {writeTx, "Store.RemoveUnconfirmed", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		rec, err := r.argTx(a, "tx")
		if err != nil {
			return nil, err
		}
		return nil, r.store.RemoveUnconfirmed(c.txmgrW(), &rec.MsgTx, &rec.Hash)
	}},
	"store.prune_unmined": {writeTx, "Store.PruneUnmined", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		sdiff, err := a.int64v("sdiff")
		if err != nil {
			return nil, err
		}
		hs, err := r.store.PruneUnmined(c.wtx, dcrutil.Amount(sdiff))
		if err != nil {
			return nil, err
		}
		return hashPtrList(hs), nil
	}},

	// Queries.
	"store.unspent_output_count": {readTx, "Store.UnspentOutputCount", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		return r.store.UnspentOutputCount(c.rtx), nil
	}},
	"store.random_utxo": {readTx, "Store.RandomUTXO", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		minConf, syncHeight, err := minConfSync(a)
		if err != nil {
			return nil, err
		}
		cr, err := r.store.RandomUTXO(c.rtx, minConf, syncHeight)
		if err != nil {
			return nil, err
		}
		return map[string]any{"credit~r": creditValue(cr)}, nil
	}},
	"store.unspent_outputs": {readTx, "Store.UnspentOutputs", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		cs, err := r.store.UnspentOutputs(c.rtx)
		if err != nil {
			return nil, err
		}
		out := []any{}
		for _, cr := range cs {
			out = append(out, creditValue(cr))
		}
		return out, nil
	}},
	"store.unspent_output": {readTx, "Store.UnspentOutput", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		op, err := r.argOutpoint(a, "outpoint")
		if err != nil {
			return nil, err
		}
		mempool, err := a.boolean("include_mempool")
		if err != nil {
			return nil, err
		}
		cr, err := r.store.UnspentOutput(c.txmgr(), op, mempool)
		if err != nil {
			return nil, err
		}
		return creditValue(cr), nil
	}},
	"store.for_each_unspent_outpoint": {readTx, "Store.ForEachUnspentOutpoint", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		out := []any{}
		err := r.store.ForEachUnspentOutpoint(c.rtx, func(op *wire.OutPoint) error {
			out = append(out, outpointValue(op))
			return nil
		})
		return out, err
	}},
	"store.is_unspent_outpoint": {readTx, "Store.IsUnspentOutpoint", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		op, err := r.argOutpoint(a, "outpoint")
		if err != nil {
			return nil, err
		}
		return r.store.IsUnspentOutpoint(c.rtx, &op), nil
	}},
	"store.exists_utxo": {readTx, "Store.ExistsUTXO", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		op, err := r.argOutpoint(a, "outpoint")
		if err != nil {
			return nil, err
		}
		return r.store.ExistsUTXO(c.rtx, &op), nil
	}},
	"store.unspent_tickets": {readTx, "Store.UnspentTickets", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		sh, err := a.i32("syncheight")
		if err != nil {
			return nil, err
		}
		imm, err := a.boolean("include_immature")
		if err != nil {
			return nil, err
		}
		hs, err := r.store.UnspentTickets(c.rtx, sh, imm)
		if err != nil {
			return nil, err
		}
		return hashList(hs), nil
	}},
	"store.own_ticket": {readTx, "Store.OwnTicket", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "ticket")
		if err != nil {
			return nil, err
		}
		return r.store.OwnTicket(c.rtx, &h), nil
	}},
	"store.iterate_tickets": {readTx, "Store.IterateTickets, TicketIterator.Next, Err, Close", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		it := r.store.IterateTickets(c.rtx)
		defer it.Close()
		out := []any{}
		for it.Next() {
			v := txRecordValue(&it.TxRecord)
			v["block"] = blockValue(&it.Block)
			v["spender"] = it.SpenderHash.String()
			out = append(out, v)
		}
		return out, it.Err()
	}},
	"store.get_multisig_output": {readTx, "Store.GetMultisigOutput", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		op, err := r.argOutpoint(a, "outpoint")
		if err != nil {
			return nil, err
		}
		m, err := r.store.GetMultisigOutput(c.txmgr(), &op)
		if err != nil {
			return nil, err
		}
		return map[string]any{
			"outpoint": outpointValue(m.OutPoint), "tree": m.Tree,
			"scripthash": hexStr(m.ScriptHash[:]), "m": m.M, "n": m.N,
			"tx": m.TxHash.String(), "block": m.BlockHash.String(),
			"height": m.BlockHeight, "amount": int64(m.Amount), "spent": m.Spent,
			"spentby": m.SpentBy.String(), "spentbyindex": m.SpentByIndex,
		}, nil
	}},
	"store.unspent_multisig_credits_for_address": {readTx, "Store.UnspentMultisigCreditsForAddress", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		addr, err := r.argAddress(c, a, "addr")
		if err != nil {
			return nil, err
		}
		ms, err := r.store.UnspentMultisigCreditsForAddress(c.rtx, addr)
		if err != nil {
			return nil, err
		}
		out := []any{}
		for _, m := range ms {
			out = append(out, map[string]any{
				"outpoint": outpointValue(m.OutPoint), "scripthash": hexStr(m.ScriptHash[:]),
				"script": hexStr(m.MSScript), "m": m.M, "n": m.N, "amount": int64(m.Amount),
			})
		}
		return out, nil
	}},
	"store.select_inputs": {readTx, "Store.MakeInputSource, InputSource.SelectInputs", opSelectInputs},
	"store.account_balance": {readTx, "Store.AccountBalance", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		minConf, err := a.i32("minconf")
		if err != nil {
			return nil, err
		}
		acct, err := a.u32("account")
		if err != nil {
			return nil, err
		}
		b, err := r.store.AccountBalance(c.rtx, minConf, acct)
		if err != nil {
			return nil, err
		}
		return balancesValue(&b), nil
	}},
	"store.account_balances": {readTx, "Store.AccountBalances", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		minConf, err := a.i32("minconf")
		if err != nil {
			return nil, err
		}
		m, err := r.store.AccountBalances(c.rtx, minConf)
		if err != nil {
			return nil, err
		}
		// Go returns a map (random order): sorted by account here.
		accts := make([]uint32, 0, len(m))
		for k := range m {
			accts = append(accts, k)
		}
		sort.Slice(accts, func(i, j int) bool { return accts[i] < accts[j] })
		out := []any{}
		for _, k := range accts {
			v := balancesValue(m[k])
			v["key"] = k
			out = append(out, v)
		}
		return out, nil
	}},
	"store.tx_details": {readTx, "Store.TxDetails, TxDetails.Height", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "tx")
		if err != nil {
			return nil, err
		}
		d, err := r.store.TxDetails(c.txmgr(), &h)
		if err != nil {
			return nil, err
		}
		return txDetailsValue(d), nil
	}},
	"store.unique_tx_details": {readTx, "Store.UniqueTxDetails", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "tx")
		if err != nil {
			return nil, err
		}
		meta, err := r.argBlockMeta(c, a, "block")
		if err != nil {
			return nil, err
		}
		var b *udb.Block
		if meta != nil {
			b = &meta.Block
		}
		d, err := r.store.UniqueTxDetails(c.txmgr(), &h, b)
		if err != nil {
			return nil, err
		}
		return txDetailsValue(d), nil
	}},
	"store.ticket_details": {readTx, "Store.TxDetails, Store.TicketDetails", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "tx")
		if err != nil {
			return nil, err
		}
		d, err := r.store.TxDetails(c.txmgr(), &h)
		if err != nil {
			return nil, err
		}
		td, err := r.store.TicketDetails(c.txmgr(), d)
		if err != nil {
			return nil, err
		}
		if td == nil {
			// Not a ticket: Go returns nil details and no error
			// (wallet/udb/txquery.go:208-215).
			return nil, nil
		}
		return map[string]any{"ticket": txDetailsValue(td.Ticket), "spender": txDetailsValue(td.Spender)}, nil
	}},
	"store.tx": {readTx, "Store.Tx", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "tx")
		if err != nil {
			return nil, err
		}
		tx, err := r.store.Tx(c.txmgr(), &h)
		if err != nil {
			return nil, err
		}
		return serializeTx(tx), nil
	}},
	"store.exists_tx": {readTx, "Store.ExistsTx", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "tx")
		if err != nil {
			return nil, err
		}
		return r.store.ExistsTx(c.txmgr(), &h), nil
	}},
	"store.exists_tx_mined_or_unmined": {readTx, "Store.ExistsTxMinedOrUnmined", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "tx")
		if err != nil {
			return nil, err
		}
		mined, unmined := r.store.ExistsTxMinedOrUnmined(c.txmgr(), &h)
		return map[string]any{"mined": mined, "unmined": unmined}, nil
	}},
	"store.tx_block_height": {readTx, "Store.TxBlockHeight", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		h, err := r.argHash(a, "tx")
		if err != nil {
			return nil, err
		}
		return r.store.TxBlockHeight(c.rtx, &h)
	}},
	"store.range_transactions": {readTx, "Store.RangeTransactions", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		begin, err := a.i32("begin")
		if err != nil {
			return nil, err
		}
		end, err := a.i32("end")
		if err != nil {
			return nil, err
		}
		stop, err := a.optI32("stop_after", -1)
		if err != nil {
			return nil, err
		}
		batches := []any{}
		err = r.store.RangeTransactions(context.Background(), c.txmgr(), begin, end, func(ds []udb.TxDetails) (bool, error) {
			batch := make([]any, 0, len(ds))
			for i := range ds {
				batch = append(batch, txDetailsValue(&ds[i]))
			}
			batches = append(batches, batch)
			return stop >= 0 && int32(len(batches)) >= stop, nil
		})
		return batches, err
	}},
	"store.range_blocks": {readTx, "Store.RangeBlocks", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		begin, err := a.i32("begin")
		if err != nil {
			return nil, err
		}
		end, err := a.i32("end")
		if err != nil {
			return nil, err
		}
		stop, err := a.optI32("stop_after", -1)
		if err != nil {
			return nil, err
		}
		out := []any{}
		err = r.store.RangeBlocks(c.txmgr(), begin, end, func(b *udb.Block) (bool, error) {
			out = append(out, blockValue(b))
			return stop >= 0 && int32(len(out)) >= stop, nil
		})
		return out, err
	}},
	"store.previous_pk_scripts": {readTx, "Store.PreviousPkScripts", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		rec, err := r.argTx(a, "tx")
		if err != nil {
			return nil, err
		}
		meta, err := r.argBlockMeta(c, a, "block")
		if err != nil {
			return nil, err
		}
		var b *udb.Block
		if meta != nil {
			b = &meta.Block
		}
		ss, err := r.store.PreviousPkScripts(c.txmgr(), rec, b)
		if err != nil {
			return nil, err
		}
		out := []any{}
		for _, s := range ss {
			out = append(out, hexStr(s))
		}
		return out, nil
	}},
	"store.spender": {readTx, "Store.Spender", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		op, err := r.argOutpoint(a, "outpoint")
		if err != nil {
			return nil, err
		}
		tx, idx, err := r.store.Spender(c.rtx, &op)
		if err != nil {
			return nil, err
		}
		return map[string]any{"tx": tx.TxHash().String(), "index": idx}, nil
	}},
	"store.unmined_txs": {readTx, "Store.UnminedTxs", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		recs, err := r.store.UnminedTxs(c.rtx)
		if err != nil {
			return nil, err
		}
		// Go orders by dependency, breaking ties in map order: the
		// records are sorted by hash here, and whether Go's order put
		// every transaction after the unmined transactions it spends is
		// reported separately.
		pos := make(map[chainhash.Hash]int, len(recs))
		for i, rec := range recs {
			pos[rec.Hash] = i
		}
		depOK := true
		for i, rec := range recs {
			for _, in := range rec.MsgTx.TxIn {
				if j, ok := pos[in.PreviousOutPoint.Hash]; ok && j > i {
					depOK = false
				}
			}
		}
		sorted := append([]*udb.TxRecord(nil), recs...)
		sort.Slice(sorted, func(i, j int) bool {
			return lessHash(&sorted[i].Hash, &sorted[j].Hash)
		})
		out := []any{}
		for _, rec := range sorted {
			out = append(out, txRecordValue(rec))
		}
		return map[string]any{"txs": out, "dependency_order": depOK}, nil
	}},
	"store.unmined_tx_hashes": {readTx, "Store.UnminedTxHashes", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		hs, err := r.store.UnminedTxHashes(c.txmgr())
		if err != nil {
			return nil, err
		}
		return hashPtrList(hs), nil
	}},
	"udb.needs_migration": {noTx, "NeedsMigration", func(r *runner, c *opCtx, a *opArgs) (any, error) {
		return udb.NeedsMigration(r.ctx, r.db)
	}},
}

// extendAgain hands Store.ExtendMainChain the stored header and filter of
// block "again" a second time, as a Go caller does when it is notified of
// a block it already connected (wallet/chainntfns.go:46 tolerates the
// Exist error this gives for a block in the main chain). No header is
// built, so the extend counter does not move.
func (r *runner) extendAgain(c *opCtx, a *opArgs) (any, error) {
	for _, k := range []string{"label", "parent", "height", "votebits", "stakeversion", "time", "filter"} {
		if _, ok := a.m[k]; ok {
			return nil, scriptErrorf("%s: again takes no header argument (%q)", a.op, k)
		}
	}
	h, err := r.argBlockHash(c, a, "again")
	if err != nil {
		return nil, err
	}
	b, err := r.store.GetSerializedBlockHeader(c.txmgr(), &h)
	if err != nil {
		return nil, resolveErr(a.op+" again", "GetSerializedBlockHeader", err)
	}
	var header wire.BlockHeader
	if err := header.Deserialize(bytes.NewReader(b)); err != nil {
		return nil, resolveErr(a.op+" again", "BlockHeader.Deserialize", err)
	}
	_, f, err := r.store.CFilterV2(c.rtx, &h)
	if err != nil {
		return nil, resolveErr(a.op+" again", "CFilterV2", err)
	}
	hash := header.BlockHash()
	if err := r.store.ExtendMainChain(c.txmgrW(), &header, &hash, f); err != nil {
		return nil, err
	}
	return map[string]any{"hash": hash.String(), "height": header.Height}, nil
}

// jsonRaw lets a []json.RawMessage decode under DisallowUnknownFields.
type jsonRaw struct{ RawMessage []byte }

func (j *jsonRaw) UnmarshalJSON(b []byte) error {
	j.RawMessage = append([]byte(nil), b...)
	return nil
}

func minConfSync(a *opArgs) (int32, int32, error) {
	minConf, err := a.i32("minconf")
	if err != nil {
		return 0, 0, err
	}
	syncHeight, err := a.i32("syncheight")
	return minConf, syncHeight, err
}

// lessHash orders hashes by their display form (the reversed bytes),
// which is how the results print them.
func lessHash(a, b *chainhash.Hash) bool {
	for i := chainhash.HashSize - 1; i >= 0; i-- {
		if a[i] != b[i] {
			return a[i] < b[i]
		}
	}
	return false
}

func sortHashes(hs []chainhash.Hash) {
	sort.Slice(hs, func(i, j int) bool { return lessHash(&hs[i], &hs[j]) })
}

// opSelectInputs runs Store.MakeInputSource and then SelectInputs once
// per target in "targets", in one read transaction, as a Go caller's
// transaction author does with increasing targets. Every call's
// InputDetail is returned; the selection consumes the injected stream
// (randomUTXO and ShuffleSlice, wallet/udb/txmined.go:2358-2422, 3169).
func opSelectInputs(r *runner, c *opCtx, a *opArgs) (any, error) {
	acct, err := a.u32("account")
	if err != nil {
		return nil, err
	}
	minConf, syncHeight, err := minConfSync(a)
	if err != nil {
		return nil, err
	}
	var targets []int64
	if err := a.decode("targets", &targets); err != nil {
		return nil, err
	}
	var ignore func(*wire.OutPoint) bool
	if a.has("ignore") {
		var refs []jsonRaw
		if err := a.decode("ignore", &refs); err != nil {
			return nil, err
		}
		set := make(map[wire.OutPoint]bool)
		for _, ref := range refs {
			op, err := r.outpoint(ref.RawMessage, a.op+" ignore")
			if err != nil {
				return nil, err
			}
			set[op] = true
		}
		ignore = func(op *wire.OutPoint) bool { return set[*op] }
	}
	src := r.store.MakeInputSource(c.rtx, acct, minConf, syncHeight, ignore)
	out := []any{}
	for _, t := range targets {
		d, err := src.SelectInputs(dcrutil.Amount(t))
		if err != nil {
			return nil, err
		}
		ins := []any{}
		for i, in := range d.Inputs {
			ins = append(ins, map[string]any{
				"outpoint": outpointValue(&in.PreviousOutPoint), "value": in.ValueIn,
				"script": hexStr(d.Scripts[i]), "redeemsize": d.RedeemScriptSizes[i],
			})
		}
		out = append(out, map[string]any{"target": t, "amount": int64(d.Amount), "inputs": ins})
	}
	return map[string]any{"selections~r": out}, nil
}
