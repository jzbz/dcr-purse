// Copyright (c) 2026 The dcr-purse developers
// Use of this source code is governed by an ISC license that can be found in
// the LICENSE file.

// Command oprunner is the operation-script runner of the oracle (brief
// section 6 Phase 0, section 7 Layer 2; docs/specs/23 section 5.5.2
// "udb.script"). It runs a JSON script (a wallet setup, then a list of
// operations) against dcrwallet's own wallet/udb at the parity pin with
// injected randomness, clock and processor count, plus the address
// issuance of dcrwallet's wallet package (the gap-limit policy), and
// returns every operation's canonically encoded result and, at
// checkpoints and at the end, the logical export of the wallet (format
// "purse-export 2", docs/adr/0001-logical-export-format.md). The script
// and result formats are docs/adr/0002-operation-script-format.md; a Rust
// runner over purse-udb will consume the same scripts. Error texts are
// rendered with dcrwallet's daemon separator ":: " (dcrwallet.go:36-39;
// ADR-0011).
//
// It speaks the linejson protocol. Commands:
//
//	run        {script, dir}
//	           Runs script in dir (missing or empty): the database is
//	           dir/wallet.db, results go to dir/results.jsonl, each export
//	           to dir/<name>.pe beside its snapshot dir/<name>.db. Returns
//	           {format, results, exports: {name: summary}, results_sha256,
//	           drawn}. A fault of the script (unknown operation or label,
//	           missing, malformed or unknown argument, an operation where
//	           the script's structure forbids it) fails the command, and
//	           so does a panic in the runner's own code; an error dcrwallet
//	           returns, including one met while resolving a reference (an
//	           account or block the wallet does not hold), and a panic
//	           inside dcrwallet's code, are operation results.
//	vocabulary {}
//	           Returns every operation: {op, tx, calls}.
//
// # How the seams reach udb
//
// udb takes its randomness from github.com/decred/dcrd/crypto/rand, a
// generator keyed from the operating system at package initialization,
// and its clock from time.Now(); neither has a seam in udb's API, and
// purse-testutil builds every oracle with a plain "go build" in this
// module, so neither a module replacement nor -overlay is available. The
// runner therefore links copies (the copy technique of docs/specs/23
// section 5.5), all under internal/:
//
//	internal/udb     wallet/udb, every non-test file
//	internal/snacl   wallet/internal/snacl (internal: not importable here)
//	internal/compat  internal/compat (internal: not importable here)
//	internal/kdf     kdf
//	internal/rand    dcrd/crypto/rand: default.go, uniform.go, doc.go
//	                 copied; prng.go replaced by the seeded stream of
//	                 docs/specs/23 section 5.6.2 (ChaCha20, zero nonce, no
//	                 re-keying, XOR into the buffer)
//	internal/seam    the injected clock and processor count
//
// copies_test.go fails unless each copied file equals the file of the
// module version go.mod pins, in the module cache, after exactly these
// rewrites and a gofmt pass, each with its exact count: imports of
// dcrd/crypto/rand, wallet/internal/snacl, internal/compat and kdf point
// at the copies; time.Now() becomes seam.Now() (5 sites); runtime.NumCPU()
// becomes seam.NumCPU() (1 site); addressdb.go loses its then unused time
// import. It also fails on any clock read, randomness import or processor
// count left anywhere in the copies, and its header lists the hand audit
// of the unmodified dependencies the copies call. The exporter
// (tools/oracle/internal/walletexport, shared with cmd/walletexport) opens
// each snapshot with the unmodified udb.
//
// The wallet session (wallet.open to wallet.close, ops_wallet.go) runs the
// unmodified wallet package over the same database handle, through a
// walletdb interposer that stamps the address rows its unmodified udb
// writes with the injected clock instead of time.Now().
//
// # Every nondeterminism source the operations reach
//
//	draw                                   site (wallet/udb unless named)      injected by
//	XChaCha20 nonce in seal                addressmanager.go:173               seeded stream
//	BLAKE2b passphrase-hasher keys         addressmanager.go:512, 1080, 1736,  seeded stream
//	  (memory only, but they consume       2783
//	  the stream)
//	Argon2id salt                          addressmanager.go:1745, 2196 via    seeded stream
//	                                       kdf.NewArgon2idParams(rand.Reader())
//	Argon2id thread count                  kdf/kdf.go:31 runtime.NumCPU()      seam.NumCPU (setup.ncpu)
//	secretbox nonce, crypto keys,          wallet/internal/snacl/snacl.go:38,  seeded stream
//	  scrypt salt                          77, 237
//	random UTXO seek and pick              txmined.go:2362, 2386, 2410         seeded stream, restarted
//	                                                                           under the operation's
//	                                                                           "rand" key when given
//	shuffle of remaining keys              txmined.go:3169                     the same
//	address add time, manager creation     addressdb.go:1360, 1375, 1390, 1683 seam.Now (setup.clock)
//	transaction store creation date        txdb.go:2181                        seam.Now
//	address add time written in a wallet   addressdb.go:1360 of the unmodified the session's interposer
//	  session (Wallet.nextAddress through    udb                              (seamDB) rewrites it to
//	  SyncAccountToAddrIndex)                                                 seam.Now in the stored row
//	passphrase-hasher keys of the wallet   the unmodified udb's Open          not injected: memory only,
//	  session's Manager                                                       never in a result
//	error separator                        errors/errors.go:27 (":\n\t")      the daemon's ":: "
//	received time a Go caller passes       callers' time.Now()                 tx.build: "received" or
//	  (NewTxRecordFromMsgTx)                                                   the injected clock
//	map order of returned maps             AccountBalances, Rollback, the four  sorted (see the ops)
//	                                       policy maps, GetVSPTicketsByFeeStatus
//	dependency order with map-order ties   UnminedTxs (kahnsort.go)            sorted by hash, plus a
//	                                                                           dependency_order verdict
//	heap address in an error text          vsp.go:203 ("%v" of &id)            "<pointer>"
//
// No other source is reached: hdkeychain.GenerateSeed (operating system
// randomness) is never called, since scripts supply the seed; of the wallet
// package only address issuance is in the vocabulary, and it draws nothing
// (its draws, coin selection shuffles, change position and jitter, are
// outside it). The stream and the seam are process-global; the
// runner executes one script at a time on one goroutine, so the order of
// draws is the order of the operations.
//
// # Coverage
//
// Every exported function of wallet/udb and every exported method of
// Manager, Store and the types they return that reads or changes state,
// at the pin (TestVocabularyCoversEveryEntryPoint, coverage_test.go, finds
// every exported package function and *Manager and *Store method of the
// pinned source named in an operation's calls, or listed there with the
// reason it is not):
//
//	Go entry point                         operation
//	Initialize, InitializeWatchOnly, Open  setup (seed or xpub); Open again in rt.reopen
//	Upgrade                                inside Initialize and InitializeWatchOnly
//	                                       (all 25 steps from the initial version)
//	NeedsMigration                         udb.needs_migration
//	Manager.Close                          rt.reopen
//	Manager.WatchingOnly, IsLocked, Lock   mgr.watching_only, mgr.is_locked, mgr.lock
//	Manager.Unlock, UnlockedWithPassphrase mgr.unlock, mgr.unlocked_with_passphrase
//	Manager.UnlockAccount, LockAccount     mgr.unlock_account, mgr.lock_account
//	Manager.SetAccountPassphrase           mgr.set_account_passphrase
//	Manager.AccountHasPassphrase           mgr.account_has_passphrase
//	Manager.ChangePassphrase               mgr.change_passphrase
//	Manager.ConvertToWatchingOnly          mgr.convert_to_watching_only
//	Manager.AccountProperties              mgr.account_properties
//	Manager.AccountExtendedPubKey,         mgr.account_extended_pub_key,
//	  AccountExtendedPrivKey,                mgr.account_extended_priv_key,
//	  AccountBranchExtendedPubKey            mgr.account_branch_extended_pub_key
//	Manager.CoinTypePrivKey, CoinType      mgr.coin_type_priv_key, mgr.coin_type
//	Manager.UpgradeToSLIP0044CoinType      mgr.upgrade_to_slip0044_coin_type
//	Manager.Address, AddrAccount,          mgr.address, mgr.addr_account,
//	  ExistsHash160                          mgr.exists_hash160
//	Manager.ImportPrivateKey,              mgr.import_private_key, mgr.import_public_key,
//	  ImportPublicKey, ImportScript,         mgr.import_script, mgr.import_xpub_account,
//	  ImportXpubAccount, ImportVotingAccount mgr.import_voting_account
//	Manager.LookupAccount, AccountName,    mgr.lookup_account, mgr.account_name,
//	  NewAccount, RenameAccount,             mgr.new_account, mgr.rename_account,
//	  ForEachAccount, LastAccount,           mgr.for_each_account, mgr.last_account,
//	  LastImportedAccount                    mgr.last_imported_account
//	ValidateAccountName                    udb.validate_account_name
//	Manager.MarkUsed, MarkUsedChildIndex,  mgr.mark_used, mgr.mark_used_child_index,
//	  MarkReturnedChildIndex                 mgr.mark_returned_child_index
//	Manager.SyncAccountToAddrIndex,        mgr.sync_account_to_addr_index,
//	  RecordDerivedAddress                   mgr.record_derived_address
//	Manager.ForEachAccountAddress,         mgr.for_each_account_address,
//	  ForEachActiveAccountAddress,           mgr.for_each_active_account_address,
//	  ForEachActiveAddress                   mgr.for_each_active_address
//	Manager.PrivateKey, HavePrivateKey,    mgr.private_key, mgr.have_private_key,
//	  RedeemScript                           mgr.redeem_script
//	Manager.Encrypt, Decrypt, ChainParams  mgr.encrypt, mgr.decrypt, mgr.chain_params
//	ManagedAddress, ManagedPubKeyAddress,  every address result (Account, Address,
//	  ManagedScriptAddress methods           AddrHash, Imported, Internal, Multisig,
//	                                         PubKey, Index, RedeemScript)
//	CoinTypes, HDKeysFromSeed,             udb.coin_types, udb.hd_keys_from_seed,
//	  IsImportedVoting                       udb.is_imported_voting
//	NewTxRecordFromMsgTx, NewTxRecord      tx.build, tx.raw
//	Store.MainChainTip, ExtendMainChain    store.main_chain_tip, store.extend_main_chain
//	Store.ProcessedTxsBlockMarker,         store.processed_txs_block_marker,
//	  UpdateProcessedTxsBlockMarker          store.update_processed_txs_block_marker
//	SetBirthState, BirthState              udb.set_birth_state, udb.birth_state
//	Store.IsMissingMainChainCFilters,      store.is_missing_main_chain_cfilters,
//	  MissingCFiltersHeight,                 store.missing_cfilters_height,
//	  InsertMissingCFilters,                 store.insert_missing_cfilters,
//	  GetMainChainCFilters, CFilterV2,       store.get_main_chain_cfilters, store.cfilter_v2,
//	  ImportCFiltersV2                       store.import_cfilters_v2
//	ExtractBlockHeaderParentHash, -Height, udb.extract_block_header
//	  -Time, RawBlockHeader.Height
//	Store.GetMainChainBlockHashForHeight,  store.get_main_chain_block_hash_for_height,
//	  GetSerializedBlockHeader,              store.get_serialized_block_header,
//	  GetBlockHeaderTime, GetBlockHeader,    store.get_block_header_time, store.get_block_header,
//	  BlockInMainChain, GetBlockMetaForHash, store.block_in_main_chain, store.get_block_meta_for_hash,
//	  GetMainChainBlockHashes, RangeBlocks   store.get_main_chain_block_hashes, store.range_blocks
//	Store.InsertMinedTx, InsertMemPoolTx,  store.insert_mined_tx, store.insert_mem_pool_tx,
//	  AddCredit, AddTicketCommitment,        store.add_credit, store.add_ticket_commitment,
//	  RedeemTicketCommitments,               store.redeem_ticket_commitments,
//	  AddMultisigOut, SpendMultisigOut       store.add_multisig_out, store.spend_multisig_out
//	Store.SetPublished, RemoveUnconfirmed, store.set_published, store.remove_unconfirmed,
//	  PruneUnmined, Rollback                 store.prune_unmined, store.rollback
//	Store.UnspentOutputCount, RandomUTXO,  store.unspent_output_count, store.random_utxo,
//	  UnspentOutputs, UnspentOutput,         store.unspent_outputs, store.unspent_output,
//	  ForEachUnspentOutpoint,                store.for_each_unspent_outpoint,
//	  IsUnspentOutpoint, ExistsUTXO          store.is_unspent_outpoint, store.exists_utxo
//	Store.MakeInputSource,                 store.select_inputs
//	  InputSource.SelectInputs
//	Store.UnspentTickets, OwnTicket,       store.unspent_tickets, store.own_ticket,
//	  IterateTickets, TicketIterator.Next,   store.iterate_tickets
//	  Err, Close
//	Store.GetMultisigOutput,               store.get_multisig_output,
//	  UnspentMultisigCreditsForAddress       store.unspent_multisig_credits_for_address
//	Store.AccountBalance, AccountBalances  store.account_balance, store.account_balances
//	Store.TxDetails, UniqueTxDetails,      store.tx_details, store.unique_tx_details,
//	  TicketDetails, TxDetails.Height        store.ticket_details (TxDetails first)
//	Store.Tx, ExistsTx,                    store.tx, store.exists_tx,
//	  ExistsTxMinedOrUnmined, TxBlockHeight  store.exists_tx_mined_or_unmined, store.tx_block_height
//	Store.RangeTransactions,               store.range_transactions,
//	  PreviousPkScripts, Spender             store.previous_pk_scripts, store.spender
//	Store.UnminedTxs, UnminedTxHashes      store.unmined_txs, store.unmined_tx_hashes
//	SetVSPTicket, DeleteVSPTicket,         udb.set_vsp_ticket, udb.delete_vsp_ticket,
//	  GetVSPTicket, GetVSPTicketsByFeeStatus udb.get_vsp_ticket, udb.get_vsp_tickets_by_fee_status
//	SetVSPHost, GetVSPHost,                udb.set_vsp_host, udb.get_vsp_host,
//	  SetVSPPubKey, GetVSPPubKey             udb.set_vsp_pub_key, udb.get_vsp_pub_key
//	SetTSpendPolicy, SetVSPTSpendPolicy,   udb.set_tspend_policy, udb.set_vsp_tspend_policy,
//	  TSpendPolicy, VSPTSpendPolicy,         udb.tspend_policy, udb.vsp_tspend_policy,
//	  TSpendPolicies, VSPTSpendPolicies      udb.tspend_policies, udb.vsp_tspend_policies
//	SetTreasuryKeyPolicy,                  udb.set_treasury_key_policy,
//	  SetVSPTreasuryKeyPolicy,               udb.set_vsp_treasury_key_policy,
//	  TreasuryKeyPolicy, VSPTreasuryKeyPolicy, udb.treasury_key_policy, udb.vsp_treasury_key_policy,
//	  TreasuryKeyPolicies,                   udb.treasury_key_policies,
//	  VSPTreasuryKeyPolicies                 udb.vsp_treasury_key_policies
//	SetDefaultAgendaPreference,            udb.set_default_agenda_preference,
//	  SetTicketAgendaPreference,             udb.set_ticket_agenda_preference,
//	  DefaultAgendaPreference,               udb.default_agenda_preference,
//	  TicketAgendaPreference                 udb.ticket_agenda_preference
//
// Of dcrwallet's wallet package, the address issuance udb does not have
// (the gap-limit policy, wallet/addresses.go:505-649), with no network
// backend:
//
//	wallet.OpenDB, wallet.Open             wallet.open (gap_limit)
//	Wallet.NewExternalAddress,             wallet.new_external_address,
//	  NewInternalAddress with                wallet.new_internal_address (policy)
//	  WithGapPolicyError, -Ignore, -Wrap
//	Wallet.BIP0044BranchNextIndexes        wallet.branch_next_indexes
//
// Not covered, and why:
//
//	Migrate             converts a pre-unified (two-file) wallet; no such fixture
//	                    exists, and dcr-purse reads no dcrwallet database
//	                    (ADR-0013)
//	UseLogger           logging only
//	the rest of the     transaction creation, ticket purchase, discovery: they draw
//	  wallet package    randomness (wallet/createtx.go, txauthor) or need a chain
//	                    backend, and the wallet package links the unmodified udb, so
//	                    those draws would not come from the seeded stream; spec 23's
//	                    wallet.script is a separate runner
//	chain-backend states revocations mined by a real chain, tspend and treasury-base
//	                    transactions, a missing-cfilters state (only upgrades from
//	                    old fixtures produce one): a later simnet harness
//
// Go panics the corpus and the generator reach (recorded with kind
// "Panic"; ADR-0012 (D10) has dcr-purse return an error; the first two are
// docs/QUIRKS.md QK-0086, the third QK-0270):
//
//	Store.Rollback to a height above tip+1   txmined.go:2232-2233,
//	                                         extractRawBlockRecordHash of a
//	                                         missing record
//	Store.AddCredit of a regular transaction txmined.go:1394-1396,
//	  with a BlockMeta whose header the      extractBlockHeaderHeight
//	  store does not hold                    (txmined.go:570) of a nil header
//	Wallet.nextAddress with gap limit 0 and  wallet/addresses.go:584,
//	  the ignore policy                      cursor % gapLimit
package main

