//! The live half of a collaborative document: a y-protocols sync session over a
//! WebSocket, served in-process by `yrs`.
//!
//! ## Why this is in the binary and not a sidecar
//!
//! The obvious way to get Yjs is Hocuspocus, which is Node. That would mean a
//! second process to deploy and a second store to keep consistent, against a repo
//! whose shape — one Rust binary over one SQLite file — is what `render.yaml` and
//! the Dockerfile actually depend on. `yrs` is the official Rust port of Yjs and
//! is wire-compatible with the browser library, so the sync protocol can be
//! spoken here and the whole question disappears.
//!
//! The protocol itself is implemented directly rather than through `yrs-axum`,
//! which pins `yrs ^0.18` against a current 0.27 — taking it would either freeze
//! the CRDT three years back or put two incompatible `yrs` versions in one tree.
//! What it would have provided is the hundred lines below.
//!
//! ## The wire format
//!
//! lib0 varint framing, one message per WebSocket binary frame:
//!
//! ```text
//! varuint messageType
//!   0 SYNC       varuint syncType
//!                  0 STEP1   varbytes  state vector  → reply STEP2
//!                  1 STEP2   varbytes  update        → apply
//!                  2 UPDATE  varbytes  update        → apply + relay
//!   1 AWARENESS  varbytes  awareness update          → relay verbatim
//!   3 QUERY_AWARENESS                                → peers answer each other
//! ```
//!
//! Awareness is **relayed, never parsed**. The server holds no awareness state
//! because it is not a participant: presence is a fact between the peers, it
//! expires on its own, and a server replica of it could only ever be a stale
//! third opinion.
//!
//! ## Memory is the fast path; SQLite is the slow one
//!
//! This is the constraint that shapes the whole module. Every write in this
//! process runs as one `IMMEDIATE` transaction behind a process-wide mutex, and
//! that serialization *is* the exactly-one-claimant guarantee for the ready
//! queue. Persisting a keystroke would put every claim, transition and heartbeat
//! behind somebody's typing.
//!
//! So: applying an update and fanning it out to the other peers touches no
//! database at all. Updates accumulate in the room and a background task flushes
//! them, merged into one blob, on a timer — and once more when the last peer
//! leaves. What a crash costs is at most [`FLUSH_INTERVAL`] of typing, which is
//! the trade being made deliberately and is why the number is small.

use crate::auth::AuthCtx;
use crate::error::{ApiError, ApiResult};
use crate::ids::token_hash;
use crate::server::AppState;
use crate::store::crdt;
use crate::store::{CollabKind, CollabSession, COMPACT_AFTER_UPDATES, SESSION_TTL_SECONDS};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use yrs::encoding::read::{Cursor, Read as _};
use yrs::encoding::write::Write as _;
use yrs::updates::decoder::Decode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

const MSG_SYNC: u64 = 0;
const MSG_AWARENESS: u64 = 1;
const SYNC_STEP1: u64 = 0;
const SYNC_STEP2: u64 = 1;
const SYNC_UPDATE: u64 = 2;

/// How long the room waits before writing accumulated edits.
///
/// This is exactly the window a crash can lose, so it wants to be small; it is
/// also the rate at which the process-wide write mutex is taken per open
/// document, so it cannot be tiny. Two seconds puts a busy document at one small
/// insert every two seconds, which is far below the noise floor of the claim
/// traffic it shares the mutex with.
const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// How often an open socket re-asks whether its session is still good.
///
/// The window a revoked credential keeps working for. Short enough that
/// revocation means something, long enough that it is one small read per socket
/// per half minute rather than one per keystroke.
const SESSION_RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Ceiling on peers in one room, so a loop cannot exhaust memory by opening
/// sockets. Far above any real editing session.
const MAX_PEERS_PER_ROOM: usize = 64;

/// The largest single message this socket will accept.
///
/// Left at the library default (64 MiB) there was no bound on one write at all,
/// and this is the one place in the store that takes binary blobs from a client
/// — the same reason initiative entries have byte caps: an unbounded upload
/// holds the write mutex that every claim, transition and heartbeat is queued
/// behind. Generous rather than tight, because a first sync legitimately carries
/// a whole document's state and a large paste is not abuse; what it rules out is
/// the frame that is abuse.
pub(crate) const MAX_SYNC_MESSAGE: usize = 8 * 1024 * 1024;

/// The whole fix rests on these being equal, so the compiler holds it rather
/// than a comment. Anything the socket accepts must be storable; when they
/// disagreed, a frame in between was applied and broadcast and then refused by
/// every flush for the life of the room.
const _: () = assert!(MAX_SYNC_MESSAGE == crate::store::MAX_UPDATE_BYTES);

/// How much unwritten work one room may hold before it stops taking more.
///
/// `requeue` keeps a batch the store refused instead of dropping it, which is
/// what stops a failed flush losing somebody's typing — but a store that keeps
/// refusing turns "keep it" into unbounded growth, because peers go on typing
/// into a queue that never drains. Neither losing the work nor running the
/// process out of memory is acceptable, so past this the room stops ACCEPTING
/// writes and holds what it already has: the work that exists is preserved, and
/// the failure is bounded and visible instead of quiet.
const MAX_PENDING_BYTES: usize = 32 * 1024 * 1024;

/// How many flush ticks before a size refusal is retried once.
///
/// Thirty ticks is a minute. The refusal exists so a hopeless compaction is not
/// attempted every two seconds; this exists so a compaction that has BECOME
/// possible — because somebody deleted something — is not refused for ever.
const RE_ARM_ESCAPE_EVERY: u64 = 30;

/// One frame on its way to the other peers: who sent it, and the bytes.
///
/// The sender id is what stops a peer receiving its own update back. Yjs would
/// survive the echo — applying an update twice is a no-op, that is the point of a
/// CRDT — but it doubles fan-out traffic for nothing.
type Frame = (u64, Arc<Vec<u8>>);

