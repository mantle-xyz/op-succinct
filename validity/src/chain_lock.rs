//! [MANTLE] Chain lock lease, its heartbeat, and the main-loop progress watchdog.
//!
//! Only one proposer may send L1 transactions for a chain at a time. The lock is a lease in the
//! `chain_locks` table, owned by one process (`owner_id`) and renewed by a background heartbeat,
//! independent of `LOOP_INTERVAL`. See MANTLE_CHANGES.md §3.10d.
//!
//! - **Acquire** ([`acquire_chain_lease`]) is one atomic statement, retried until the previous
//!   holder's lease lapses.
//! - **Renew** ([`spawn_chain_lease_heartbeat`]) only extends a lease this process still holds.
//!   "Not the holder" marks the lease lost immediately.
//! - **Send guard** ([`ChainLease::ensure_held`]) is checked before every L1 transaction. It is not
//!   just a flag kept by the heartbeat: it compares a conservative local deadline on both the
//!   monotonic and the wall clock, so a process that was paused (or whose VM was suspended —
//!   Linux's monotonic clock stops then) refuses to send even if the heartbeat has not run yet.
//! - **Progress watchdog** ([`spawn_progress_watchdog`]): the heartbeat keeps renewing while the
//!   process is alive, so a wedged main loop would hold the lock forever. When the loop has
//!   completed no iteration for longer than any iteration can take *and* no L1 send is in flight,
//!   the watchdog marks the lease lost, which stops renewal and exits the process for the
//!   supervisor to restart. It never cuts a send short: a broadcast may already have reached the
//!   node.

use std::{
    sync::{atomic::Ordering, Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Result};
use op_succinct_host_utils::metrics::MetricsGauge;
use tokio::{sync::watch, task::JoinHandle};
use tracing::{error, info, warn};

use crate::{DriverDBClient, ValidityGauge};

/// Lease length. Long enough to ride out a few failed renewals (database blips, scheduling
/// pauses) at [`CHAIN_LOCK_RENEW_INTERVAL`]; short enough that a crashed proposer's successor
/// takes over within about two minutes.
pub const CHAIN_LOCK_LEASE: Duration = Duration::from_secs(120);
/// How often the heartbeat renews.
pub const CHAIN_LOCK_RENEW_INTERVAL: Duration = Duration::from_secs(30);
/// Bound on each lock statement.
pub const CHAIN_LOCK_DB_TIMEOUT: Duration = Duration::from_secs(10);
/// Taken off the lease when computing the local deadline, so this process stops sending before
/// the database could consider the lease lapsed.
pub const CHAIN_LOCK_SAFETY_MARGIN: Duration = Duration::from_secs(15);
/// How often a starting proposer retries a held lock.
pub const CHAIN_LOCK_ACQUIRE_RETRY: Duration = Duration::from_secs(10);
/// How often the progress watchdog looks.
pub const PROGRESS_WATCHDOG_INTERVAL: Duration = Duration::from_secs(30);

/// A point in time on both clocks. Captured *before* a lock statement is sent, so the lease the
/// database grants (`NOW() + lease`, with `NOW()` no earlier than this) ends no earlier than
/// `at + lease` — the local deadline errs early, never late.
#[derive(Debug, Clone, Copy)]
struct Stamp {
    mono: Instant,
    wall: SystemTime,
}

impl Stamp {
    fn now() -> Self {
        Self { mono: Instant::now(), wall: SystemTime::now() }
    }

    fn plus(self, d: Duration) -> Self {
        Self { mono: self.mono + d, wall: self.wall + d }
    }

    /// True once *either* clock has reached this point.
    fn reached(self, now: Stamp) -> bool {
        now.mono >= self.mono || now.wall >= self.wall
    }
}

struct LeaseInner {
    owner_id: String,
    l1_chain_id: i64,
    l2_chain_id: i64,
    lease: Duration,
    /// Local deadline: the last successful acquire/renew stamp + lease − safety margin.
    valid_until: Mutex<Stamp>,
    /// `Some(reason)` once the lease is lost. Never goes back to `None`.
    lost: watch::Sender<Option<String>>,
}

/// This process's hold on the chain lock. Cheap to clone.
#[derive(Clone)]
pub struct ChainLease {
    inner: Arc<LeaseInner>,
}

impl std::fmt::Debug for ChainLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainLease")
            .field("owner_id", &self.inner.owner_id)
            .field("lost", &*self.inner.lost.borrow())
            .finish()
    }
}

