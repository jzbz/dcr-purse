// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"time"

	"decred.org/dcrwallet/v5/wallet/udb"
	"decred.org/dcrwallet/v5/wallet/walletdb"
	"github.com/decred/dcrd/chaincfg/chainhash"
	"github.com/decred/dcrd/dcrutil/v4"
	"github.com/decred/dcrd/txscript/v4/stdscript"
)

// The hot command: the access paths brief section 6 Phase 2 names as
// dcrwallet's costs (balances recomputed by a full scan of the unspent
// outputs, range queries that walk every block record, address and account
// lookups), plus the cost of a small durable commit, each timed through
// udb's own API exactly as the wallet calls it. The Rust half is
// crates/purse-udb/tests/measure.rs, which runs the same queries over
// purse-store and holds every check value below equal to its own.

type balanceSpec struct {
	Account uint32 `json:"account"`
	MinConf int32  `json:"minconf"`
}

type hotArgs struct {
	DB       string `json:"db"`
	MinReps  int    `json:"min_reps"`
	MaxReps  int    `json:"max_reps"`
	BudgetMs int    `json:"budget_ms"`
	// AccountBalance(minconf, account), one call per repetition.
	Balances []balanceSpec `json:"balances"`
	// AccountBalances(minconf), one call per repetition.
	AllMinConfs []int32 `json:"all_minconfs"`
	// RangeTransactions(begin, end), stopping after limit transactions
	// when limit is above 0.
	Ranges []rangeSpec `json:"ranges"`
	// Output scripts (hex) looked up through Manager.Address and
	// Manager.AddrAccount, all in one read transaction per repetition.
	Scripts []string `json:"scripts"`
	// Account names looked up through Manager.LookupAccount, all in one
	// read transaction per repetition.
	Names []string `json:"names"`
	// Accounts read through Manager.AccountProperties, all in one read
	// transaction per repetition.
	Props []uint32 `json:"props"`
	// Write transactions recording one more external child of account 0
	// (SyncAccountToAddrIndex), each its own durable commit; last, since it
	// changes the database.
	NextAddr int `json:"next_addr"`
	// Store.MakeInputSource and SelectInputs(target) for (account,
	// minconf, target), the tip as the sync height, one source per
	// repetition (before NextAddr, which changes the database).
	Selects []selectSpec `json:"selects"`
	// Store.RandomUTXO calls at minconf 1, all in one read transaction per
	// repetition.
	RandomUTXOs int `json:"random_utxos"`
	// Store.UnspentTickets(tip, includeImmature) and a walk of
	// Store.IterateTickets, each one call per repetition (before NextAddr).
	Tickets bool `json:"tickets"`
}

type selectSpec struct {
	Account uint32 `json:"account"`
	MinConf int32  `json:"minconf"`
	Target  int64  `json:"target"`
}

// balanceText is a Balances value as the check compares it.
func balanceText(b *udb.Balances) string {
	return fmt.Sprintf("%d %d %d %d %d %d %d", b.Spendable, b.ImmatureCoinbaseRewards,
		b.ImmatureStakeGeneration, b.LockedByTickets, b.VotingAuthority, b.Unconfirmed, b.Total)
}

// digestText is "<count> <SHA-256 of the lines, each ending in a newline>".
func digestText(n int, lines []string) string {
	h := sha256.New()
	for _, l := range lines {
		h.Write([]byte(l))
		h.Write([]byte{'\n'})
	}
	return fmt.Sprintf("%d %s", n, hex.EncodeToString(h.Sum(nil)))
}

// rangeLines are RangeTransactions' batches as cmdExport writes them.
func rangeLines(details []udb.TxDetails) []string {
	lines := []string{fmt.Sprintf("batch %d", len(details))}
	for i := range details {
		d := &details[i]
		var sb strings.Builder
		fmt.Fprintf(&sb, "tx %s %d", hex.EncodeToString(d.Hash[:]), d.Block.Height)
		for _, c := range d.Credits {
			fmt.Fprintf(&sb, " c%d:%d:%t:%t", c.Index, c.Amount, c.Spent, c.Change)
		}
		for _, db := range d.Debits {
			fmt.Fprintf(&sb, " d%d:%d", db.Index, db.Amount)
		}
		lines = append(lines, sb.String())
	}
	return lines
}