/// One open document, shared by everyone editing it.
pub struct Room {
    id: String,
    /// The authoritative replica. A `std::sync::Mutex` and never held across an
    /// `await` — every use below is a short synchronous burst.
    doc: Mutex<Doc>,
    /// Updates applied since the last flush, still only in memory.
    pending: Mutex<Vec<Vec<u8>>>,
    tx: tokio::sync::broadcast::Sender<Frame>,
    peers: AtomicUsize,
    /// Rows in the persisted log, tracked so compaction does not need a query.
    rows: AtomicU64,
    /// Set when the object, or the project above it, has been archived.
    ///
    /// `can_write` on a session is decided once, when the ticket is minted, and
    /// a ticket lives for hours. Archiving refuses every REST write and refuses
    /// to mint a new ticket — but somebody who already had the page open kept a
    /// socket whose `can_write` was decided before the freeze, and their typing
    /// went on being applied and persisted. Archiving is meant to freeze every
    /// write beneath a project, so this is the socket's half of that promise.
    frozen: AtomicBool,
    /// Set when compaction has refused this object for SIZE, cleared by the next
    /// successful append.
    ///
    /// The store can only refuse after it has been handed the blob, and building
    /// that blob is the expensive half: a permanently wedged room would encode
    /// the whole document — measured at 35 MB — on the blocking pool every two
    /// seconds and throw it away. This is the caller-side stop, which ends the
    /// work rather than only the damage.
    escape_refused: AtomicBool,
    /// Set when `pending` has outgrown [`MAX_PENDING_BYTES`] because the store
    /// keeps refusing the flush. Separate from `frozen` on purpose: that one is
    /// recomputed from the archive state, so sharing it would let an unrelated
    /// unarchive clear a backpressure stop that nothing had resolved.
    overloaded: AtomicBool,
    /// Where this replica started: the highest log `seq` at the moment it
    /// replayed. Everything above this arrived afterwards.
    base_seq: AtomicI64,
    /// How many rows this replica has written since. Compaction compares the two
    /// against the log; anything unaccounted for is a row nobody here replayed,
    /// and replacing the log would destroy it.
    own_appends: AtomicI64,
    /// Held for the length of a flush, so only one runs at a time.
    ///
    /// Without it `flush` is not a durability barrier, only a nudge: it takes
    /// `pending` and returns early when it is empty — which is exactly what it
    /// finds when the background flusher took the batch a moment earlier and is
    /// still inside its store write. A caller that awaited `flush` to make an
    /// edit durable before answering a request would be told the work was done
    /// while it was still in flight. Taking this first means "everything queued
    /// before this call is now written" is a promise the function can keep.
    flushing: tokio::sync::Mutex<()>,
}

impl Room {
    /// Apply an update to the replica and remember it for the next flush.
    fn apply(&self, update: &[u8]) -> Result<(), String> {
        let update = Update::decode_v1(update).map_err(|e| e.to_string())?;
        let doc = self.doc.lock().expect("room doc mutex");
        let mut txn = doc.transact_mut();
        txn.apply_update(update).map_err(|e| e.to_string())?;
        drop(txn);
        Ok(())
    }

    fn state_vector(&self) -> Vec<u8> {
        let doc = self.doc.lock().expect("room doc mutex");
        let txn = doc.transact();
        let sv = txn.state_vector();
        yrs::updates::encoder::Encode::encode_v1(&sv)
    }

    fn diff(&self, sv: &StateVector) -> Vec<u8> {
        let doc = self.doc.lock().expect("room doc mutex");
        let txn = doc.transact();
        txn.encode_state_as_update_v1(sv)
    }

    /// The whole document as one update — what a compaction writes.
    fn full_state(&self) -> Vec<u8> {
        self.diff(&StateVector::default())
    }

    /// Run a closure against the replica, then broadcast whatever it changed.
    ///
    /// This is how an agent writes: `f` mutates the `Doc` directly, the resulting
    /// update is fanned out to every connected peer and queued for the next
    /// flush, so a proposal appears in an open browser immediately rather than on
    /// the next reload.
    ///
    /// The state vector is taken BEFORE `f` runs and the diff after, which is
    /// what makes this correct under concurrency: whatever else landed while `f`
    /// was working is simply part of the diff, and Yjs deduplicates it at the
    /// far end.
    pub fn mutate<T>(&self, f: impl FnOnce(&Doc) -> ApiResult<T>) -> ApiResult<T> {
        let (out, update) = {
            let doc = self.doc.lock().expect("room doc mutex");
            let before = doc.transact().state_vector();
            let out = f(&doc)?;
            let update = doc.transact().encode_state_as_update_v1(&before);
            (out, update)
        };
        if !update.is_empty() {
            self.pending
                .lock()
                .expect("pending mutex")
                .push(update.clone());
            // Peer id 0: no socket sent this, so no socket should be skipped when
            // it is relayed.
            let _ = self
                .tx
                .send((0, Arc::new(sync_message(SYNC_UPDATE, &update))));
        }
        Ok(out)
    }

    /// Read the replica without changing it.
    pub fn read<T>(&self, f: impl FnOnce(&Doc) -> T) -> T {
        let doc = self.doc.lock().expect("room doc mutex");
        f(&doc)
    }
}

/// A room held open for one operation, released on drop.
///
/// The drop guard matters: an early return between joining and leaving would
/// leave the peer count permanently above zero, and the flusher would then keep
/// the room — and its whole document — in memory for the life of the process.
pub struct RoomGuard {
    state: Arc<AppState>,
    room: Arc<Room>,
}

impl std::ops::Deref for RoomGuard {
    type Target = Arc<Room>;
    fn deref(&self) -> &Self::Target {
        &self.room
    }
}

impl Drop for RoomGuard {
    fn drop(&mut self) {
        Rooms::leave(&self.state, &self.room);
    }
}

/// Open a document with no WebSocket attached — the path an agent takes.
///
/// It joins the same room a browser would, so an agent and the people editing
/// are looking at ONE replica rather than two that have to be reconciled later.
pub async fn open_room(state: &Arc<AppState>, id: &str) -> ApiResult<RoomGuard> {
    let room = Rooms::join(state, id).await?;
    Ok(RoomGuard {
        state: state.clone(),
        room,
    })
}

