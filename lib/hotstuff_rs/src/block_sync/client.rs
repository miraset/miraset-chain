/*
    Copyright © 2023, ParallelChain Lab
    Licensed under the Apache License, Version 2.0: http://www.apache.org/licenses/LICENSE-2.0
*/

//! Implements the [`BlockSyncClient`], which helps a replica catch up with the head of the blockchain
//! by requesting blocks from the sync server of another replica.
//!
//! The client is responsible for:
//! 1. Triggering Block Sync when:
//!     1. A replica has not made progress for a configurable amount of time.
//!     2. Or, sees evidence that others are ahead.
//! 2. Managing the list of peers available as sync servers and a blacklist for sync servers that have
//!    provided incorrect information in the past.
//! 3. Selecting a peer to sync with from the list of available peers.
//! 4. Attempting to sync with a given peer.
//!
//! ## Triggering Block Sync
//!
//! HotStuff-rs replicas implement two complementary and configurable block sync trigger mechanisms:
//! 1. Event-based sync trigger: on receiving an [`AdvertisePC`] message with a correct PC from the
//!    future. By how many views the received PC must be ahead of the current view to trigger sync can
//!    be configured by setting `block_sync_trigger_min_view_difference` in the replica
//!    [configuration](crate::replica::Configuration).
//! 2. Timeout-based sync trigger: when no "progress" is made for a sufficiently long time. Here
//!    "progress" is understood as updating the Highest PC stored in the [block tree](BlockTree) - in the
//!    context of the HotStuff SMR, updating the Highest PC means that the validators are achieving
//!    consensus and extending the blockchain. The amount of time without progress or sync attempts
//!    required to trigger sync can be configured by setting `block_sync_trigger_timeout` in the replica
//!    configuration.
//!
//! The two sync trigger mechanims offer fallbacks for different liveness-threatening scenarios that a
//! replica may face:
//! 1. The event-based sync trigger can help a replica catch up with the head of the blockchain in case
//!    there is a quorum of validators known to the replica making progress ahead, but the replica has
//!    fallen behind them in terms of view number.
//! 2. The timeout-based sync trigger can help a replica catch up in case there is either no quorum
//!    ahead (e.g., if others have also fallen out of sync with each other), or the validator set making
//!    progress ahead is unknown to the replica because it doesn't know about the most recent validator
//!    set updates. Note that in the latter case, sync will only be succesful if some of the sync peers
//!    known to the replica are up-to-date with the head of the blockchain. Otherwise, a manual sync
//!    attempt may be required to recover from this situation.
//!
//! ## Block Sync procedure
//!
//! When sync is triggered, the sync client picks a random sync server from its list of available sync
//! servers, and iteratively sends it sync requests and processes the received sync responses until
//! the sync is terminated. Sync can be terminated if either:
//! 1. The sync client reaches timeout waiting for a response,
//! 2. The response contains incorrect blocks,
//! 3. The response contains no blocks.
//!
//! In the first two cases, the sync server may be blacklisted if the above-mentioned behaviour is
//! inconsistent with the server's promise to provide blocks up to a given height, as conveyed
//! through its earlier [`AdvertiseBlock`] message.
//!
//! ## Available sync servers
//!
//! In general, available sync servers are current and potential validators that:
//! 1. Are "in sync" or ahead of the replica in terms of highest committed block height,
//! 2. Notify the replica's sync client about their availability,
//! 3. Have not provided false information to the block sync client within a certain period of time
//!    specified by the blacklist expiry time. By "false information" we mean incorrect blocks or
//!    incorrect highest committed block height (used to determine 1).
//!
//! Keeping track of which peers can be considered available sync servers is done by maintaining a
//! hashmap of available sync servers, and a queue of blacklisted sync servers together with their
//! expiry times.

use std::{
    collections::{HashMap, VecDeque},
    sync::mpsc::Sender,
    time::{Duration, Instant, SystemTime},
};

use ed25519_dalek::VerifyingKey;
use rand::seq::IteratorRandom;

use crate::{
    app::{App, ValidateBlockRequest, ValidateBlockResponse},
    block_sync::messages::{
        AdvertiseBlock, AdvertisePC, BlockSyncAdvertiseMessage, BlockSyncRequest, BlockSyncResponse,
    },
    block_tree::{
        accessors::internal::{BlockTreeError, BlockTreeSingleton},
        invariants::{safe_block, safe_pc},
        pluggables::KVStore,
    },
    events::{EndSyncEvent, Event, InsertBlockEvent, StartSyncEvent},
    hotstuff::types::PhaseCertificate,
    networking::{
        network::{Network, ValidatorSetUpdateHandle},
        receiving::{BlockSyncClientStub, BlockSyncResponseReceiveError},
        sending::SenderHandle,
    },
    types::{
        block::Block,
        data_types::{BlockHeight, ChainID, ViewNumber},
        signed_messages::{Certificate, SignedMessage},
        update_sets::ValidatorSetUpdates,
        validator_set::ValidatorSetUpdatesStatus,
    },
};