impl ChainLease {
    fn new(
        owner_id: String,
        l1_chain_id: i64,
        l2_chain_id: i64,
        lease: Duration,
        granted: Stamp,
    ) -> Self {
        let (lost, _) = watch::channel(None);
        let lease_handle = Self {
            inner: Arc::new(LeaseInner {
                owner_id,
                l1_chain_id,
                l2_chain_id,
                lease,
                valid_until: Mutex::new(granted),
                lost,
            }),
        };
        lease_handle.extend(granted);
        lease_handle
    }

    pub fn owner_id(&self) -> &str {
        &self.inner.owner_id
    }

    fn deadline_for(&self, granted: Stamp) -> Stamp {
        granted.plus(self.inner.lease.saturating_sub(CHAIN_LOCK_SAFETY_MARGIN))
    }

    /// Records a successful acquire/renew whose statement was sent at `granted`.
    fn extend(&self, granted: Stamp) {
        *self.inner.valid_until.lock().unwrap() = self.deadline_for(granted);
    }

    /// Time left before the local deadline, by the more pessimistic clock.
    pub fn remaining(&self) -> Duration {
        let until = *self.inner.valid_until.lock().unwrap();
        let now = Stamp::now();
        let mono = until.mono.saturating_duration_since(now.mono);
        let wall = until.wall.duration_since(now.wall).unwrap_or_default();
        mono.min(wall)
    }

    /// Marks the lease lost. The first reason wins; later calls are no-ops.
    pub fn mark_lost(&self, reason: impl Into<String>) {
        let reason = reason.into();
        let first = self.inner.lost.send_if_modified(|current| {
            if current.is_none() {
                *current = Some(reason.clone());
                true
            } else {
                false
            }
        });
        if first {
            error!(owner_id = %self.inner.owner_id, %reason, "Chain lock lost; stopping L1 sends");
            ValidityGauge::ChainLockHeld.set(0.0);
        }
    }

    pub fn lost_reason(&self) -> Option<String> {
        self.inner.lost.borrow().clone()
    }

    /// Fails unless this process may send an L1 transaction now: the lease has not been marked
    /// lost, and neither clock has passed the local deadline. Call it immediately before every
    /// L1 send. Passing the deadline marks the lease lost on the spot.
    pub fn ensure_held(&self) -> Result<()> {
        self.ensure_held_at(Stamp::now())
    }

    fn ensure_held_at(&self, now: Stamp) -> Result<()> {
        if let Some(reason) = self.lost_reason() {
            bail!("chain lock not held ({reason}); refusing to send an L1 transaction");
        }
        let until = *self.inner.valid_until.lock().unwrap();
        if until.reached(now) {
            self.mark_lost("lease reached its local deadline without a successful renewal");
            bail!("chain lock lease expired locally; refusing to send an L1 transaction");
        }
        Ok(())
    }

    /// Resolves once the lease is lost, with the reason.
    pub async fn lost(&self) -> String {
        let mut rx = self.inner.lost.subscribe();
        loop {
            if let Some(reason) = rx.borrow_and_update().clone() {
                return reason;
            }
            if rx.changed().await.is_err() {
                // Sender lives in `self`; unreachable while we hold a clone.
                std::future::pending::<()>().await;
            }
        }
    }

    /// Publishes the lease gauges (held, remaining). The heartbeat calls this every renewal.
    pub fn publish_metrics(&self) {
        let held = self.lost_reason().is_none();
        ValidityGauge::ChainLockHeld.set(if held { 1.0 } else { 0.0 });
        ValidityGauge::ChainLockLeaseRemainingSecs.set(self.remaining().as_secs_f64());
    }
}

/// A process-unique owner id: pod name (`HOSTNAME`), pid and start time. Readable in the
/// `chain_locks` row, which is what an operator looks at when two proposers contend.
fn new_owner_id() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown-host".to_string());
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{host}:{}:{nanos:x}:{seq}", std::process::id())
}