/// Every open document in the process.
///
/// A room exists only while somebody is editing: it is created on the first join
/// (hydrated from the update log) and dropped after the last leave (having
/// flushed). Nothing is cached between sessions, which is what keeps this from
/// becoming a second, divergent copy of the store.
#[derive(Default)]
pub struct Rooms {
    map: Mutex<HashMap<String, Arc<Room>>>,
    /// Bumped once per `resync_frozen`, under the map's lock, together with the
    /// snapshot of which rooms that call will publish to.
    ///
    /// A joining room reads the store to decide whether it is frozen, and that
    /// read is a blocking round-trip: an archive can commit during it, and
    /// `resync_frozen` — which releases the lock BEFORE it stores — can publish
    /// `true` that a stale `false` then overwrites, leaving an archived object
    /// writable for the life of the room. The join checks this counter has not
    /// moved while it was reading, under the same lock the publisher takes, and
    /// re-reads if it has.
    freeze_epoch: AtomicU64,
    /// Held for the whole of `resync_frozen`, so two of them cannot read in one
    /// order and publish in the other.
    resyncing: Mutex<()>,
}

impl Rooms {
    /// Re-decide, for every room currently open, whether it may still be written.
    ///
    /// Called after anything that archives or restores — a project or a single
    /// object. It asks the STORE the same question the REST handlers ask
    /// (`ensure_collab_writable`), so there is one predicate rather than a second
    /// copy of the archive rules that could drift from the first. Archiving is
    /// rare and open rooms are few, so a store read each is affordable; doing it
    /// here rather than per frame keeps it off the path every keystroke takes.
    ///
    /// A room whose object has gone is frozen rather than left alone: the safe
    /// reading of "I cannot tell" is "do not write".
    pub fn resync_frozen(state: &Arc<AppState>) {
        // Only one publisher at a time, so publish order is read order.
        //
        // The lock below orders each publish against a join, but two concurrent
        // resyncs — archive one document while another is unarchived — could
        // read in one order and publish in the other, landing the stale answer
        // last. Archives are rare; serialising them costs nothing.
        let _one_at_a_time = state.rooms.resyncing.lock().expect("resync mutex");

        // The snapshot and the bump happen under ONE hold of the map lock.
        //
        // Split across two, the bump landed after the guard was released, so a
        // join that acquired the lock in that gap loaded an unchanged epoch,
        // installed `false`, and was not in the snapshot to be corrected. Taken
        // together, a join installing after the snapshot necessarily locks after
        // the bump, and a join whose epoch post-dates the bump necessarily read
        // after the commit. That is the whole ordering argument, and it only
        // holds while these two stay in the same scope.
        let rooms: Vec<(String, Arc<Room>)> = {
            let map = state.rooms.map.lock().expect("rooms mutex");
            state.rooms.freeze_epoch.fetch_add(1, Ordering::SeqCst);
            map.iter()
                .map(|(id, room)| (id.clone(), room.clone()))
                .collect()
        };
        let decided: Vec<(Arc<Room>, bool)> = rooms
            .into_iter()
            .map(|(id, room)| {
                let frozen = state.store.ensure_collab_writable(&id).is_err();
                (room, frozen)
            })
            .collect();
        // Published under the map lock, so a join cannot check its epoch and then
        // be overtaken between that check and its own store. No second bump: the
        // one taken with the snapshot is what every reader compares against, and
        // a second only halved the join's retry budget.
        let _guard = state.rooms.map.lock().expect("rooms mutex");
        for (room, frozen) in decided {
            room.frozen.store(frozen, Ordering::SeqCst);
        }
    }