pub(crate) struct BlockSyncClient<N: Network> {
    config: BlockSyncClientConfiguration,
    receiver: BlockSyncClientStub,
    sender: SenderHandle<N>,
    validator_set_update_handle: ValidatorSetUpdateHandle<N>,
    block_sync_client_state: BlockSyncClientState,
    event_publisher: Option<Sender<Event>>,
    /// MIRASET PATCH #19: blocks asked for per request (see [`AdaptiveRequestLimit`]).
    request_limit: AdaptiveRequestLimit,
}

impl<N: Network> BlockSyncClient<N> {
    /// Create a new instance of the [BlockSyncClient].
    pub(crate) fn new(
        config: BlockSyncClientConfiguration,
        receiver: BlockSyncClientStub,
        sender: SenderHandle<N>,
        validator_set_update_handle: ValidatorSetUpdateHandle<N>,
        event_publisher: Option<Sender<Event>>,
    ) -> Self {
        Self {
            request_limit: AdaptiveRequestLimit::new(config.request_limit),
            config,
            receiver,
            sender,
            validator_set_update_handle,
            block_sync_client_state: BlockSyncClientState::initialize(),
            event_publisher,
        }
    }

    /// Process a received [`BlockSyncAdvertiseMessage`].
    pub(crate) fn on_receive_msg<K: KVStore>(
        &mut self,
        msg: BlockSyncAdvertiseMessage,
        origin: &VerifyingKey,
        block_tree: &mut BlockTreeSingleton<K>,
        app: &mut impl App<K>,
    ) -> Result<(), BlockSyncClientError> {
        match msg {
            BlockSyncAdvertiseMessage::AdvertiseBlock(advertise_block) => {
                self.on_receive_advertise_block(advertise_block, origin, block_tree)
            }
            BlockSyncAdvertiseMessage::AdvertisePC(advertise_pc) => {
                self.on_receive_advertise_pc(advertise_pc, origin, block_tree, app)
            }
        }
    }

    /// Update the [`BlockSyncClient`]'s internal state, and possibly trigger sync on reaching sync trigger
    /// timeout.
    pub(crate) fn tick<K: KVStore>(
        &mut self,
        block_tree: &mut BlockTreeSingleton<K>,
        app: &mut impl App<K>,
    ) -> Result<(), BlockSyncClientError> {
        // 1. Check if any blacklistings have expired, and if so remove the expired blacklistings
        //    from the blacklist.
        self.block_sync_client_state
            .remove_expired_blacklisted_servers();

        // 2. Update highest_pc_view if needed.
        let highest_pc_view = block_tree.highest_pc()?.view;
        if highest_pc_view > self.block_sync_client_state.highest_pc_view {
            self.block_sync_client_state.highest_pc_view = highest_pc_view;
            self.block_sync_client_state.last_progress_or_sync_time = Instant::now();
        }

        // 3. Check if sync should be triggered due to timeout, and if yes then trigger sync.
        if Instant::now() - self.block_sync_client_state.last_progress_or_sync_time
            >= self.config.block_sync_trigger_timeout
        {
            self.sync(block_tree, app)?;
            self.block_sync_client_state.last_progress_or_sync_time = Instant::now();
        };

        Ok(())
    }

    /// Process an [`AdvertiseBlock`] message. This can lead to registering the sender as an available sync
    /// server and storing information on the server's claimed highest committed block height.
    fn on_receive_advertise_block<K: KVStore>(
        &mut self,
        advertise_block: AdvertiseBlock,
        origin: &VerifyingKey,
        block_tree: &BlockTreeSingleton<K>,
    ) -> Result<(), BlockSyncClientError> {
        // 1. Check if the advertise block message has the correct chain id, and is correctly signed.
        if advertise_block.chain_id != self.config.chain_id || !advertise_block.is_correct(origin) {
            return Ok(());
        }

        // 2. Check if the sender address is a valid sync server address.
        if !is_sync_server_address(origin, block_tree)? {
            return Ok(());
        }

        // 3. Check if the sender is not blacklisted.
        if self
            .block_sync_client_state
            .blacklist_contains_server_address(origin)
        {
            return Ok(());
        }

        // 4. Register the sender as an available sync server, committed to sending blocks at least up to
        // the provided block height (never this replica itself, MIRASET PATCH #20).
        self.block_sync_client_state
            .register_advertised_sync_server(
                &self.config.local_key,
                *origin,
                advertise_block.highest_committed_block_height,
            );

        Ok(())
    }

    /// Process an [`AdvertisePC`] message. This can lead to triggering sync if the criteria for event-based
    /// sync trigger are met.
    fn on_receive_advertise_pc<K: KVStore>(
        &mut self,
        advertise_pc: AdvertisePC,
        origin: &VerifyingKey,
        block_tree: &mut BlockTreeSingleton<K>,
        app: &mut impl App<K>,
    ) -> Result<(), BlockSyncClientError> {
        let highest_view_entered = block_tree.highest_view_entered()?;

        // If the sender is blacklisted, ignore the message.
        if self
            .block_sync_client_state
            .blacklist_contains_server_address(origin)
        {
            return Ok(());
        }

        // If the advertised PC has smaller view number than the highest view entered,
        // then it cannot trigger sync.
        if advertise_pc.highest_pc.view < highest_view_entered {
            return Ok(());
        }

        // If the received PC is correct *and* the difference between its view and the highest view
        // entered is sufficiently big, then trigger sync.
        let view_difference = (advertise_pc.highest_pc.view - highest_view_entered) as u64;
        if view_difference >= self.config.block_sync_trigger_min_view_difference
            && advertise_pc.highest_pc.is_correct(block_tree)?
        {
            self.sync(block_tree, app)?;
            self.block_sync_client_state.last_progress_or_sync_time = Instant::now();
        };

        Ok(())
    }