/// Takes the chain lock, waiting as long as another live proposer holds it.
///
/// Retries every [`CHAIN_LOCK_ACQUIRE_RETRY`] with the atomic acquire statement: a holder that is
/// still renewing keeps this process waiting indefinitely, and one that has exited is replaced as
/// soon as its lease lapses — no fixed full-lease wait.
pub async fn acquire_chain_lease(
    db: &DriverDBClient,
    l1_chain_id: i64,
    l2_chain_id: i64,
) -> ChainLease {
    let owner_id = new_owner_id();
    let mut waiting_logged = false;
    loop {
        let sent_at = Stamp::now();
        let attempt = tokio::time::timeout(
            CHAIN_LOCK_DB_TIMEOUT,
            db.try_acquire_chain_lock(l1_chain_id, l2_chain_id, &owner_id, CHAIN_LOCK_LEASE),
        )
        .await;
        match attempt {
            Ok(Ok(true)) => {
                info!(%owner_id, lease_secs = CHAIN_LOCK_LEASE.as_secs(), "Acquired chain lock");
                let lease =
                    ChainLease::new(owner_id, l1_chain_id, l2_chain_id, CHAIN_LOCK_LEASE, sent_at);
                lease.publish_metrics();
                return lease;
            }
            Ok(Ok(false)) => {
                let holder = db.chain_lock_holder(l1_chain_id, l2_chain_id).await.ok().flatten();
                warn!(
                    ?holder,
                    retry_secs = CHAIN_LOCK_ACQUIRE_RETRY.as_secs(),
                    "Chain lock is held by another proposer; waiting for its lease to lapse"
                );
                waiting_logged = true;
            }
            Ok(Err(e)) => warn!(error = ?e, "Chain lock acquire failed; retrying"),
            Err(_) => warn!(
                timeout_secs = CHAIN_LOCK_DB_TIMEOUT.as_secs(),
                "Chain lock acquire timed out; retrying"
            ),
        }
        if waiting_logged {
            ValidityGauge::ChainLockHeld.set(0.0);
        }
        tokio::time::sleep(CHAIN_LOCK_ACQUIRE_RETRY).await;
    }
}

/// Renews the lease every [`CHAIN_LOCK_RENEW_INTERVAL`] until it is lost.
///
/// - Renewed → the local deadline moves forward.
/// - "Not the holder" (expired, or taken) → lost immediately.
/// - Error / timeout → counted and retried at the next tick; the lease is lost once the local
///   deadline passes without a successful renewal.
/// - Already lost (e.g. by the progress watchdog) → stops renewing.
pub fn spawn_chain_lease_heartbeat(db: Arc<DriverDBClient>, lease: ChainLease) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(CHAIN_LOCK_RENEW_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await; // the first tick is immediate; the lease was just granted
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = lease.lost() => return,
            }
            if lease.ensure_held().is_err() {
                return;
            }
            if renew_once(&db, &lease).await == RenewOutcome::Lost {
                return;
            }
            lease.publish_metrics();
        }
    })
}

/// Outcome of one renewal attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenewOutcome {
    Renewed,
    /// Errored or timed out; retried at the next tick.
    Failed,
    /// No longer the holder; the lease has been marked lost.
    Lost,
}

/// One renewal: extends the local deadline on success, marks the lease lost at once when the
/// database says this process is no longer the holder.
pub(crate) async fn renew_once(db: &DriverDBClient, lease: &ChainLease) -> RenewOutcome {
    let sent_at = Stamp::now();
    let renewed = tokio::time::timeout(
        CHAIN_LOCK_DB_TIMEOUT,
        db.renew_chain_lock(
            lease.inner.l1_chain_id,
            lease.inner.l2_chain_id,
            &lease.inner.owner_id,
            lease.inner.lease,
        ),
    )
    .await;
    match renewed {
        Ok(Ok(true)) => {
            lease.extend(sent_at);
            RenewOutcome::Renewed
        }
        Ok(Ok(false)) => {
            lease.mark_lost(
                "renewal found this process is no longer the holder (lease lapsed or taken by \
                 another proposer)",
            );
            RenewOutcome::Lost
        }
        Ok(Err(e)) => {
            ValidityGauge::ChainLockRenewFailures.increment(1.0);
            warn!(error = ?e, remaining = ?lease.remaining(), "Chain lock renewal failed");
            RenewOutcome::Failed
        }
        Err(_) => {
            ValidityGauge::ChainLockRenewFailures.increment(1.0);
            warn!(remaining = ?lease.remaining(), "Chain lock renewal timed out");
            RenewOutcome::Failed
        }
    }
}

/// Main-loop progress, shared by the proposer (which reports it) and the watchdog.
///
/// One mutex guards all of it, so "start an L1 send" ([`Self::begin_send`]) and "no send is in
/// flight — declare the loop stalled" ([`Self::try_declare_stalled`]) are mutually exclusive:
/// either the send registers first and the watchdog sees it in flight, or the watchdog decides
/// first and the send is refused before anything is signed or broadcast. Once declared stalled,
/// no send starts again in this process.
#[derive(Clone)]
pub struct LoopProgress {
    inner: Arc<Mutex<ProgressState>>,
}