    /// Join a document, creating and hydrating the room if this is the first peer.
    ///
    /// The replay is a blocking store read, so it runs on the blocking pool: a
    /// large document must not occupy an async worker thread while it loads.
    async fn join(state: &Arc<AppState>, id: &str) -> ApiResult<Arc<Room>> {
        // Fast path: somebody is already in the room.
        if let Some(room) = state
            .rooms
            .map
            .lock()
            .expect("rooms mutex")
            .get(id)
            .cloned()
        {
            if room.peers.load(Ordering::SeqCst) >= MAX_PEERS_PER_ROOM {
                return Err(too_many_peers(id));
            }
            room.peers.fetch_add(1, Ordering::SeqCst);
            return Ok(room);
        }

        // The load AND the replay both go to the blocking pool.
        //
        // The comment above used to claim this, and only the load was on it: the
        // replay — decoding and applying every row — ran inline on an async
        // worker. With an object allowed to reach 32 MiB that is a multi-second
        // CPU burn on a runtime thread, on a path any joiner can trigger. Doing
        // both in one hop is also one fewer thread handoff than doing them in
        // two.
        let store_id = id.to_string();
        let store = state.clone();
        let (doc, rows, seen_seq) = super::blocking_read(move || {
            let updates = store.store.load_collab_updates(&store_id)?;
            let seen_seq = store.store.max_collab_seq(&store_id)?;
            let doc = Doc::new();
            let rows = updates.len() as u64;
            {
                let mut txn = doc.transact_mut();
                for blob in &updates {
                    // A blob that will not decode is skipped rather than failing
                    // the open. Refusing to serve a document because one
                    // historical row is unreadable would turn a recoverable
                    // problem into a lost document; the rest of the log still
                    // rebuilds most of it, and the next compaction quietly drops
                    // the bad row.
                    match Update::decode_v1(blob) {
                        Ok(u) => {
                            if let Err(e) = txn.apply_update(u) {
                                eprintln!("{store_id}: skipping unapplicable update: {e}");
                            }
                        }
                        Err(e) => eprintln!("{store_id}: skipping undecodable update: {e}"),
                    }
                }
            }
            Ok((doc, rows, seen_seq))
        })
        .await?;

        // Whether this object may be written, asked once as the room opens.
        //
        // NOT assumed writable because minting a ticket is refused for an
        // archived object: a ticket minted BEFORE the freeze is still valid
        // afterwards, and opens the first room for that object with nobody
        // present for `resync_frozen` to have found. That assumption was written
        // here as a comment and was wrong — a stale ticket wrote into an
        // archived project on a running server with it in place.

        let (tx, _) = tokio::sync::broadcast::channel(256);
        let room = Arc::new(Room {
            id: id.to_string(),
            doc: Mutex::new(doc),
            pending: Mutex::new(Vec::new()),
            tx,
            peers: AtomicUsize::new(0),
            rows: AtomicU64::new(rows),
            // Decided before this room is installed, and set under the same lock
            // as the insert; see the loop below.
            frozen: AtomicBool::new(false),
            escape_refused: AtomicBool::new(false),
            overloaded: AtomicBool::new(false),
            base_seq: AtomicI64::new(seen_seq),
            own_appends: AtomicI64::new(0),
            flushing: tokio::sync::Mutex::new(()),
        });

        // Decide whether this object may be written BEFORE installing the room,
        // and let the epoch guard the install itself.
        //
        // The ordering argument: every caller runs `resync_frozen` AFTER its
        // store commit, so a read that missed an archive necessarily happened
        // before that resync's epoch bump — and the bump is published under this
        // same lock. Either the epoch still matches, and no resync has run since
        // our read, or it does not and we read again.
        //
        // Deciding before the insert is what removes the last unserialised
        // store. The previous shape installed the room first and stored the flag
        // afterwards, so a fallback arm that ran out of retries wrote outside the
        // lock — and could freeze a room on a LIVE project for its whole life,
        // with the socket then dropping that person's typing and telling them
        // nothing. Refusing to open is the safe end of that branch; a retry costs
        // a reconnect, a wrong freeze costs somebody their words.
        for attempt in 0..3 {
            let before = state.rooms.freeze_epoch.load(Ordering::SeqCst);
            let recheck_id = id.to_string();
            let recheck = state.clone();
            let frozen_now = super::blocking_read(move || {
                Ok(recheck.store.ensure_collab_writable(&recheck_id).is_err())
            })
            .await?;

            let mut rooms = state.rooms.map.lock().expect("rooms mutex");
            if state.rooms.freeze_epoch.load(Ordering::SeqCst) != before {
                drop(rooms);
                if attempt == 2 {
                    return Err(ApiError::conflict(
                        "conflict.collab_busy",
                        "The project's archive state changed while this document was \
                         being opened. Nothing is wrong with it — open it again."
                            .to_string(),
                    ));
                }
                continue;
            }

            // Two peers can race to open the same object, and the loser must join
            // the winner's room rather than install a second replica of one text.
            let room = match rooms.get(id) {
                Some(existing) => existing.clone(),
                None => {
                    room.frozen.store(frozen_now, Ordering::SeqCst);
                    rooms.insert(id.to_string(), room.clone());
                    spawn_flusher(state.clone(), room.clone());
                    room
                }
            };
            if room.peers.load(Ordering::SeqCst) >= MAX_PEERS_PER_ROOM {
                return Err(too_many_peers(id));
            }
            room.peers.fetch_add(1, Ordering::SeqCst);
            return Ok(room);
        }
        unreachable!("the loop returns or errors on its last attempt")
    }

    /// Leave a room. The last one out takes the room with them.
    fn leave(state: &Arc<AppState>, room: &Arc<Room>) {
        if room.peers.fetch_sub(1, Ordering::SeqCst) == 1 {
            // The flusher notices `peers == 0`, performs a final flush and
            // exits; removing the entry here would let the next joiner build a
            // fresh room from a log that has not been written yet.
            let _ = state;
        }
    }
}

fn too_many_peers(id: &str) -> ApiError {
    crdt::too_many_peers(kind_of(id), id)
}

/// Which kind of object a room id belongs to.
///
/// Only used to phrase an error. An id whose prefix names no kind cannot reach a
/// room at all — `load_collab_updates` 404s on it first — so this fallback is
/// never what a caller ends up reading.
fn kind_of(id: &str) -> CollabKind {
    CollabKind::from_id(id).unwrap_or(CollabKind::Document)
}

/// Write accumulated edits, and compact when the log has grown long enough.
///
/// Runs on the blocking pool: both calls take the process-wide write mutex, and
/// blocking an async worker on it would be the very stall this design avoids.
/// Is this the one failure compaction can actually fix — a log at its size cap?
///
/// Named rather than matched by suffix. `ends_with("_too_large")` also catches
/// the per-UPDATE cap, and while that one happens to be unreachable here (the
/// merged path diverts oversized batches before the store sees them), a gate
/// whose correctness depends on an invariant three functions away is a gate that
/// will be wrong after the next change.
fn wedged(e: &ApiError) -> bool {
    matches!(
        e.body.code.as_str(),
        "validation.document_too_large" | "validation.mindmap_too_large"
    )
}

/// Put a batch the store would not take back on the queue, ahead of whatever
/// arrived while the write was in flight.
///
/// `flush` TAKES `pending` before writing, so a failed write used to end with the
/// work discarded and one line on stderr. Nobody noticed at the time: the room
/// keeps serving its replica from memory, so every peer still saw their words —
/// until the room was dropped and they were simply gone. A project archived
/// within the flush window was enough to do it, and that is not an exotic race:
/// somebody typing when an admin archives is the ordinary case.
///
/// Restoring it in ORDER matters. These updates are older than anything queued
/// since, and while a CRDT converges regardless of the order updates are
/// applied, the log is also replayed to rebuild a room — so keeping it in the
/// order it happened is what stops a compaction reading a partial history.
///
/// A permanently unwritable object retries every tick and never grows, because a
/// room that cannot be written is also frozen; when its last peer leaves it is
/// dropped, which is the right end for a batch with nothing left to be written
/// into.
fn requeue(room: &Arc<Room>, batch: Arc<Vec<u8>>) {
    let batch = Arc::try_unwrap(batch).unwrap_or_else(|shared| (*shared).clone());
    requeue_all(room, std::slice::from_ref(&batch));
}