    /// Blocks `peer` committed to provide beyond `init` through its `AdvertiseBlock` (upstream's
    /// check). MIRASET PATCH #21: 0 for a server no longer registered (upstream unwrapped).
    fn min_blocks_expected(&self, peer: &VerifyingKey, init: BlockHeight) -> u64 {
        self.block_sync_client_state
            .available_sync_servers
            .get(peer)
            .map_or(0, |&advertised| advertised.int().saturating_sub(init.int()))
    }

    /// Apply a sync response's highest PC if it is correct and safe (upstream's per-block step).
    /// MIRASET PATCH #21: validator-set updates this commits are forwarded to the network, as for
    /// a block's justify (upstream dropped them on this path).
    fn apply_highest_pc<K: KVStore>(
        &mut self,
        highest_pc: &PhaseCertificate,
        block_tree: &mut BlockTreeSingleton<K>,
    ) -> Result<(), BlockSyncClientError> {
        if highest_pc.is_correct(block_tree)?
            && safe_pc(highest_pc, block_tree, self.config.chain_id)?
        {
            if let Some(vs_updates) = block_tree.update(highest_pc, &self.event_publisher)? {
                self.validator_set_update_handle
                    .update_validator_set(vs_updates)
            }
        }
        Ok(())
    }

    /// MIRASET PATCH #21: wait until `deadline` for `peer`'s response to the request for blocks
    /// from `start_height`, skipping late responses to earlier requests ([`answers_request`]).
    fn recv_answer(
        &self,
        peer: &VerifyingKey,
        start_height: BlockHeight,
        deadline: Instant,
    ) -> Result<BlockSyncResponse, BlockSyncResponseReceiveError> {
        loop {
            let response = self.receiver.recv_response(*peer, deadline)?;
            if answers_request(&response, start_height) {
                return Ok(response);
            }
            log::info!(
                target: "hotstuff_rs::block_sync",
                "SkipStaleSyncResponse, {}, first height {:?}, blocks {}, expected height {}",
                key_prefix(peer),
                response.blocks.first().map(|block| block.height.int()),
                response.blocks.len(),
                start_height,
            );
        }
    }

    /// Sync with a randomly selected peer.
    fn sync<K: KVStore>(
        &mut self,
        block_tree: &mut BlockTreeSingleton<K>,
        app: &mut impl App<K>,
    ) -> Result<(), BlockSyncClientError> {
        let highest_committed_block_height = block_tree.highest_committed_block_height()?;
        if let Some(peer) = self
            .block_sync_client_state
            .random_sync_server(&highest_committed_block_height)
        {
            self.sync_with(&peer, block_tree, app)
        } else {
            Ok(())
        }
    }