import (
	"encoding/json"
	"sort"

	"github.com/jzbz/dcr-purse/tools/oracle/internal/linejson"
)

func main() {
	linejson.Serve(map[string]linejson.Handler{
		"run":        cmdRun,
		"vocabulary": cmdVocabulary,
	})
}

func cmdVocabulary(json.RawMessage) (any, error) {
	kinds := []string{"none", "read", "write", "wallet"}
	names := make([]string, 0, len(ops))
	for name := range ops {
		names = append(names, name)
	}
	sort.Strings(names)
	out := []any{
		map[string]any{"op": "update", "tx": "write", "calls": "(a group in one walletdb.Update)"},
		map[string]any{"op": "view", "tx": "read", "calls": "(a group in one walletdb.View)"},
		map[string]any{"op": "rt.checkpoint", "tx": "none", "calls": "walletdb.DB.Copy, then the logical export"},
		map[string]any{"op": "rt.reopen", "tx": "none", "calls": "Manager.Close, walletdb.Open, Open"},
		map[string]any{"op": "wallet.open", "tx": "wallet", "calls": "wallet.OpenDB, wallet.Open, then Manager.Close of the copied udb"},
		map[string]any{"op": "wallet.close", "tx": "wallet", "calls": "Open of the copied udb"},
	}
	for _, name := range names {
		def := ops[name]
		out = append(out, map[string]any{"op": name, "tx": kinds[def.kind], "calls": def.calls})
	}
	return map[string]any{"format": scriptFormat, "ops": out}, nil
}
