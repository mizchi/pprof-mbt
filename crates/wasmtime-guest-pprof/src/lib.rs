//! Drive wasmtime's [`GuestProfiler`] for you and emit gzip'd pprof
//! directly. Spares callers from the epoch-tick / take-and-restore
//! plumbing and the JSON-intermediate step.
//!
//! # Quick start
//!
//! See `runners/wasmtime-runner/src/main.rs` in this repo for a worked
//! example; the key types are [`ProfileSession`], [`ProfilerHost`], and
//! [`TakeProfileSession`].

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use firefox_to_pprof::{Builder, FirefoxProfile, FuncTableResolver, SampleWeighting};
use wasmtime::{Engine, GuestProfiler, Module, Store, UpdateDeadline};

/// Owned wrapper around wasmtime's [`GuestProfiler`].
///
/// Wrapped in [`Option`] internally so the deadline callback can `take()`
/// the profiler out, call `sample()` with the store as `AsContext`, and
/// put it back — wasmtime's signature requires holding both `&mut
/// GuestProfiler` and a context simultaneously, which the borrow checker
/// won't grant on a plain `&mut data().profiler` field.
pub struct ProfileSession {
    inner: Option<GuestProfiler>,
}

impl ProfileSession {
    /// Construct a new session. See [`GuestProfiler::new`] for the
    /// parameter contract.
    pub fn new(
        engine: &Engine,
        name: &str,
        interval: Duration,
        modules: impl IntoIterator<Item = (String, Module)>,
    ) -> Result<Self> {
        let prof = GuestProfiler::new(engine, name, interval, modules)?;
        Ok(Self { inner: Some(prof) })
    }

    /// Borrow the underlying profiler, panicking if it was taken and not
    /// returned (a bug in the deadline callback).
    pub fn get_mut(&mut self) -> Option<&mut GuestProfiler> {
        self.inner.as_mut()
    }

    /// Hand the profiler over to the deadline callback. The callback must
    /// put it back via [`put_back`](Self::put_back).
    pub fn take(&mut self) -> Option<GuestProfiler> {
        self.inner.take()
    }

    /// Restore the profiler after a `take()`.
    pub fn put_back(&mut self, profiler: GuestProfiler) {
        self.inner = Some(profiler);
    }

    /// Consume the session, serialise to Firefox JSON in-memory, and
    /// convert to gzip'd pprof.
    pub fn into_pprof(self) -> Result<Vec<u8>> {
        let mut json = Vec::new();
        self.into_json(&mut json)?;
        json_to_pprof(&json)
    }

    /// Consume the session and write Firefox JSON.
    pub fn into_json(self, w: &mut Vec<u8>) -> Result<()> {
        let profiler = self
            .inner
            .ok_or_else(|| anyhow!("profiler was taken but never returned"))?;
        profiler.finish(w)?;
        Ok(())
    }
}

/// Convert Firefox Profiler JSON bytes into gzip'd pprof.
///
/// Same conventions wasmtime's `GuestProfiler` uses — funcTable holds
/// pre-resolved symbols, per-sample `timeDeltas` weigh each sample.
pub fn json_to_pprof(json: &[u8]) -> Result<Vec<u8>> {
    let profile: FirefoxProfile =
        serde_json::from_slice(json).context("parsing Firefox Profiler JSON")?;
    Builder::new(
        &profile,
        FuncTableResolver,
        SampleWeighting::PerSampleTimeDeltas {
            default_interval_ns: (profile.meta.interval.max(1.0) * 1_000_000.0) as i64,
        },
    )
    .encode()
}

/// Implemented by your `Store<T>` data so the crate can find the
/// [`ProfileSession`] in the deadline callback.
pub trait ProfilerHost: Sized {
    /// Return a mutable reference to the embedded profile session.
    fn profiler(&mut self) -> &mut ProfileSession;
}

/// Methods auto-applied to any type that implements [`ProfilerHost`].
pub trait ProfilerHostExt: ProfilerHost {
    /// Install the deadline callback on `store`. After this, every epoch
    /// tick produces one sample.
    fn install(store: &mut Store<Self>)
    where
        Self: 'static,
    {
        store.set_epoch_deadline(1);
        store.epoch_deadline_callback(|mut ctx| {
            if let Some(mut prof) = ctx.data_mut().profiler().take() {
                prof.sample(&ctx, Duration::ZERO);
                ctx.data_mut().profiler().put_back(prof);
            }
            Ok(UpdateDeadline::Continue(1))
        });
    }

    /// Spin up a background thread that calls `engine.increment_epoch()`
    /// every `interval`, driving the deadline callback. Returns an RAII
    /// guard; drop it (or let it go out of scope) to stop sampling.
    fn start_ticker(engine: &Engine, interval: Duration) -> EpochTicker {
        EpochTicker::start(engine, interval)
    }

    /// Consume the store, extract the profile session, and emit gzip'd
    /// pprof bytes.
    fn finish_pprof(store: Store<Self>) -> Result<Vec<u8>>
    where
        Self: TakeProfileSession,
    {
        let session = Self::take_session(store);
        session.into_pprof()
    }
}

impl<T: ProfilerHost> ProfilerHostExt for T {}

/// Companion trait for stores whose data owns the session — needed to
/// extract the session out of a consumed [`Store`].
pub trait TakeProfileSession: ProfilerHost {
    /// Consume `store`, returning the embedded session.
    fn take_session(store: Store<Self>) -> ProfileSession;
}

/// Background thread that bumps wasmtime's epoch counter on a fixed
/// cadence so the deadline callback fires. RAII — drop to stop.
pub struct EpochTicker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    /// Start a ticker on `engine` with the given `interval`. Most callers
    /// reach this via [`ProfilerHostExt::start_ticker`].
    pub fn start(engine: &Engine, interval: Duration) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let engine = engine.clone();
        let handle = thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                thread::sleep(interval);
                engine.increment_epoch();
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