    /// Sync with a given peer. This involves possibly multiple iterations of:
    /// 1. Sending a sync request to the peer for a given number of blocks,
    /// 2. Waiting for a response from the peer,
    /// 3. Processing the response: validating blocks, inserting into the block tree, performing related
    ///    block tree and app state updates.
    ///
    /// As part of this process, a sync peer can be blacklisted if:
    /// 1. It sends an incorrect or unsafe block,
    /// 2. It sends a block that is not validated by the [App],
    /// 3. It fails to provide the minimum number of blocks it committed to providing through [AdvertiseBlock].
    fn sync_with<K: KVStore>(
        &mut self,
        peer: &VerifyingKey,
        block_tree: &mut BlockTreeSingleton<K>,
        app: &mut impl App<K>,
    ) -> Result<(), BlockSyncClientError> {
        Event::StartSync(StartSyncEvent {
            timestamp: SystemTime::now(),
            peer: peer.clone(),
        })
        .publish(&self.event_publisher);

        let mut blocks_synced = 0;
        let init_highest_committed_block_height =
            match block_tree.highest_committed_block_height()? {
                Some(height) => height,
                None => BlockHeight::new(0),
            };

        // MIRASET PATCH #10: requests start at `highest committed + 1` as upstream. If such a
        // response brings nothing new although this session inserted blocks beyond that height
        // (a response that committed nothing, e.g. across view gaps left by timed-out leaders),
        // retry once from after the last inserted block (`ahead`). Upstream ended the session
        // there, and every later session repeated it: the replica never caught up. An `ahead`
        // response that does not extend our blocks (our speculative tip lost a fork) ends the
        // session without blacklisting the server, which did nothing wrong. Every block is
        // still validated before insertion.
        let mut next_height: Option<BlockHeight> = None;
        let mut ahead = false;
        // MIRASET PATCH #21: the session starts after the block of our highest PC, whose
        // ancestors we all hold. Starting at `committed + 1` re-fetched every uncommitted block we
        // held, one large block after another over a lossy link (8.5.3: the same 1.8 MB block in
        // every session, and the session ended before reaching an unknown block). If the server's
        // chain does not extend that block (our PC lost a fork), the session falls back to
        // `committed + 1` once, without blacklisting the server (`from_tip`).
        let mut from_tip = false;
        let highest_pc = block_tree.highest_pc()?;
        if !highest_pc.is_genesis_pc() {
            if let Some(tip) = block_tree.block_height(&highest_pc.block)? {
                if tip > init_highest_committed_block_height {
                    next_height = Some(tip + 1);
                    ahead = true;
                    from_tip = true;
                }
            }
        }
        // MIRASET PATCH #21: consecutive known-block responses that committed nothing.
        let mut known_block_steps: u32 = 0;
        // MIRASET PATCH #18: session outcome log (see `log_sync_end`).
        let session_started = Instant::now();
        // Start height and send time of the last request, for `log_sync_end`.
        let mut last_request = (init_highest_committed_block_height, session_started);

        'requests: loop {
            // MIRASET PATCH #21: the client runs on the consensus thread, and with the longer
            // waits of patch #19 a server answering just in time with valid blocks could hold it
            // for many requests. A session that has run for `session_time_cap` ends before its
            // next request, without blacklisting; the replica syncs again on the next trigger
            // (8.5.3 review; the longest session measured on the WAN devnet took 6.4 s).
            if session_started.elapsed() >= session_time_cap(self.config.response_timeout) {
                log_sync_end(
                    peer,
                    "session time cap",
                    blocks_synced,
                    session_started,
                    last_request.0,
                    last_request.1,
                    self.request_limit.get(),
                );
                Event::EndSync(EndSyncEvent {
                    timestamp: SystemTime::now(),
                    peer: *peer,
                    blocks_synced,
                })
                .publish(&self.event_publisher);
                return Ok(());
            }
            let after_committed = match block_tree.highest_committed_block_height()? {
                Some(height) => height + 1,
                None => BlockHeight::new(0),
            };
            let request = BlockSyncRequest {
                chain_id: self.config.chain_id,
                start_height: match next_height {
                    Some(next) if ahead && next > after_committed => next,
                    _ => after_committed,
                },
                limit: self.request_limit.get(),
            };
            let request_start_height = request.start_height;
            self.sender.send(*peer, request);
            let request_sent = Instant::now();
            last_request = (request_start_height, request_sent);

            match self.recv_answer(
                peer,
                request_start_height,
                Instant::now()
                    + self
                        .request_limit
                        .response_timeout(self.config.response_timeout),
            ) {
                Ok(response) => {
                    let response_last_height = response.blocks.last().map(|block| block.height);
                    let new_blocks: Vec<Block> = response
                        .blocks
                        .into_iter()
                        .skip_while(|block| block_tree.contains(&block.hash))
                        .collect();
                    if new_blocks.is_empty() {
                        // MIRASET PATCH #21: every block of the response is already in our block
                        // tree (inserted, not yet committed). Apply the server's highest PC, which
                        // may commit them, and continue after the last of them. Upstream dropped
                        // the response, PC included, and blacklisted the server: a replica that
                        // held uncommitted blocks never committed them (8.5.3 finding 37). Each
                        // such step starts strictly higher (the response starts at the requested
                        // height, `answers_request`), so the session still ends.
                        // At most `MAX_KNOWN_BLOCK_STEPS` such steps in a row that commit nothing,
                        // so a server answering slowly with known blocks cannot hold the consensus
                        // thread for long (8.5.3 review).
                        if let Some(last) = response_last_height {
                            let committed_before = committed_height(block_tree)?;
                            self.apply_highest_pc(&response.highest_pc, block_tree)?;
                            if committed_height(block_tree)? > committed_before {
                                known_block_steps = 0;
                            } else {
                                known_block_steps += 1;
                            }
                            if known_block_steps <= MAX_KNOWN_BLOCK_STEPS {
                                // A late answer may start below the request: never go back.
                                next_height = Some(if last + 1 > request_start_height {
                                    last + 1
                                } else {
                                    request_start_height
                                });
                                ahead = true;
                                continue;
                            }
                        } else if from_tip && blocks_synced == 0 {
                            // Nothing after the block of our highest PC: retry from the
                            // committed height (patch #10's path).
                            from_tip = false;
                            ahead = false;
                            next_height = None;
                            continue;
                        } else if !ahead && next_height.is_some_and(|next| next > after_committed) {
                            ahead = true;
                            continue;
                        }
                        // Check if the server's commitment to providing blocks at least up to a
                        // given height was observed. If not, blacklist the server, unless this
                        // session made progress (MIRASET PATCH #21, `blacklists`).
                        let progressed = blocks_synced > 0
                            || committed_height(block_tree)? > init_highest_committed_block_height;
                        if blacklists(
                            SessionEnd::NoNewBlocks {
                                below_commitment: blocks_synced
                                    < self.min_blocks_expected(
                                        peer,
                                        init_highest_committed_block_height,
                                    ),
                            },
                            progressed,
                        ) {
                            self.block_sync_client_state.blacklist_sync_server(
                                peer.clone(),
                                self.config.blacklist_expiry_time,
                            )
                        }
                        log_sync_end(
                            peer,
                            "no new blocks",
                            blocks_synced,
                            session_started,
                            request_start_height,
                            request_sent,
                            self.request_limit.get(),
                        );

                        Event::EndSync(EndSyncEvent {
                            timestamp: SystemTime::now(),
                            peer: *peer,
                            blocks_synced,
                        })
                        .publish(&self.event_publisher);
                        return Ok(());
                    }

                    // Only new blocks show that data gets through: a quick answer with blocks we
                    // already hold says nothing about the large blocks after them (8.5.3: limit
                    // and timeout then oscillated and the timeout never grew).
                    self.request_limit.on_response();
                    for block in new_blocks {
                        let correct = block.is_correct(block_tree)?;
                        if !correct || !safe_block(&block, block_tree, self.config.chain_id)? {
                            // A block whose justify we do not hold is not correct either; only a
                            // certificate that fails to verify proves the server wrong.
                            let unknown_justify =
                                !correct && !block_tree.contains(&block.justify.block);
                            if from_tip && blocks_synced == 0 && (correct || unknown_justify) {
                                // The server's chain does not extend the block of our highest
                                // PC: retry from the committed height, without blacklisting.
                                from_tip = false;
                                ahead = false;
                                next_height = None;
                                continue 'requests;
                            }
                            // Blacklist the sync server, unless the block only fails to
                            // extend blocks this session inserted or committed (our speculative
                            // tip lost a fork, patch #10). A block whose certificate does not
                            // verify is never an honest fork (8.5.2 review of patch #10). MIRASET
                            // PATCH #21 bases the exception on progress, not on `ahead`.
                            let progressed = blocks_synced > 0
                                || committed_height(block_tree)?
                                    > init_highest_committed_block_height;
                            let end = if correct {
                                SessionEnd::UnsafeBlock
                            } else {
                                SessionEnd::IncorrectBlock
                            };
                            if blacklists(end, progressed) {
                                self.block_sync_client_state.blacklist_sync_server(
                                    peer.clone(),
                                    self.config.blacklist_expiry_time,
                                );
                            }
                            let reason = if correct {
                                format!(
                                    "unsafe block at height {} (justify view {}, justified block \
                                     known {}, locked view {})",
                                    block.height,
                                    block.justify.view,
                                    block_tree.contains(&block.justify.block),
                                    block_tree.locked_pc()?.view,
                                )
                            } else {
                                format!("incorrect block at height {}", block.height)
                            };
                            log_sync_end(
                                peer,
                                &reason,
                                blocks_synced,
                                session_started,
                                request_start_height,
                                request_sent,
                                self.request_limit.get(),
                            );
                            Event::EndSync(EndSyncEvent {
                                timestamp: SystemTime::now(),
                                peer: *peer,
                                blocks_synced,
                            })
                            .publish(&self.event_publisher);
                            return Ok(());
                        }

                        let parent_block = if block.justify.is_genesis_pc() {
                            None
                        } else {
                            Some(&block.justify.block)
                        };

                        let validate_block_request =
                            ValidateBlockRequest::new(&block, block_tree.app_view(parent_block)?);

                        if let ValidateBlockResponse::Valid {
                            app_state_updates,
                            validator_set_updates,
                        } = app.validate_block_for_sync(validate_block_request)
                        {
                            // 1. Insert the block into the block tree.
                            block_tree.insert(
                                &block,
                                app_state_updates.as_ref(),
                                validator_set_updates.as_ref(),
                            )?;
                            Event::InsertBlock(InsertBlockEvent {
                                timestamp: SystemTime::now(),
                                block: block.clone(),
                            })
                            .publish(&self.event_publisher);

                            // 2. Trigger block tree updates: update highest PC, lock, commit.
                            let committed_validator_set_updates =
                                block_tree.update(&block.justify, &self.event_publisher)?;

                            if let Some(vs_updates) = committed_validator_set_updates {
                                self.validator_set_update_handle
                                    .update_validator_set(vs_updates)
                            }

                            blocks_synced += 1;
                            next_height = Some(block.height + 1);
                        } else {
                            // Blacklist the sync server.
                            self.block_sync_client_state.blacklist_sync_server(
                                peer.clone(),
                                self.config.blacklist_expiry_time,
                            );
                            log_sync_end(
                                peer,
                                "invalid block",
                                blocks_synced,
                                session_started,
                                request_start_height,
                                request_sent,
                                self.request_limit.get(),
                            );
                            Event::EndSync(EndSyncEvent {
                                timestamp: SystemTime::now(),
                                peer: *peer,
                                blocks_synced,
                            })
                            .publish(&self.event_publisher);
                            return Ok(());
                        }

                        self.apply_highest_pc(&response.highest_pc, block_tree)?;
                    }
                    // This response made progress. MIRASET PATCH #10 started the next request
                    // from the committed height again; when the inserted blocks did not commit,
                    // the server then resent them (on a lossy link, up to the 12 MiB byte limit).
                    // MIRASET PATCH #21 continues after the last inserted block instead.
                    ahead = true;
                }
                Err(error) => {
                    // A disconnect returns at once and says nothing about the link's speed.
                    if matches!(error, BlockSyncResponseReceiveError::Timeout) {
                        self.request_limit.on_timeout();
                    }
                    // Check if the server's commitment to providing blocks at least up to a given height was observed.
                    // If not, blacklist the server.
                    let below_commitment = blocks_synced
                        < self.min_blocks_expected(peer, init_highest_committed_block_height);
                    if blacklists(SessionEnd::NoResponse { below_commitment }, false) {
                        self.block_sync_client_state
                            .blacklist_sync_server(peer.clone(), self.config.blacklist_expiry_time)
                    }
                    log_sync_end(
                        peer,
                        "no response",
                        blocks_synced,
                        session_started,
                        request_start_height,
                        request_sent,
                        self.request_limit.get(),
                    );

                    Event::EndSync(EndSyncEvent {
                        timestamp: SystemTime::now(),
                        peer: *peer,
                        blocks_synced,
                    })
                    .publish(&self.event_publisher);
                    return Ok(());
                }
            }
        }
    }
}