func cmdHot(args json.RawMessage) (any, error) {
	var a hotArgs
	if err := unmarshal(args, &a); err != nil {
		return nil, err
	}
	w, err := openWallet(a.DB)
	if err != nil {
		return nil, err
	}
	defer w.close()
	ctx := context.Background()
	budget := time.Duration(a.BudgetMs) * time.Millisecond
	var out []timed
	checks := map[string]string{}
	view := func(f func(dbtx walletdb.ReadTx) error) error {
		return walletdb.View(ctx, w.db, f)
	}
	timeIt := func(name string, f func() error) error {
		s, err := repeat(a.MinReps, a.MaxReps, budget, f)
		if err != nil {
			return fmt.Errorf("%s: %w", name, err)
		}
		out = append(out, timed{Name: name, Summary: s})
		return nil
	}

	for _, b := range a.Balances {
		name := fmt.Sprintf("balance/account=%d/minconf=%d", b.Account, b.MinConf)
		var got udb.Balances
		err := timeIt(name, func() error {
			return view(func(dbtx walletdb.ReadTx) error {
				var err error
				got, err = w.store.AccountBalance(dbtx, b.MinConf, b.Account)
				return err
			})
		})
		if err != nil {
			return nil, err
		}
		checks[name] = balanceText(&got)
	}
	for _, mc := range a.AllMinConfs {
		name := fmt.Sprintf("balance_all/minconf=%d", mc)
		var got map[uint32]*udb.Balances
		err := timeIt(name, func() error {
			return view(func(dbtx walletdb.ReadTx) error {
				var err error
				got, err = w.store.AccountBalances(dbtx, mc)
				return err
			})
		})
		if err != nil {
			return nil, err
		}
		accts := make([]uint32, 0, len(got))
		for acct := range got {
			accts = append(accts, acct)
		}
		sort.Slice(accts, func(i, j int) bool { return accts[i] < accts[j] })
		parts := make([]string, len(accts))
		for i, acct := range accts {
			parts[i] = fmt.Sprintf("%d:%s", acct, balanceText(got[acct]))
		}
		checks[name] = strings.Join(parts, ";")
	}

	// The timed work of a range is the query alone: udb decodes every
	// transaction it passes, and the callback keeps each batch (udb reuses
	// its slice between calls, so the details are copied, as a caller
	// keeping them must). The text the check compares is built after the
	// timing, as the Rust side builds it from the batches its query
	// returned (crates/purse-udb/tests/measure.rs, rust_hot). Resetting a
	// kept answer with [:0] frees nothing here (the old batches are left
	// to the collector); the Rust side releases its previous answer before
	// its timing starts, for every query that keeps one (repeat_kept).
	for _, r := range a.Ranges {
		name := "range/" + r.Name
		var batches [][]udb.TxDetails
		var n int
		err := timeIt(name, func() error {
			batches, n = batches[:0], 0
			return view(func(dbtx walletdb.ReadTx) error {
				ns := dbtx.ReadBucket(wtxmgrKey)
				return w.store.RangeTransactions(ctx, ns, r.Begin, r.End,
					func(d []udb.TxDetails) (bool, error) {
						batches = append(batches, append([]udb.TxDetails(nil), d...))
						n += len(d)
						return r.Limit > 0 && n >= r.Limit, nil
					})
			})
		})
		if err != nil {
			return nil, err
		}
		var lines []string
		for _, b := range batches {
			lines = append(lines, rangeLines(b)...)
		}
		checks[name] = digestText(n, lines)
	}

	scripts := make([][]byte, len(a.Scripts))
	for i, s := range a.Scripts {
		if scripts[i], err = hex.DecodeString(s); err != nil {
			return nil, err
		}
	}
	if len(scripts) > 0 {
		var lines []string
		var hits int
		err := timeIt("lookup/address", func() error {
			lines, hits = lines[:0], 0
			return view(func(dbtx walletdb.ReadTx) error {
				ns := dbtx.ReadBucket(waddrmgrKey)
				for _, script := range scripts {
					_, addrs := stdscript.ExtractAddrs(0, script, params)
					line := "-"
					if len(addrs) == 1 {
						if ma, err := w.manager.Address(ns, addrs[0]); err == nil {
							branch, index := uint32(0), uint32(0)
							if ma.Internal() {
								branch = 1
							}
							if pk, ok := ma.(udb.ManagedPubKeyAddress); ok {
								index = pk.Index()
							}
							line = fmt.Sprintf("%d %d %d", ma.Account(), branch, index)
							hits++
						}
					}
					lines = append(lines, line)
				}
				return nil
			})
		})
		if err != nil {
			return nil, err
		}
		checks["lookup/address"] = digestText(hits, lines)

		err = timeIt("lookup/addr_account", func() error {
			lines, hits = lines[:0], 0
			return view(func(dbtx walletdb.ReadTx) error {
				ns := dbtx.ReadBucket(waddrmgrKey)
				for _, script := range scripts {
					_, addrs := stdscript.ExtractAddrs(0, script, params)
					line := "-"
					if len(addrs) == 1 {
						if acct, err := w.manager.AddrAccount(ns, addrs[0]); err == nil {
							line = fmt.Sprintf("%d", acct)
							hits++
						}
					}
					lines = append(lines, line)
				}
				return nil
			})
		})
		if err != nil {
			return nil, err
		}
		checks["lookup/addr_account"] = digestText(hits, lines)
	}

	if len(a.Names) > 0 {
		var lines []string
		var hits int
		err := timeIt("lookup/account_by_name", func() error {
			lines, hits = lines[:0], 0
			return view(func(dbtx walletdb.ReadTx) error {
				ns := dbtx.ReadBucket(waddrmgrKey)
				for _, name := range a.Names {
					line := "-"
					if acct, err := w.manager.LookupAccount(ns, name); err == nil {
						line = fmt.Sprintf("%d", acct)
						hits++
					}
					lines = append(lines, line)
				}
				return nil
			})
		})
		if err != nil {
			return nil, err
		}
		checks["lookup/account_by_name"] = digestText(hits, lines)
	}

	if len(a.Props) > 0 {
		var lines []string
		err := timeIt("lookup/account_properties", func() error {
			lines = lines[:0]
			return view(func(dbtx walletdb.ReadTx) error {
				ns := dbtx.ReadBucket(waddrmgrKey)
				for _, acct := range a.Props {
					p, err := w.manager.AccountProperties(ns, acct)
					if err != nil {
						return err
					}
					lines = append(lines, fmt.Sprintf("%d %s %d %d %d %d", p.AccountNumber,
						p.AccountName, p.LastUsedExternalIndex, p.LastUsedInternalIndex,
						p.LastReturnedExternalIndex, p.LastReturnedInternalIndex))
				}
				return nil
			})
		})
		if err != nil {
			return nil, err
		}
		checks["lookup/account_properties"] = digestText(len(lines), lines)
	}

	// Input selection. udb draws from dcrd's generator, seeded from the
	// operating system here as in the wallet, so a selection for a target
	// is checked only for covering it; a selection of everything (target
	// 0) is checked as the set of outputs, which the shuffle does not
	// change (crates/purse-udb/tests/measure.rs, rust_hot).
	var tipHeight int32
	if err := view(func(dbtx walletdb.ReadTx) error {
		_, tipHeight = w.store.MainChainTip(dbtx)
		return nil
	}); err != nil {
		return nil, err
	}
	for _, sel := range a.Selects {
		name := fmt.Sprintf("select/account=%d/minconf=%d/target=%d", sel.Account, sel.MinConf, sel.Target)
		var check string
		err := timeIt(name, func() error {
			return view(func(dbtx walletdb.ReadTx) error {
				src := w.store.MakeInputSource(dbtx, sel.Account, sel.MinConf, tipHeight, nil)
				d, err := src.SelectInputs(dcrutil.Amount(sel.Target))
				if err != nil {
					return err
				}
				if sel.Target == 0 {
					lines := make([]string, len(d.Inputs))
					for i, in := range d.Inputs {
						lines[i] = fmt.Sprintf("%s %d %d", in.PreviousOutPoint.Hash,
							in.PreviousOutPoint.Index, in.ValueIn)
					}
					sort.Strings(lines)
					check = digestText(len(lines), lines)
				} else {
					check = fmt.Sprintf("%t", int64(d.Amount) >= sel.Target)
				}
				return nil
			})
		})
		if err != nil {
			return nil, err
		}
		checks[name] = check
	}
	if a.RandomUTXOs > 0 {
		var found int
		err := timeIt("random_utxo/minconf=1", func() error {
			found = 0
			return view(func(dbtx walletdb.ReadTx) error {
				for i := 0; i < a.RandomUTXOs; i++ {
					c, err := w.store.RandomUTXO(dbtx, 1, tipHeight)
					if err != nil {
						return err
					}
					if c != nil {
						found++
					}
				}
				return nil
			})
		})
		if err != nil {
			return nil, err
		}
		checks["random_utxo/minconf=1"] = fmt.Sprintf("%d", found)
	}

	// Ticket queries. The answers are formatted after the timing, as the
	// Rust side formats the vectors its queries return.
	if a.Tickets {
		var hashes []chainhash.Hash
		err := timeIt("tickets/unspent", func() error {
			return view(func(dbtx walletdb.ReadTx) error {
				var err error
				hashes, err = w.store.UnspentTickets(dbtx, tipHeight, true)
				return err
			})
		})
		if err != nil {
			return nil, err
		}
		lines := make([]string, len(hashes))
		for i := range hashes {
			lines[i] = hex.EncodeToString(hashes[i][:])
		}
		checks["tickets/unspent"] = digestText(len(lines), lines)

		var tickets []udb.Ticket
		err = timeIt("tickets/iterate", func() error {
			tickets = tickets[:0]
			return view(func(dbtx walletdb.ReadTx) error {
				it := w.store.IterateTickets(dbtx)
				defer it.Close()
				for it.Next() {
					tickets = append(tickets, it.Ticket)
				}
				return it.Err()
			})
		})
		if err != nil {
			return nil, err
		}
		lines = make([]string, len(tickets))
		for i := range tickets {
			t := &tickets[i]
			lines[i] = fmt.Sprintf("%s %d %s", hex.EncodeToString(t.Hash[:]), t.Block.Height,
				hex.EncodeToString(t.SpenderHash[:]))
		}
		checks["tickets/iterate"] = digestText(len(lines), lines)
	}

	if a.NextAddr > 0 {
		// One past the last recorded external child of account 0, so each
		// commit derives and records one address, as cmdBench's
		// next_address does.
		var next uint32
		err := view(func(dbtx walletdb.ReadTx) error {
			ns := dbtx.ReadBucket(waddrmgrKey)
			return w.manager.ForEachAccountAddress(ns, 0, func(ma udb.ManagedAddress) error {
				if pk, ok := ma.(udb.ManagedPubKeyAddress); ok && !ma.Internal() && pk.Index() >= next {
					next = pk.Index() + 1
				}
				return nil
			})
		})
		if err != nil {
			return nil, err
		}
		first := next
		s, err := repeat(a.NextAddr, a.NextAddr, 0, func() error {
			child := next
			next++
			return walletdb.Update(ctx, w.db, func(dbtx walletdb.ReadWriteTx) error {
				ns := dbtx.ReadWriteBucket(waddrmgrKey)
				return w.manager.SyncAccountToAddrIndex(ns, 0, child, 0)
			})
		})
		if err != nil {
			return nil, err
		}
		out = append(out, timed{Name: "commit/next_address", Summary: s})
		checks["commit/next_address"] = fmt.Sprintf("%d %d", first, next)
	}

	return map[string]any{"timings": out, "checks": checks}, nil
}