/// Put several unwritten updates back, oldest first, and apply the same cap.
///
/// The per-row path used to re-insert by hand and so bypassed both
/// [`MAX_PENDING_BYTES`] and `overloaded` — which mattered most in the one case
/// that reaches it permanently: once a log nears [`MAX_OBJECT_BYTES`] every
/// append fails, and an uncapped queue then grows for as long as the room lives.
fn requeue_all(room: &Arc<Room>, rest: &[Vec<u8>]) {
    let mut pending = room.pending.lock().expect("pending mutex");
    for (i, one) in rest.iter().enumerate() {
        pending.insert(i, one.clone());
    }
    let held: usize = pending.iter().map(Vec::len).sum();
    if held > MAX_PENDING_BYTES {
        // Stop taking more rather than lose what is here or grow without end.
        // Cleared when a flush finally succeeds.
        if !room.overloaded.swap(true, Ordering::SeqCst) {
            eprintln!(
                "{}: holding {held} bytes the store will not take; refusing further \
                 writes until it does",
                room.id
            );
        }
    }
}

pub(crate) async fn flush(state: &Arc<AppState>, room: &Arc<Room>, actor: &str) {
    // Serialize with any flush already running, so returning from here means the
    // log has caught up with everything queued before the call.
    let _writing = room.flushing.lock().await;
    let batch = {
        let mut pending = room.pending.lock().expect("pending mutex");
        if pending.is_empty() {
            return;
        }
        std::mem::take(&mut *pending)
    };

    let merged = match yrs::merge_updates_v1(&batch) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{}: cannot merge pending updates: {e}", room.id);
            return;
        }
    };

    // A quiet second failure mode of the same mismatch: every individual update
    // fits a row, but two seconds of them merged together do not. Merging is an
    // optimisation, not a requirement — the log is replayed in order either way
    // — so when the merge does not fit, write the batch as the rows it already
    // is instead of refusing it.
    if merged.len() > crate::store::MAX_UPDATE_BYTES {
        for (i, one) in batch.iter().enumerate() {
            let store = state.clone();
            let id = room.id.clone();
            let writer = actor.to_string();
            let blob = one.clone();
            let rows = tokio::task::spawn_blocking(move || {
                store.store.append_collab_update(&id, &blob, &writer)
            })
            .await;
            match rows {
                Ok(Ok((rows, _seq))) => {
                    room.rows.store(rows as u64, Ordering::SeqCst);
                    room.own_appends.fetch_add(1, Ordering::SeqCst);
                    room.escape_refused.store(false, Ordering::SeqCst);
                }
                Ok(Err(e)) => {
                    eprintln!("{}: flush failed: {}", room.id, e.body.message);
                    requeue_all(room, &batch[i..]);
                    // The same escape the merged path has. Without it an object
                    // that reached its size cap while this room was producing
                    // batches too big to merge wedged permanently — the fourth
                    // road to that failure, and this path simply did not have the
                    // way out the other one had been given.
                    if wedged(&e) && !room.escape_refused.load(Ordering::SeqCst) {
                        compact(state, room, actor).await;
                    }
                    return;
                }
                Err(e) => {
                    eprintln!("{}: flush task failed: {e}", room.id);
                    requeue_all(room, &batch[i..]);
                    return;
                }
            }
        }
        room.overloaded.store(false, Ordering::SeqCst);
        note_size_if_mindmap(state, room).await;
        return;
    }

    // Held so a failed write can put the work back rather than drop it.
    let merged = Arc::new(merged);

    let store = state.clone();
    let id = room.id.clone();
    let writer = actor.to_string();
    let blob = merged.clone();
    let rows =
        tokio::task::spawn_blocking(move || store.store.append_collab_update(&id, &blob, &writer))
            .await;

    let (rows, _seq) = match rows {
        Ok(Ok(pair)) => pair,
        Ok(Err(e)) => {
            eprintln!("{}: flush failed: {}", room.id, e.body.message);
            requeue(room, merged);
            // A log that has reached `MAX_OBJECT_BYTES` refuses every append, and
            // compaction — the only thing that shrinks it — was attempted only
            // AFTER a successful one. So the object wedged permanently for every
            // room that would ever open it, and raising the row cap to match the
            // socket made that four times cheaper to reach. Compacting on the way
            // out gives it the one escape it has.
            //
            // ONLY for that failure. Trying it on any failed flush meant an
            // archived room with a requeued batch ran a whole-document encode and
            // took the store's write mutex every two seconds for ever, against a
            // compaction that is itself refused for being archived — the same
            // spinning this path was added to cure, by a third road.
            // No row-count condition: four 8 MiB frames reach the object cap
            // with FOUR rows, so requiring 256 of them meant the escape was never
            // attempted for exactly the large-paste shape it was written for. The
            // error itself is the signal.
            if wedged(&e) && !room.escape_refused.load(Ordering::SeqCst) {
                compact(state, room, actor).await;
            }
            return;
        }
        Err(e) => {
            eprintln!("{}: flush task failed: {e}", room.id);
            requeue(room, merged);
            return;
        }
    };
    room.rows.store(rows as u64, Ordering::SeqCst);
    room.own_appends.fetch_add(1, Ordering::SeqCst);
    // The store took something, so a previous size refusal may no longer hold.
    room.escape_refused.store(false, Ordering::SeqCst);
    // The store took it, so whatever backpressure was on can come off.
    room.overloaded.store(false, Ordering::SeqCst);
    note_size_if_mindmap(state, room).await;

    if rows >= COMPACT_AFTER_UPDATES {
        compact(state, room, actor).await;
    }
}