/// MIRASET PATCH #21: known-block responses in a row that commit nothing before a session ends.
const MAX_KNOWN_BLOCK_STEPS: u32 = 4;

/// MIRASET PATCH #21: how a sync session ended, for [`blacklists`].
#[derive(Debug, Clone, Copy)]
enum SessionEnd {
    /// A block whose certificate does not verify.
    IncorrectBlock,
    /// A correct block that is not safe to insert (does not extend our lock).
    UnsafeBlock,
    /// A response with no blocks we did not have. `below_commitment`: fewer blocks synced than
    /// the server's `AdvertiseBlock` committed to.
    NoNewBlocks { below_commitment: bool },
    /// No response before the timeout.
    NoResponse { below_commitment: bool },
}

/// MIRASET PATCH #21: whether a session's end blacklists its server. `progressed`: the session
/// inserted or committed blocks. Without progress this is upstream's rule (an unsafe block, or
/// fewer blocks than committed to). With progress, an unsafe block or a short answer means our own
/// speculative blocks lost a fork or the server has nothing beyond them (patch #10), so it is not
/// held against the server. A client-side continuation alone (`ahead`) is no longer enough: a
/// server answering with one known block and then nothing is blacklisted (8.5.3 review).
fn blacklists(end: SessionEnd, progressed: bool) -> bool {
    match end {
        SessionEnd::IncorrectBlock => true,
        SessionEnd::UnsafeBlock => !progressed,
        SessionEnd::NoNewBlocks { below_commitment } => below_commitment && !progressed,
        SessionEnd::NoResponse { below_commitment } => below_commitment,
    }
}

