//! Reuse of the desktop diagnostics report across observation calls.
//!
//! `doctor_report` runs about two dozen probes (busctl, gsettings, window
//! backend probes) on every call. `get_app_state` reports only its compact
//! readiness block, so it reads this cache and probes again only when a call
//! needs a fresh report: `verbose: true`, a failed action, a setup tool, an
//! observation that contradicts the cached report, or an expired TTL.

use crate::diagnostics::{doctor_report, DoctorReport};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

/// Longest time a report is reused without another refresh trigger.
///
/// Failed observations, failed actions, and setup tools already force a
/// refresh, so the TTL bounds staleness only for facts an observation does not
/// exercise, such as input backend availability. Those change when a person
/// installs or starts a backend, so a report at most 30 s old reflects such a
/// change about as soon as the person can act on it, while an agent loop that
/// observes every few seconds still reuses one report across several calls.
///
/// The 30 s are monotonic time. `Instant` reads `CLOCK_MONOTONIC` on Linux, so
/// time spent suspended does not count, and a report from before a suspend
/// stays fresh for whatever remained of its 30 s after resume.
pub(crate) const DIAGNOSTICS_TTL: Duration = Duration::from_secs(30);

pub(crate) struct DiagnosticsCache<T> {
    probe: Arc<dyn Fn() -> T + Send + Sync>,
    ttl: Duration,
    /// Bumped by `invalidate`. A report computed under an older generation is
    /// stale, including one whose probes were still running at the time.
    generation: AtomicU64,
    entry: tokio::sync::Mutex<Option<Entry<T>>>,
}

struct Entry<T> {
    report: T,
    generation: u64,
    computed_at: Instant,
}

impl<T> Entry<T> {
    fn is_fresh(&self, generation: u64, now: Instant, ttl: Duration) -> bool {
        self.generation == generation && now.saturating_duration_since(self.computed_at) < ttl
    }
}

impl Default for DiagnosticsCache<DoctorReport> {
    fn default() -> Self {
        Self::new(Arc::new(doctor_report), DIAGNOSTICS_TTL)
    }
}

impl<T> DiagnosticsCache<T> {
    pub(crate) fn new(probe: Arc<dyn Fn() -> T + Send + Sync>, ttl: Duration) -> Self {
        Self {
            probe,
            ttl,
            generation: AtomicU64::new(0),
            entry: tokio::sync::Mutex::new(None),
        }
    }

    /// Marks the cached report stale, so the next `report` call probes again.
    pub(crate) fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

impl<T: Clone + Send + 'static> DiagnosticsCache<T> {
    /// A copy of the cached report, or a new one from the probe when `refresh`
    /// is set or the cached report is missing, invalidated, or older than the
    /// TTL. A new report replaces the cached one.
    pub(crate) async fn report(&self, refresh: bool) -> T {
        self.report_and_probed(refresh).await.0
    }

    /// Like `report`, and also whether this call ran the probe.
    pub(crate) async fn report_and_probed(&self, refresh: bool) -> (T, bool) {
        // Held across the probes so concurrent callers share one run.
        let mut entry = self.entry.lock().await;
        let generation = self.generation.load(Ordering::SeqCst);
        let now = Instant::now();
        if !refresh {
            if let Some(cached) = entry
                .as_ref()
                .filter(|cached| cached.is_fresh(generation, now, self.ttl))
            {
                return (cached.report.clone(), false);
            }
        }
        let probe = Arc::clone(&self.probe);
        let report = tokio::task::spawn_blocking(move || probe())
            .await
            .expect("diagnostics task panicked");
        *entry = Some(Entry {
            report: report.clone(),
            generation,
            computed_at: now,
        });
        (report, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    const HOUR: Duration = Duration::from_secs(3600);

    /// A cache whose probe returns how many times it has run.
    fn counting_cache(ttl: Duration) -> (DiagnosticsCache<usize>, Arc<AtomicUsize>) {
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&runs);
        let probe = move || counter.fetch_add(1, Ordering::SeqCst) + 1;
        (DiagnosticsCache::new(Arc::new(probe), ttl), runs)
    }

    #[tokio::test]
    async fn reuses_one_report_within_the_ttl() {
        let (cache, runs) = counting_cache(HOUR);
        for _ in 0..3 {
            assert_eq!(cache.report(false).await, 1);
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn report_and_probed_says_whether_this_call_ran_the_probe() {
        let (cache, _) = counting_cache(HOUR);
        assert_eq!(cache.report_and_probed(false).await, (1, true));
        assert_eq!(cache.report_and_probed(false).await, (1, false));
        cache.invalidate();
        assert_eq!(cache.report_and_probed(false).await, (2, true));
        assert_eq!(cache.report_and_probed(true).await, (3, true));
    }

    #[tokio::test]
    async fn invalidation_during_a_probe_leaves_its_report_stale() {
        // The first probe run sees a failure reported while it is running.
        let cache = Arc::new_cyclic(|cache: &std::sync::Weak<DiagnosticsCache<usize>>| {
            let cache = cache.clone();
            let runs = AtomicUsize::new(0);
            let probe = move || {
                let run = runs.fetch_add(1, Ordering::SeqCst) + 1;
                if run == 1 {
                    cache.upgrade().unwrap().invalidate();
                }
                run
            };
            DiagnosticsCache::new(Arc::new(probe), HOUR)
        });
        assert_eq!(cache.report(false).await, 1);
        assert_eq!(cache.report(false).await, 2);
        assert_eq!(cache.report(false).await, 2);
    }

    #[test]
    fn report_expires_exactly_at_the_ttl() {
        let computed_at = Instant::now();
        let ttl = DIAGNOSTICS_TTL;
        let entry = Entry {
            report: (),
            generation: 4,
            computed_at,
        };
        assert!(entry.is_fresh(4, computed_at, ttl));
        assert!(entry.is_fresh(4, computed_at + ttl - Duration::from_nanos(1), ttl));
        assert!(!entry.is_fresh(4, computed_at + ttl, ttl));
        assert!(!entry.is_fresh(5, computed_at, ttl));
    }

    #[tokio::test]
    async fn callers_get_copies_the_cache_does_not_share() {
        let cache = DiagnosticsCache::new(Arc::new(|| vec!["probed"]), HOUR);
        let mut first = cache.report(false).await;
        first.push("recorded by one call");
        assert_eq!(cache.report(false).await, ["probed"]);
    }

    #[tokio::test]
    async fn concurrent_callers_share_one_probe() {
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&runs);
        let probe = move || {
            std::thread::sleep(Duration::from_millis(20));
            counter.fetch_add(1, Ordering::SeqCst) + 1
        };
        let cache = Arc::new(DiagnosticsCache::new(Arc::new(probe), HOUR));
        let calls = (0..8)
            .map(|_| {
                let cache = Arc::clone(&cache);
                tokio::spawn(async move { cache.report(false).await })
            })
            .collect::<Vec<_>>();
        for call in calls {
            assert_eq!(call.await.unwrap(), 1);
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }
}