/// Replace the log with the replica, when the replica is entitled to.
///
/// Reached from a successful flush that crossed the threshold, and from a FAILED
/// one on a log that has grown too large to append to — that second path is the
/// only way a wedged object ever shrinks again.
async fn compact(state: &Arc<AppState>, room: &Arc<Room>, actor: &str) {
    // The state we pass is the room's replica, which by construction holds
    // every update in the log plus anything that arrived while we were
    // writing — so replacing the log with it cannot lose a concurrent edit.
    let state_blob = room.full_state();
    // Where this replica started, and how much of what followed it wrote.
    let base = room.base_seq.load(Ordering::SeqCst);
    let mine = room.own_appends.load(Ordering::SeqCst);
    let store = state.clone();
    let id = room.id.clone();
    let writer = actor.to_string();
    let done = tokio::task::spawn_blocking(move || {
        store
            .store
            .compact_collab(&id, &state_blob, &writer, base, mine)
    })
    .await;
    match done {
        Ok(Ok(())) => {
            room.rows.store(1, Ordering::SeqCst);
            // The log is now exactly the row this replica just wrote, so the
            // accounting restarts from it.
            room.own_appends.store(1, Ordering::SeqCst);
        }
        Ok(Err(e)) => {
            // Refused because the log has rows this replica never replayed.
            // Read them, apply them, and move the baseline — otherwise the
            // refusal is permanent: `base_seq` never advances, every later
            // flush is refused too, and the log grows without bound past the
            // compaction threshold for the life of the room. That is the
            // failure the refusal exists to prevent, arriving by the other
            // road, and the remedy this error prints was untrue until now.
            eprintln!(
                "document {}: compaction deferred: {}",
                room.id, e.body.message
            );
            // A refusal for SIZE is not something the next tick improves on — the
            // content is simply that large — so stop paying for the encode until
            // an append succeeds, or until the periodic re-arm below.
            //
            // Matched on the CODE. It was matched on the message containing
            // "byte", which happened to be true of the two size refusals and not
            // the row-mismatch one — a property of prose, three functions away,
            // that nothing pinned. Rewording that message would have disabled
            // this silently and permanently.
            if e.body.code == "conflict.collab_compaction_size" {
                room.escape_refused.store(true, Ordering::SeqCst);
            }
            let store = state.clone();
            let id = room.id.clone();
            let from = base;
            let caught_up =
                tokio::task::spawn_blocking(move || store.store.collab_updates_since(&id, from))
                    .await;
            if let Ok(Ok(rows)) = caught_up {
                let mut highest = base;
                for (seq, blob) in &rows {
                    if room.apply(blob).is_err() {
                        eprintln!("{}: skipping an uncatchable row {seq}", room.id);
                    }
                    highest = highest.max(*seq);
                }
                // Everything up to here is now IN the replica, so it is a
                // baseline this room can honestly claim.
                room.base_seq.store(highest, Ordering::SeqCst);
                room.own_appends.store(0, Ordering::SeqCst);
            }
        }
        Err(e) => eprintln!("{}: compaction task failed: {e}", room.id),
    }
}

/// Keep a mindmap's denormalised node count in step with its document.
///
/// The flusher is the ONLY thing that sees an edit made in the browser — a
/// canvas writes over the socket and never touches a REST route — so without
/// this a map grown entirely in the UI reports zero nodes in the list forever.
/// One small UPDATE per flush, and flushes are debounced to a couple of seconds.
async fn note_size_if_mindmap(state: &Arc<AppState>, room: &Arc<Room>) {
    if kind_of(&room.id) != CollabKind::Mindmap {
        return;
    }
    let nodes = room.read(|doc| {
        let (all, _, _) = crate::store::mindmapdoc::snapshot(doc, "");
        all.len() as i64
    });
    let store = state.clone();
    let id = room.id.clone();
    let _ = tokio::task::spawn_blocking(move || store.store.note_mindmap_size(&id, nodes)).await;
}

fn spawn_flusher(state: Arc<AppState>, room: Arc<Room>) {
    tokio::spawn(async move {
        let mut ticks: u64 = 0;
        let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            ticks = ticks.wrapping_add(1);

            // Re-arm the size refusal periodically.
            //
            // Its only other clear is a successful append — which is exactly what
            // a wedged object cannot do, so in the narrow band where the log has
            // less headroom than a small update, the room could never retry even
            // after somebody DELETED enough to make compaction succeed. Measured
            // by a reviewer: dropping three of four 7 MB pastes takes the state
            // from 28 MB to 7 MB, so the escape would have worked. One retry a
            // minute costs an encode a minute in the hopeless case and restores
            // the recoverable one.
            if ticks.is_multiple_of(RE_ARM_ESCAPE_EVERY) {
                room.escape_refused.store(false, Ordering::SeqCst);
            }

            // Re-decide whether this object may be written, as a BACKSTOP.
            //
            // The join decides synchronously, so this is not what closes the
            // window — it is what gives a wrong answer a way back. This flag has
            // been wrong three times, and every one of them stayed wrong for the
            // life of the room, because nothing ever asked again. A read per open
            // room per tick, on the reader pool, buys "wrong for at most one
            // tick" instead.
            let before = state.rooms.freeze_epoch.load(Ordering::SeqCst);
            let ask = state.clone();
            let ask_id = room.id.clone();
            if let Ok(Ok(frozen)) = tokio::task::spawn_blocking(move || {
                ask.store
                    .ensure_collab_writable(&ask_id)
                    .map(|()| false)
                    .or_else(|e| {
                        // A missing object reads as frozen; a failed read is not
                        // evidence of anything and leaves the flag alone.
                        if e.body.code.starts_with("notfound") || e.body.code.ends_with("archived")
                        {
                            Ok(true)
                        } else {
                            Err(e)
                        }
                    })
            })
            .await
            {
                // Published under the same discipline as the join: only if no
                // resync has run since the read. Without this the backstop was
                // itself a way to overwrite a correct freeze with a stale answer
                // — a smaller version of the bug it exists to insure against.
                let guard = state.rooms.map.lock().expect("rooms mutex");
                if state.rooms.freeze_epoch.load(Ordering::SeqCst) == before {
                    room.frozen.store(frozen, Ordering::SeqCst);
                }
                drop(guard);
            }

            flush(&state, &room, "docsync").await;
            if room.peers.load(Ordering::SeqCst) == 0 {
                // Empty the room under the same lock a joiner takes, so nobody
                // can join the room we are about to drop.
                let mut rooms = state.rooms.map.lock().expect("rooms mutex");
                if room.peers.load(Ordering::SeqCst) == 0 {
                    rooms.remove(&room.id);
                    break;
                }
            }
        }
        // One last flush after leaving the map: an edit could have landed between
        // the tick above and the removal.
        flush(&state, &room, "docsync").await;
    });
}

// ---------------------------------------------------------------------------
// The ticket

#[derive(serde::Deserialize)]
pub struct SyncQuery {
    ticket: Option<String>,
}