struct ProgressState {
    last_progress: Instant,
    sends_in_flight: usize,
    stalled: bool,
}

impl Default for LoopProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopProgress {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ProgressState {
                last_progress: Instant::now(),
                sends_in_flight: 0,
                stalled: false,
            })),
        }
    }

    /// Records progress: an iteration completed (or the loop just started).
    pub fn mark_progress(&self) {
        self.inner.lock().unwrap().last_progress = Instant::now();
        let unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        ValidityGauge::LoopLastProgressTimestamp.set(unix as f64);
    }

    /// Registers an L1 send as in flight until the returned guard is dropped, or refuses if the
    /// watchdog has already declared the loop stalled (the process is about to exit; starting a
    /// send now could be cut off mid-broadcast). While any send is in flight the watchdog does
    /// not fire: sends are bounded on their own (signer `SEND_TIMEOUT` / `L1_RPC_TIMEOUT`).
    pub fn begin_send(&self) -> Result<SendInFlight> {
        let mut state = self.inner.lock().unwrap();
        if state.stalled {
            bail!("main loop was declared stalled; not starting an L1 send");
        }
        state.sends_in_flight += 1;
        Ok(SendInFlight { inner: self.inner.clone() })
    }

    /// The watchdog's decision, atomic with respect to [`Self::begin_send`]: if no send is in
    /// flight and the loop has been idle for at least `threshold`, marks it stalled and returns
    /// the idle time. Otherwise `None`.
    fn try_declare_stalled(&self, threshold: Duration) -> Option<Duration> {
        let mut state = self.inner.lock().unwrap();
        let idle = state.last_progress.elapsed();
        if watchdog_should_fire(idle, state.sends_in_flight, threshold) {
            state.stalled = true;
            Some(idle)
        } else {
            None
        }
    }
}

/// See [`LoopProgress::begin_send`].
pub struct SendInFlight {
    inner: Arc<Mutex<ProgressState>>,
}

impl Drop for SendInFlight {
    fn drop(&mut self) {
        self.inner.lock().unwrap().sends_in_flight -= 1;
    }
}

/// Whether the watchdog should declare the main loop dead.
fn watchdog_should_fire(idle: Duration, sends_in_flight: usize, threshold: Duration) -> bool {
    sends_in_flight == 0 && idle >= threshold
}

/// Default watchdog threshold: longer than the slowest possible iteration — the loop interval,
/// two worst-case L1 sends (aggregation submission and checkpoint), and a margin for the rest of
/// the iteration's work.
pub fn default_watchdog_threshold(loop_interval_secs: u64, worst_case_send_secs: u64) -> Duration {
    Duration::from_secs(
        loop_interval_secs
            .saturating_add(worst_case_send_secs.saturating_mul(2))
            .saturating_add(600),
    )
}