/// Highest committed block height, 0 before the first commit (as upstream's session start).
fn committed_height<K: KVStore>(
    block_tree: &BlockTreeSingleton<K>,
) -> Result<BlockHeight, BlockTreeError> {
    Ok(block_tree
        .highest_committed_block_height()?
        .unwrap_or(BlockHeight::new(0)))
}

/// MIRASET PATCH #21: whether `response` answers a request for blocks from `start_height`. The
/// server sends blocks from exactly the requested height, so a response starting elsewhere answers
/// an earlier request whose wait already timed out. One that starts lower still carries the
/// requested blocks after a known prefix (measured in 8.5.3: a ~4 MB block arrived after its
/// timeout, while the next session asked from the committed height again), so it is accepted. One
/// that starts higher (a late answer to a request past our blocks) cannot be inserted and is
/// skipped. A response without blocks cannot be matched and is accepted.
fn answers_request(response: &BlockSyncResponse, start_height: BlockHeight) -> bool {
    response
        .blocks
        .first()
        .is_none_or(|block| block.height <= start_height)
}

/// First 7 characters of a key's base64 encoding, as in the patch #17/#18 log lines.
pub(crate) fn key_prefix(key: &VerifyingKey) -> String {
    base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD_NO_PAD,
        key.to_bytes(),
    )
    .get(0..7)
    .unwrap_or_default()
    .to_string()
}

/// MIRASET PATCH #18: one `info` line per sync session (target `hotstuff_rs::block_sync`): the
/// server, why the session ended, blocks synced, session time, and the last request's start height
/// and wait. Upstream only published events, so a replica that never caught up under loss could
/// not be diagnosed (8.5.3 finding 37). Logging only.
fn log_sync_end(
    peer: &VerifyingKey,
    reason: &str,
    blocks_synced: u64,
    session_started: Instant,
    last_start_height: BlockHeight,
    last_request_sent: Instant,
    next_request_limit: u32,
) {
    log::info!(
        target: "hotstuff_rs::block_sync",
        "EndSync, {}, {}, blocks {}, session {} ms, last request from height {} waited {} ms, \
         next request limit {}",
        key_prefix(peer),
        reason,
        blocks_synced,
        session_started.elapsed().as_millis(),
        last_start_height,
        last_request_sent.elapsed().as_millis(),
        next_request_limit,
    );
}