/// `POST /v1/documents/{id}/session` (read) — mint a ticket for the sync socket.
///
/// A browser `WebSocket` cannot set an `Authorization` header, which is the same
/// limitation that keeps `/board` polling `/v1/events` rather than using SSE.
/// Polling is not an option for a CRDT, so the credential has to ride the
/// handshake — and putting the caller's real `tk_` token in a query string would
/// scatter it through every access log on the path.
///
/// So this mints a `tkd_` ticket instead: one document, expiring, revocable, and
/// carrying **no more** than the token that asked for it — `can_write` is copied
/// from the caller's scopes, so a `read`-only reader joins as a read-only peer.
pub async fn create_document_session(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    mint(state, ctx, id, CollabKind::Document).await
}

/// `POST /v1/mindmaps/{id}/session` (read) — the same ticket, for a map.
pub async fn create_mindmap_session(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AuthCtx>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    mint(state, ctx, id, CollabKind::Mindmap).await
}

/// Mint a ticket for one collaborative object, whichever kind it is.
///
/// `expect` is checked against the id rather than assumed from the route, so a
/// `mm-…` id presented at the documents route is a 404 rather than a ticket for
/// something the caller did not ask for.
async fn mint(
    state: Arc<AppState>,
    ctx: AuthCtx,
    id: String,
    expect: CollabKind,
) -> ApiResult<Json<Value>> {
    ctx.require_scope("read")?;
    let object = state.store.collab_object(&id)?;
    if object.kind != expect {
        return Err(match expect {
            CollabKind::Document => ApiError::not_found("document", &id),
            CollabKind::Mindmap => ApiError::not_found("mindmap", &id),
        });
    }
    ctx.require_project(&object.project)?;

    let can_write = ctx.require_scope("write").is_ok();
    let expires_at = crate::ids::now_ms() + SESSION_TTL_SECONDS * 1000;
    // What collaborators see next to a caret. The person's handle when the
    // credential is bound to one, else the actor string the token carries —
    // never the token or its id.
    let display = ctx.user.clone().unwrap_or_else(|| ctx.actor.clone());

    let (session, token) =
        state
            .store
            .create_collab_session(&crate::store::crdt::NewCollabSession {
                object_id: &id,
                actor: &ctx.actor,
                display: &display,
                user: ctx.user.as_deref(),
                can_write,
                expires_at,
                // So revoking this token reaches the session it just minted.
                minted_by: &ctx.token_id,
            })?;

    let mut body = json!({
        // `object` is the field to read. The kind-specific key beside it stays
        // because a client written against `/documents` reads `document`, and
        // widening this route must not break it.
        "object": id,
        "kind": object.kind.as_str(),
        "session": session.id,
        "token": token,
        "can_write": can_write,
        "display": display,
        // The STORED expiry, which the clamp against the minting token may have
        // brought forward — telling the client the value we asked for rather
        // than the one we kept would have it trust a ticket past its life.
        "expires_at": crate::ids::iso(session.expires_at),
        // Split into the base and the room because that is the shape
        // `y-websocket` takes: it appends `/<room>` and then `?<params>` itself,
        // so handing it a full URL with a query string produces a mangled one.
        "url": "/v1/docsync",
        "room": id,
        "note": "Connect a y-websocket provider with serverUrl=`url`, room=`room`, \
                 and params={ticket}. The ticket reaches this one object and expires.",
    });
    body[object.kind.as_str()] = json!(id);
    Ok(Json(body))
}

/// Resolve a `tkd_` ticket, or say precisely why it will not do.
///
/// The unknown/dead split is the one answer grants use: a typo and an expired
/// link call for different reactions from whoever is looking at the screen.
fn resolve_ticket(
    state: &Arc<AppState>,
    object: &str,
    ticket: Option<&str>,
) -> ApiResult<CollabSession> {
    let kind = kind_of(object);
    let ticket = ticket.ok_or_else(|| crdt::session_missing(kind, object))?;

    let session = state
        .store
        .lookup_collab_session_by_hash(&token_hash(ticket))?
        .ok_or_else(|| crdt::session_invalid(kind))?;

    if session.revoked_at.is_some() || session.expires_at <= crate::ids::now_ms() {
        return Err(crdt::session_expired(kind));
    }
    // A ticket is minted for ONE object. Checking it here rather than trusting
    // the path is the whole point of scoping it — otherwise it would be an
    // ordinary token with a short life.
    if session.object != object {
        return Err(crdt::session_wrong_object(kind, object, &session.object));
    }
    Ok(session)
}

/// `GET /v1/docsync/{id}?ticket=` — the y-protocols sync socket.
///
/// The id is the last path segment because `y-websocket` composes its URL as
/// `serverUrl + "/" + room + "?" + params`; anything after the room comes out in
/// the wrong place.
///
/// Mounted OUTSIDE the bearer middleware, for the reason `/oauth/*` is: it
/// authenticates with a credential of its own, and running it through a
/// middleware that demands a different one would make it unreachable.
pub async fn sync(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SyncQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let session = match resolve_ticket(&state, &id, q.ticket.as_deref()) {
        Ok(s) => s,
        // The handshake has not happened yet, so this answers as ordinary HTTP —
        // which is also what a browser surfaces most usefully on a failed
        // WebSocket connect.
        Err(e) => return e.into_response(),
    };

    let room = match Rooms::join(&state, &id).await {
        Ok(room) => room,
        Err(e) => return e.into_response(),
    };
    // Held by a guard, not left to an explicit `leave` at the end of the
    // callback. An upgrade that never completes drops the callback without
    // running it, and the peer count then stays above zero for ever: the room is
    // never removed from the map and its flusher ticks for the life of the
    // process. The same leak was fixed one function up and left open here.
    let guard = RoomGuard {
        state: state.clone(),
        room,
    };

    ws.max_message_size(MAX_SYNC_MESSAGE)
        .max_frame_size(MAX_SYNC_MESSAGE)
        .on_upgrade(move |socket| async move {
            let room = guard.room.clone();
            session_loop(socket, state.clone(), room, session).await;
            drop(guard);
        })
}

fn message(kind: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.write_var(kind);
    out.write_buf(payload);
    out
}

fn sync_message(sync_kind: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.write_var(MSG_SYNC);
    out.write_var(sync_kind);
    out.write_buf(payload);
    out
}