/// Marks the lease lost — stopping renewal and exiting the process — once the main loop has made
/// no progress for `threshold` while no L1 send is in flight.
pub fn spawn_progress_watchdog(
    progress: LoopProgress,
    lease: ChainLease,
    threshold: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!(threshold_secs = threshold.as_secs(), "Main-loop progress watchdog armed");
        let mut ticker = tokio::time::interval(PROGRESS_WATCHDOG_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                _ = lease.lost() => return,
            }
            // Decided under the same lock `begin_send` takes, so no send can start in between.
            if let Some(idle) = progress.try_declare_stalled(threshold) {
                lease.mark_lost(format!(
                    "progress watchdog: no main-loop iteration completed for {}s (threshold {}s) \
                     and no L1 send in flight",
                    idle.as_secs(),
                    threshold.as_secs()
                ));
                return;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease_granted_at(granted: Stamp) -> ChainLease {
        ChainLease::new("test-owner".into(), 1, 10, CHAIN_LOCK_LEASE, granted)
    }

    #[test]
    fn fresh_lease_is_held() {
        assert!(lease_granted_at(Stamp::now()).ensure_held().is_ok());
    }

    #[test]
    fn lease_past_its_local_deadline_refuses_to_send_and_marks_itself_lost() {
        let lease = lease_granted_at(Stamp::now());
        let later = Stamp::now().plus(CHAIN_LOCK_LEASE);
        assert!(lease.ensure_held_at(later).is_err());
        // Lost for good — even "now" is refused afterwards.
        assert!(lease.ensure_held().is_err());
        assert!(lease.lost_reason().is_some());
    }

    #[test]
    fn either_clock_passing_the_deadline_is_enough() {
        // Monotonic clock stopped (VM suspend), wall clock moved on: still refused.
        let lease = lease_granted_at(Stamp::now());
        let now = Stamp::now();
        let wall_only = Stamp { mono: now.mono, wall: now.wall + CHAIN_LOCK_LEASE };
        assert!(lease.ensure_held_at(wall_only).is_err());

        let lease = lease_granted_at(Stamp::now());
        let now = Stamp::now();
        let mono_only = Stamp { mono: now.mono + CHAIN_LOCK_LEASE, wall: now.wall };
        assert!(lease.ensure_held_at(mono_only).is_err());
    }

    #[test]
    fn deadline_keeps_a_safety_margin() {
        let granted = Stamp::now();
        let lease = lease_granted_at(granted);
        let just_inside =
            granted.plus(CHAIN_LOCK_LEASE - CHAIN_LOCK_SAFETY_MARGIN - Duration::from_secs(1));
        assert!(lease.ensure_held_at(just_inside).is_ok());
        let in_margin = granted.plus(CHAIN_LOCK_LEASE - CHAIN_LOCK_SAFETY_MARGIN);
        assert!(lease.ensure_held_at(in_margin).is_err());
    }

    #[test]
    fn marking_lost_is_immediate_and_sticky() {
        let lease = lease_granted_at(Stamp::now());
        lease.mark_lost("first");
        lease.mark_lost("second");
        assert_eq!(lease.lost_reason().as_deref(), Some("first"));
        assert!(lease.ensure_held().is_err());
    }

    #[tokio::test]
    async fn lost_resolves_with_the_reason() {
        let lease = lease_granted_at(Stamp::now());
        let waiter = tokio::spawn({
            let lease = lease.clone();
            async move { lease.lost().await }
        });
        lease.mark_lost("taken over");
        assert_eq!(waiter.await.unwrap(), "taken over");
    }

    #[test]
    fn watchdog_never_fires_during_a_send() {
        let threshold = Duration::from_secs(100);
        assert!(!watchdog_should_fire(Duration::from_secs(10_000), 1, threshold));
        assert!(!watchdog_should_fire(Duration::from_secs(99), 0, threshold));
        assert!(watchdog_should_fire(Duration::from_secs(100), 0, threshold));
    }

    #[test]
    fn send_guard_tracks_in_flight_sends() {
        let progress = LoopProgress::new();
        {
            let _a = progress.begin_send().unwrap();
            let _b = progress.begin_send().unwrap();
            // Two sends in flight: the watchdog must not fire, however long the loop is idle.
            assert!(progress.try_declare_stalled(Duration::ZERO).is_none());
        }
        // Both guards dropped: now it may.
        assert!(progress.try_declare_stalled(Duration::ZERO).is_some());
    }

    #[test]
    fn no_send_starts_after_the_watchdog_declared_a_stall() {
        let progress = LoopProgress::new();
        assert!(progress.try_declare_stalled(Duration::ZERO).is_some());
        assert!(progress.begin_send().is_err());
        // Progress afterwards does not re-open sending: the process is exiting.
        progress.mark_progress();
        assert!(progress.begin_send().is_err());
    }

    /// The race the single mutex closes: a send starting at the same moment the watchdog
    /// decides. Over many real-thread races, exactly one side may win — either the send started
    /// (and the watchdog stood down) or the watchdog declared the stall (and the send was
    /// refused). Never both.
    #[test]
    fn begin_send_and_stall_declaration_are_mutually_exclusive() {
        use std::{sync::Barrier, thread};

        for _ in 0..2000 {
            let progress = LoopProgress::new();
            let barrier = Arc::new(Barrier::new(2));

            let sender = thread::spawn({
                let progress = progress.clone();
                let barrier = barrier.clone();
                move || {
                    barrier.wait();
                    // Keep the guard alive until the assertion below.
                    progress.begin_send()
                }
            });
            barrier.wait();
            let declared = progress.try_declare_stalled(Duration::ZERO).is_some();
            let send = sender.join().unwrap();

            assert_ne!(
                send.is_ok(),
                declared,
                "a send started and the watchdog declared a stall in the same race"
            );
        }
    }

    #[test]
    fn default_threshold_exceeds_the_slowest_iteration() {
        // 60s loop, 1140s worst-case send (defaults) → 60 + 2280 + 600.
        assert_eq!(default_watchdog_threshold(60, 1140), Duration::from_secs(2940));
    }

    #[test]
    fn owner_ids_are_unique_per_call() {
        assert_ne!(new_owner_id(), new_owner_id());
    }
}