/// MIRASET PATCH #19: number of blocks the sync client asks for per request. It halves on every
/// response timeout (down to one block) and doubles on every response with new blocks, up to the
/// configured `request_limit`. Upstream always asked for `request_limit` blocks: with large blocks
/// on a lossy link (about 1 MB/s per TCP stream) such a response (up to the server's byte limit,
/// 12 MiB in miraset) cannot arrive within `response_timeout`, so every request timed out, each
/// server was blacklisted in turn, and a lagging replica never caught up (8.5.3 finding 37). The
/// sync client runs on the consensus thread, so a longer timeout for every request stalls the
/// replica; only a single block that does not arrive in time gets a longer wait: once the limit is
/// one block, every further timeout doubles the response timeout, up to
/// `MAX_RESPONSE_TIMEOUT_FACTOR` times the configured one (one block may hold up to 8 MiB in
/// miraset, about 8 s at 1 MB/s). A response with new blocks restores the configured timeout.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AdaptiveRequestLimit {
    now: u32,
    max: u32,
    timeout_factor: u32,
}

/// MIRASET PATCH #19: longest response wait, as a multiple of the configured `response_timeout`.
pub(crate) const MAX_RESPONSE_TIMEOUT_FACTOR: u32 = 8;

/// MIRASET PATCH #21: after this long a session sends no further request (one longest wait, 12 s
/// by default). With the request in flight a session holds the consensus thread for at most twice
/// that.
pub(crate) fn session_time_cap(configured_response_timeout: Duration) -> Duration {
    configured_response_timeout.saturating_mul(MAX_RESPONSE_TIMEOUT_FACTOR)
}

impl AdaptiveRequestLimit {
    pub(crate) fn new(configured: u32) -> Self {
        let max = configured.max(1);
        Self {
            now: max,
            max,
            timeout_factor: 1,
        }
    }

    pub(crate) fn get(&self) -> u32 {
        self.now
    }

    /// How long to wait for the next response, given the configured `response_timeout`.
    pub(crate) fn response_timeout(&self, configured: Duration) -> Duration {
        configured.saturating_mul(self.timeout_factor)
    }

    pub(crate) fn on_timeout(&mut self) {
        if self.now == 1 {
            self.timeout_factor = (self.timeout_factor * 2).min(MAX_RESPONSE_TIMEOUT_FACTOR);
        }
        self.now = (self.now / 2).max(1);
    }

    pub(crate) fn on_response(&mut self) {
        self.now = self.now.saturating_mul(2).min(self.max);
        self.timeout_factor = 1;
    }
}

/// Configuration parameters that define the behaviour of the [`BlockSyncClient`]. These should not
/// change after the block sync client starts.
pub(crate) struct BlockSyncClientConfiguration {
    /// Chain ID of the target blockchain. The block sync client will only process advertise messages whose
    /// Chain ID matches the configured value.
    pub(crate) chain_id: ChainID,

    /// The maximum number of blocks requested with every block sync request.
    pub(crate) request_limit: u32,

    /// Timeout for waiting for a single block sync response.
    pub(crate) response_timeout: Duration,

    /// Time after which a blacklisted sync server should be removed from the block sync blacklist.
    pub(crate) blacklist_expiry_time: Duration,

    /// By how many views a PC received via [`AdvertisePC`] must be ahead of the current view in order to
    /// trigger sync (via the event-based sync trigger).
    pub(crate) block_sync_trigger_min_view_difference: u64,

    /// How much time needs to pass without any progress (i.e., updating the highest PC) or sync attempts in
    /// order to trigger sync (via the timeout-based sync trigger).
    pub(crate) block_sync_trigger_timeout: Duration,

    /// MIRASET PATCH #20: this replica's own key, never registered as a sync server.
    pub(crate) local_key: VerifyingKey,
}

struct BlockSyncClientState {
    /// Replicas that are currently available to be sync servers,
    available_sync_servers: HashMap<VerifyingKey, BlockHeight>,

    /// A list of replicas (identified by their public addresses) that will be ignored by the block sync
    /// client until the paired instant. "Ignored" here means that:
    /// - These replicas will not be selected as sync servers.
    /// - Advertise messages received from these replicas will be ignored.
    blacklist: VecDeque<(VerifyingKey, Instant)>,

    /// The most recent instant in time when either:
    /// 1. Progress was made (the highest PC was updated).
    /// 2. The block sync procedure was completed (not necessarily successfully).
    last_progress_or_sync_time: Instant,

    /// Cached value of `block_tree.highest_pc().view`.
    highest_pc_view: ViewNumber,
}

impl BlockSyncClientState {
    /// Initialize the internal state of the block sync client.
    fn initialize() -> Self {
        Self {
            available_sync_servers: HashMap::new(),
            blacklist: VecDeque::new(),
            last_progress_or_sync_time: Instant::now(),
            highest_pc_view: ViewNumber::new(0),
        }
    }

    /// Check if a given server address is in the blacklist.
    fn blacklist_contains_server_address(&self, sync_server: &VerifyingKey) -> bool {
        self.blacklist
            .iter()
            .find(|(vk, _)| vk == sync_server)
            .is_some()
    }

    // Register a sync server
    /// MIRASET PATCH #20: register the sender of an `AdvertiseBlock` unless it is this replica.
    /// A broadcast is delivered to its sender too, so upstream registered every replica as a sync
    /// server of itself: a session with itself ended at once with no blocks (`EndSync`, patch #18:
    /// about one session in five), and the next attempt waited for the sync trigger again.
    fn register_advertised_sync_server(
        &mut self,
        local_key: &VerifyingKey,
        sync_server: VerifyingKey,
        highest_committed_block_height: BlockHeight,
    ) {
        if sync_server != *local_key {
            self.register_or_update_sync_server(sync_server, highest_committed_block_height);
        }
    }