async fn session_loop(
    socket: WebSocket,
    state: Arc<AppState>,
    room: Arc<Room>,
    session: CollabSession,
) {
    static NEXT_PEER: AtomicU64 = AtomicU64::new(1);
    let me = NEXT_PEER.fetch_add(1, Ordering::SeqCst);

    let (mut sink, mut stream) = socket.split();
    let mut rx = room.tx.subscribe();

    // Open with SyncStep1 — our state vector — which is what a y-websocket client
    // expects and what makes the first exchange symmetric: it replies with what we
    // are missing and asks for what it is missing.
    if sink
        .send(Message::Binary(
            sync_message(SYNC_STEP1, &room.state_vector()).into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    // A session is checked once, at the handshake, and then lives for hours. So
    // revoking the token it came from has to reach a socket that is ALREADY open,
    // or revocation only stops the next connection and the leaked one keeps
    // writing. Re-asking on a timer rather than per frame keeps a database read
    // off the path every keystroke takes; the cost is that the window is this
    // interval rather than nothing, which is the trade the flush interval makes
    // too.
    let mut recheck = tokio::time::interval(SESSION_RECHECK_INTERVAL);
    recheck.tick().await; // the first tick completes immediately

    loop {
        tokio::select! {
            _ = recheck.tick() => {
                let store = state.clone();
                let sid = session.id.clone();
                let still_valid = tokio::task::spawn_blocking(move || {
                    store.store.collab_session_is_live(&sid)
                })
                .await;
                // A read that FAILED is not proof of revocation — a busy database
                // must not sign everybody out — so only a definite "no" closes it.
                if matches!(still_valid, Ok(Ok(false))) {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            }

            // Fan-out from other peers.
            frame = rx.recv() => {
                match frame {
                    Ok((from, bytes)) => {
                        if from == me {
                            continue;
                        }
                        if sink.send(Message::Binary(bytes.as_ref().clone().into())).await.is_err() {
                            break;
                        }
                    }
                    // Lagged: this peer fell behind the broadcast buffer. Yjs
                    // recovers from a gap by design — resend our state vector and
                    // let it ask for whatever it missed. This is exactly why the
                    // protocol has a step 1.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if sink
                            .send(Message::Binary(sync_message(SYNC_STEP1, &room.state_vector()).into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }

            incoming = stream.next() => {
                let Some(Ok(msg)) = incoming else { break };
                let bytes = match msg {
                    Message::Binary(b) => b,
                    Message::Close(_) => break,
                    // Ping/pong are handled by axum; text frames are not part of
                    // this protocol and are ignored rather than treated as an
                    // error, because a stray one must not drop a live session.
                    _ => continue,
                };

                if let Some(reply) = handle_frame(&room, &session, me, &bytes) {
                    if sink.send(Message::Binary(reply.into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    // A writer leaving is the cheapest moment to persist: the peer count is about
    // to drop and the flusher may be seconds away from its next tick.
    if session.can_write {
        flush(&state, &room, &session.actor).await;
    }
}

/// Decode one frame, act on it, and return anything owed straight back.
///
/// Returns `None` when there is nothing to reply — a relayed update, an ignored
/// message, or a frame that would not decode. A malformed frame is dropped rather
/// than closing the socket: the peer may simply be a newer client sending a
/// message type this server does not know, and disconnecting them over it would
/// be worse than ignoring it.
fn handle_frame(room: &Room, session: &CollabSession, me: u64, bytes: &[u8]) -> Option<Vec<u8>> {
    let mut dec = Cursor::new(bytes);
    let kind: u64 = dec.read_var().ok()?;

    match kind {
        MSG_SYNC => {
            let sync_kind: u64 = dec.read_var().ok()?;
            match sync_kind {
                SYNC_STEP1 => {
                    let sv_bytes = dec.read_buf().ok()?;
                    let sv = StateVector::decode_v1(sv_bytes).ok()?;
                    Some(sync_message(SYNC_STEP2, &room.diff(&sv)))
                }
                SYNC_STEP2 | SYNC_UPDATE => {
                    let update = dec.read_buf().ok()?;
                    // No size check here. `MAX_SYNC_MESSAGE` and the store's
                    // `MAX_UPDATE_BYTES` are equal, so anything this socket
                    // accepts is storable — which is what removes the class.
                    // They were 8 MiB and 1 MiB, and a frame in between was
                    // accepted, applied and broadcast as confirmed, then refused
                    // by every flush for the life of the room; with a refused
                    // batch kept rather than dropped it merged with everything
                    // typed since, and the map read back EMPTY.
                    //
                    // Refusing the frame here was the first fix and was worse: a
                    // browser keeps its replica across a reconnect and answers
                    // the handshake with the same oversized diff, so the tab
                    // looped and the ordinary edits riding in that diff went with
                    // the paste. Measured both ways.
                    // A read-only peer is silently not applied rather than
                    // disconnected: it can legitimately hold a document open, and
                    // its own editor will simply never see its change confirmed.
                    if !session.can_write
                        || room.frozen.load(Ordering::SeqCst)
                        || room.overloaded.load(Ordering::SeqCst)
                    {
                        return None;
                    }
                    if let Err(e) = room.apply(update) {
                        eprintln!("{}: rejecting update: {e}", room.id);
                        return None;
                    }
                    room.pending
                        .lock()
                        .expect("pending mutex")
                        .push(update.to_vec());
                    // Relay as an UPDATE regardless of which of the two arrived:
                    // step 2 is an answer to *our* step 1 and means nothing to the
                    // other peers, but the bytes it carries are theirs to have.
                    let relay = Arc::new(sync_message(SYNC_UPDATE, update));
                    let _ = room.tx.send((me, relay));
                    None
                }
                _ => None,
            }
        }
        // Presence. Relayed verbatim and never parsed: the server is not a
        // participant, so a replica of who is where could only be a stale third
        // opinion about a fact the peers already hold.
        MSG_AWARENESS => {
            let payload = dec.read_buf().ok()?;
            let _ = room
                .tx
                .send((me, Arc::new(message(MSG_AWARENESS, payload))));
            None
        }
        _ => None,
    }
}