    fn register_or_update_sync_server(
        &mut self,
        sync_server: VerifyingKey,
        highest_committed_block_height: BlockHeight,
    ) {
        let _ = self
            .available_sync_servers
            .insert(sync_server, highest_committed_block_height);
    }

    /// Blacklist a given sync server by:
    /// 1. Removing it from available sync servers,
    /// 2. Adding it to the blacklist.
    fn blacklist_sync_server(
        &mut self,
        sync_server: VerifyingKey,
        blacklist_expiry_time: Duration,
    ) {
        // MIRASET PATCH #17: make every blacklisting visible (8.5.3 measures sync liveness under
        // loss; upstream recorded it nowhere). Logging only, no behaviour change.
        log::warn!(
            target: "hotstuff_rs::block_sync",
            "BlacklistSyncServer, {}, {}s",
            base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD_NO_PAD,
                sync_server.to_bytes()
            )
            .get(0..7)
            .unwrap_or_default(),
            blacklist_expiry_time.as_secs_f64()
        );

        // Remove any entry corresponding to this server address from the available servers list.
        let _ = self.available_sync_servers.remove(&sync_server);

        // Push a new blacklist entry to the end of the blacklist queue.
        self.blacklist
            .push_back((sync_server, Instant::now() + blacklist_expiry_time))
    }

    /// Remove all sync servers whose blacklisting has expired from the blacklist.
    ///
    /// MIRASET PATCH #13: pop entries whose expiry has passed (`expiry <= now`). Upstream popped
    /// those still in force (`expiry >= now`), so every blacklisting was lifted on the next tick.
    /// Entries are pushed with a fixed expiry time, so the queue is ordered by expiry.
    fn remove_expired_blacklisted_servers(&mut self) {
        let now = Instant::now();
        while self
            .blacklist
            .front()
            .is_some_and(|(_, expiry)| expiry <= &now)
        {
            let _ = self.blacklist.pop_front();
        }
    }

    /// Select a random sync server from available sync servers, with the condition that the sync server
    /// must have advertised a block higher than `highest_committed_block_height`.
    fn random_sync_server(
        &self,
        min_highest_committed_block_height: &Option<BlockHeight>,
    ) -> Option<VerifyingKey> {
        match min_highest_committed_block_height {
            None => self
                .available_sync_servers
                .keys()
                .choose(&mut rand::thread_rng())
                .copied(),
            Some(min_height) => self
                .available_sync_servers
                .keys()
                .filter(|vk| {
                    self.available_sync_servers
                        .get(vk)
                        .is_some_and(|height| height >= min_height)
                })
                .choose(&mut rand::thread_rng())
                .copied(),
        }
    }
}

/// The block sync client may fail if there is an error when trying to read from or write to the
/// [block tree][BlockTree].
#[derive(Debug)]
pub enum BlockSyncClientError {
    BlockTreeError(BlockTreeError),
}

impl From<BlockTreeError> for BlockSyncClientError {
    fn from(value: BlockTreeError) -> Self {
        BlockSyncClientError::BlockTreeError(value)
    }
}

/// Returns whether a given [verifying key](VerifyingKey) is recognised as a valid sync server address.
///
/// A replica is allowed to act as a sync server if either:
/// 1. It is a member of the current committed validator set, or
/// 2. One of the current speculative blocks proposes to add the replica to the validator set.
///
/// Recognising only committed and candidate validators as potential sync servers is an effective,
/// though rather conservative solution to the problem of sybil attacks.
fn is_sync_server_address<K: KVStore>(
    verifying_key: &VerifyingKey,
    block_tree: &BlockTreeSingleton<K>,
) -> Result<bool, BlockSyncClientError> {
    let committed_validator_set = block_tree.committed_validator_set()?;
    if committed_validator_set.contains(verifying_key) {
        return Ok(true);
    }

    match block_tree.highest_committed_block()? {
        Some(block) => {
            let mut speculative_vs_updates = block_tree
                .blocks_in_branch(block)
                .filter(|block| {
                    block_tree
                        .validator_set_updates_status(block)
                        .is_ok_and(|vsu_status| vsu_status.is_pending())
                })
                .map(|block| {
                    if let Ok(ValidatorSetUpdatesStatus::Pending(vs_updates)) =
                        block_tree.validator_set_updates_status(&block)
                    {
                        vs_updates
                    } else {
                        ValidatorSetUpdates::new()
                    }
                });

            Ok(speculative_vs_updates
                .find(|vs_updates| vs_updates.get_insert(verifying_key).is_some())
                .is_some())
        }
        None => Ok(false),
    }
}

#[cfg(test)]
#[path = "blacklist_tests.rs"]
mod blacklist_tests;

#[cfg(test)]
#[path = "request_limit_tests.rs"]
mod request_limit_tests;

#[cfg(test)]
#[path = "self_sync_tests.rs"]
mod self_sync_tests;

#[cfg(test)]
#[path = "stale_response_tests.rs"]
mod stale_response_tests;

#[cfg(test)]
#[path = "blacklist_decision_tests.rs"]
mod blacklist_decision_tests;
