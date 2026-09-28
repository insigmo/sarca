//! One budget for everything this server transmits to Telegram.
//!
//! Chunk relays used to be limited only by counters that each looked sensible
//! alone — three chunks of a file at once, four files at once, three sends per
//! channel — and multiplied out to nine to twelve `sendDocument` bodies in
//! flight. Measured on the production Pi, the uplink stopped improving at three
//! streams (8.0 MB/s at one, 12.6 at three, 11.9 at six), so everything past
//! that was pure load. On that Pi the load was not free: its on-board Wi-Fi is
//! an SDIO chip (`brcmfmac`), and under sustained relay traffic it failed four
//! different ways in four days — transmit wedged at kilobytes a second, a
//! network that passed nothing but ICMP, and a kernel oops in the SDIO transfer
//! path — each taking uploads down for minutes to hours.
//!
//! So transmit is governed here, in one place:
//!
//! - **Where the server runs sets the ceiling.** A host whose uplink is Wi-Fi, or a Raspberry Pi
//!   seen from inside a container (which cannot see the host's interfaces, and whose on-board Wi-Fi
//!   is always SDIO), gets the [`Profile::Constrained`] limits: a byte-rate cap near what a single
//!   stream achieves anyway and two streams. Anything else gets [`Profile::Standard`]: no byte cap
//!   and a stream count past which nothing was ever gained. `UPLINK_PROFILE`,
//!   `TELEGRAM_UPLOAD_RATE_MB` and `TELEGRAM_UPLOAD_STREAMS` override the guess.
//! - **Sending is paced, not bursty.** Upload bodies are metered through a shared GCRA pacer as the
//!   HTTP client pulls them, so the radio sees an even stream instead of a dozen connections
//!   filling their windows at once.
//! - **Trouble shrinks the budget.** A transport error on an upload halves the rate and the stream
//!   count (several streams failing together count once); a run of clean sends grows them back
//!   towards the ceiling. Retries against a link that is already struggling are what turned a slow
//!   patch into an outage, and this is what stops them arriving at full rate.
//!
//! The relay does not need to be faster than the client leg that feeds it: a
//! file only has to be relayed about as fast as it arrives, and the spool
//! absorbs the difference while it does.

use std::{
    path::Path,
    sync::{Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant},
};

use tokio::sync::Notify;

const MIB: f64 = 1024.0 * 1024.0;

/// Byte-rate ceiling on a constrained host, in MiB/s. One `sendDocument`
/// stream reached ~8 MB/s from the Pi on its own, and the client leg into the
/// Pi runs at ~8.4 MB/s over the public route, so this keeps the relay close to
/// the rate files arrive at while taking the bursts out of it.
const CONSTRAINED_RATE_MIB: f64 = 7.0;
/// Streams in flight at once on a constrained host. Two keep the link busy
/// across one request's round trip; more only added load.
const CONSTRAINED_STREAMS: usize = 2;
/// Streams in flight at once elsewhere. Twice where the Pi's uplink stopped
/// improving; the per-channel flood limit binds long before this does.
const STANDARD_STREAMS: usize = 6;

/// The back-off never goes below this, in MiB/s — slow enough to be gentle on
/// a sick link, fast enough that a chunk still finishes in well under a minute.
const FLOOR_RATE_MIB: f64 = 1.0;
/// What an uncapped host is assumed to have been doing when its first failure
/// arrives, in MiB/s. The first back-off halves this, like any other.
const UNCAPPED_ASSUMED_RATE_MIB: f64 = 16.0;
/// On an uncapped host, recovery that climbs past this lifts the cap entirely.
const UNCAPPED_LIFT_RATE_MIB: f64 = 64.0;
/// Clean sends in a row before stepping back up.
const RECOVERY_STREAK: u32 = 6;
const RECOVERY_FACTOR: f64 = 1.25;
const BACKOFF_FACTOR: f64 = 0.5;
/// Failures closer together than this are one event: when a link drops, every
/// stream on it fails within the same few seconds, and that is one reason to
/// slow down, not three.
const BACKOFF_DEBOUNCE: Duration = Duration::from_secs(10);
/// How far ahead of the average rate the pacer lets a sender run. Enough to
/// keep a TCP window moving, short enough that there is no burst to speak of.
const BURST: Duration = Duration::from_millis(200);

/// The lowest rate an override may ask for, in MiB/s.
const MIN_OVERRIDE_RATE_MIB: f64 = 0.25;
const MAX_STREAMS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// The uplink is Wi-Fi, or probably is: pace and limit streams.
    Constrained,
    /// No reason to hold back beyond not being wasteful.
    Standard,
}

/// The ceiling the governor works under, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub profile: Profile,
    /// Bytes per second, `None` for no cap.
    pub rate: Option<f64>,
    pub streams: usize,
    /// Why this profile, for the startup log line.
    pub reason: String,
}

/// What can be learned about the machine without asking anyone.
#[derive(Debug, Default, Clone)]
struct HostFacts {
    /// Board model, e.g. "Raspberry Pi 4 Model B Rev 1.4".
    model: Option<String>,
    /// Inside a container, the network interfaces visible are the container's
    /// own, not the host's — so they say nothing about the real uplink.
    in_container: bool,
    /// The interface the default route leaves through, and whether it is
    /// wireless.
    uplink: Option<(String, bool)>,
}

impl HostFacts {
    fn read() -> Self {
        let model = std::fs::read_to_string("/proc/device-tree/model")
            .ok()
            .map(|m| m.trim_end_matches('\0').trim().to_owned())
            .filter(|m| !m.is_empty())
            .or_else(|| {
                std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|c| parse_cpuinfo_model(&c))
            });
        let in_container = std::env::var_os("SARCA_IN_DOCKER").is_some()
            || Path::new("/.dockerenv").exists()
            || Path::new("/run/.containerenv").exists();
        let uplink = std::fs::read_to_string("/proc/net/route")
            .ok()
            .and_then(|routes| parse_default_route(&routes))
            .map(|iface| {
                let wireless = Path::new("/sys/class/net").join(&iface).join("wireless").exists()
                    || std::fs::read_to_string("/proc/net/wireless").is_ok_and(|w| {
                        w.lines().any(|l| l.trim_start().starts_with(&format!("{iface}:")))
                    });
                (iface, wireless)
            });
        Self {
            model,
            in_container,
            uplink,
        }
    }
}

/// The board model from `/proc/cpuinfo` — the one place a Raspberry Pi names
/// itself that is still visible from inside a container (`/proc/device-tree`
/// is masked there).
fn parse_cpuinfo_model(cpuinfo: &str) -> Option<String> {
    cpuinfo
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.trim() == "Model")
        .map(|(_, value)| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The interface of the lowest-metric default route in `/proc/net/route`.
fn parse_default_route(routes: &str) -> Option<String> {
    const RTF_UP: u32 = 0x1;
    routes
        .lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let (iface, dest, flags, metric, mask) =
                (cols.first()?, cols.get(1)?, cols.get(3)?, cols.get(6)?, cols.get(7)?);
            let flags = u32::from_str_radix(flags, 16).ok()?;
            let is_default = *dest == "00000000" && *mask == "00000000" && flags & RTF_UP != 0;
            is_default.then(|| (metric.parse::<u32>().unwrap_or(u32::MAX), (*iface).to_owned()))
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, iface)| iface)
}

/// Which profile the facts point to, and a sentence saying why.
fn classify(facts: &HostFacts) -> (Profile, String) {
    // On the host itself the route is the whole answer, whatever the board.
    if !facts.in_container {
        if let Some((iface, wireless)) = &facts.uplink {
            return if *wireless {
                (Profile::Constrained, format!("uplink {iface} is Wi-Fi"))
            } else {
                (Profile::Standard, format!("uplink {iface} is wired"))
            };
        }
    }
    match &facts.model {
        Some(model) if model.contains("Raspberry Pi") => {
            let reason = if facts.in_container {
                format!(
                    "{model}; the host's network is not visible from the container, so assuming \
                     its on-board SDIO Wi-Fi"
                )
            } else {
                format!("{model}; uplink unknown, so assuming its on-board SDIO Wi-Fi")
            };
            (Profile::Constrained, reason)
        },
        _ => (Profile::Standard, "no Wi-Fi uplink detected".to_owned()),
    }
}

/// The detected profile with the operator's overrides applied on top.
fn resolve_limits(
    facts: &HostFacts,
    profile_env: Option<&str>,
    rate_env: Option<&str>,
    streams_env: Option<&str>,
) -> Limits {
    let (profile, reason) = match profile_env.map(|p| p.trim().to_ascii_lowercase()).as_deref() {
        Some("constrained") => (Profile::Constrained, "UPLINK_PROFILE=constrained".to_owned()),
        Some("standard") => (Profile::Standard, "UPLINK_PROFILE=standard".to_owned()),
        _ => classify(facts),
    };
    let (default_rate, default_streams) = match profile {
        Profile::Constrained => (Some(CONSTRAINED_RATE_MIB * MIB), CONSTRAINED_STREAMS),
        Profile::Standard => (None, STANDARD_STREAMS),
    };
    let rate = match rate_env.and_then(|r| r.trim().parse::<f64>().ok()) {
        Some(mib) if mib.is_finite() && mib > 0.0 => Some(mib.max(MIN_OVERRIDE_RATE_MIB) * MIB),
        // "0" asks for no cap at all.
        Some(mib) if mib.abs() < f64::EPSILON => None,
        _ => default_rate,
    };
    let streams = streams_env
        .and_then(|s| s.trim().parse::<usize>().ok())
        .map_or(default_streams, |n| n.clamp(1, MAX_STREAMS));
    Limits {
        profile,
        rate,
        streams,
        reason,
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// The process-wide governor, detected and logged on first use.
pub fn governor() -> &'static Uplink {
    static GOVERNOR: OnceLock<Uplink> = OnceLock::new();
    GOVERNOR.get_or_init(|| {
        let limits = resolve_limits(
            &HostFacts::read(),
            env("UPLINK_PROFILE").as_deref(),
            env("TELEGRAM_UPLOAD_RATE_MB").as_deref(),
            env("TELEGRAM_UPLOAD_STREAMS").as_deref(),
        );
        tracing::info!(
            "[UPLINK] {} profile ({}): Telegram uploads {}, {} stream(s) at once; override with \
             UPLINK_PROFILE / TELEGRAM_UPLOAD_RATE_MB / TELEGRAM_UPLOAD_STREAMS",
            match limits.profile {
                Profile::Constrained => "constrained",
                Profile::Standard => "standard",
            },
            limits.reason,
            describe_rate(limits.rate),
            limits.streams,
        );
        Uplink::new(limits)
    })
}

fn describe_rate(rate: Option<f64>) -> String {
    rate.map_or_else(|| "uncapped".to_owned(), |r| format!("capped at {:.1} MiB/s", r / MIB))
}

/// A governor's current rate and stream limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Budget {
    /// Bytes per second, `None` for no cap.
    pub rate: Option<f64>,
    pub streams: usize,
}

/// Paces and counts what is sent to Telegram. See the module docs.
pub struct Uplink {
    max: Limits,
    state: Mutex<State>,
    slot_freed: Notify,
}

#[derive(Debug)]
struct State {
    /// Current cap in bytes per second — at or below `max.rate` — or `None`.
    rate: Option<f64>,
    /// Current stream limit, at or below `max.streams`.
    streams: usize,
    in_flight: usize,
    /// When the bytes reserved so far will have gone out at `rate`.
    tat: Option<Instant>,
    ok_streak: u32,
    last_backoff: Option<Instant>,
}

impl Uplink {
    pub fn new(max: Limits) -> Self {
        let state = State {
            rate: max.rate,
            streams: max.streams.max(1),
            in_flight: 0,
            tat: None,
            ok_streak: 0,
            last_backoff: None,
        };
        Self {
            max,
            state: Mutex::new(state),
            slot_freed: Notify::new(),
        }
    }

    /// The ceiling this governor was built with.
    pub fn limits(&self) -> &Limits {
        &self.max
    }

    /// The budget in force right now, after any back-off.
    pub fn current(&self) -> Budget {
        let state = self.lock();
        Budget {
            rate: state.rate,
            streams: state.streams,
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Nothing panics while holding it; recover the data if something ever does.
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Wait for one of the streams the budget allows. Hold the slot for as long
    /// as a request body is being sent.
    pub async fn stream(&self) -> StreamSlot<'_> {
        loop {
            let notified = self.slot_freed.notified();
            tokio::pin!(notified);
            // Register before checking, so a slot freed in between still wakes us.
            notified.as_mut().enable();
            if self.try_take_slot() {
                return StreamSlot {
                    uplink: self,
                };
            }
            notified.await;
        }
    }

    /// Kept out of `stream` so no lock guard is ever in scope across its await.
    fn try_take_slot(&self) -> bool {
        let mut state = self.lock();
        if state.in_flight < state.streams {
            state.in_flight += 1;
            true
        } else {
            false
        }
    }

    /// Account for `bytes` about to go out, sleeping for as long as the current
    /// rate requires. Returns at once when there is no cap.
    pub async fn pace(&self, bytes: usize) {
        if let Some(release) = self.reserve(bytes, Instant::now()) {
            tokio::time::sleep_until(release.into()).await;
        }
    }

    /// GCRA: push the theoretical send time out by `bytes` at the current rate,
    /// and release the caller once it is within [`BURST`] of that time.
    fn reserve(&self, bytes: usize, now: Instant) -> Option<Instant> {
        // One reservation is at most a chunk; u32 holds that with room to spare.
        let bytes = f64::from(u32::try_from(bytes).unwrap_or(u32::MAX));
        let mut state = self.lock();
        let rate = state.rate?;
        let base = state.tat.filter(|tat| *tat > now).unwrap_or(now);
        let tat = base + Duration::from_secs_f64(bytes / rate);
        state.tat = Some(tat);
        drop(state);
        tat.checked_sub(BURST).filter(|release| *release > now)
    }

    /// A request body went out and Telegram answered.
    pub fn note_ok(&self) {
        let mut state = self.lock();
        state.ok_streak += 1;
        if state.ok_streak < RECOVERY_STREAK {
            return;
        }
        state.ok_streak = 0;
        let before = (state.rate, state.streams);
        state.streams = (state.streams + 1).min(self.max.streams);
        state.rate = state.rate.and_then(|rate| {
            let next = rate * RECOVERY_FACTOR;
            // Up to the ceiling; with no ceiling, until the cap is high enough
            // to be pointless, and then off.
            self.max.rate.map_or_else(
                || (next < UNCAPPED_LIFT_RATE_MIB * MIB).then_some(next),
                |max| Some(next.min(max)),
            )
        });
        let after = (state.rate, state.streams);
        drop(state);
        if after != before {
            if after == (self.max.rate, self.max.streams) {
                tracing::info!(
                    "[UPLINK] Telegram uploads back at full budget: {}, {} stream(s)",
                    describe_rate(after.0),
                    after.1
                );
            } else {
                tracing::debug!(
                    "[UPLINK] stepping back up: {}, {} stream(s)",
                    describe_rate(after.0),
                    after.1
                );
            }
            if after.1 > before.1 {
                self.slot_freed.notify_waiters();
            }
        }
    }

    /// An upload failed below HTTP — the connection broke, timed out or never
    /// came up.
    pub fn note_transport_error(&self) {
        self.back_off_at(Instant::now());
    }

    fn back_off_at(&self, now: Instant) {
        let mut state = self.lock();
        state.ok_streak = 0;
        if state.last_backoff.is_some_and(|at| now.saturating_duration_since(at) < BACKOFF_DEBOUNCE)
        {
            return;
        }
        state.last_backoff = Some(now);
        let current = state.rate.unwrap_or(UNCAPPED_ASSUMED_RATE_MIB * MIB);
        state.rate = Some((current * BACKOFF_FACTOR).max(FLOOR_RATE_MIB * MIB));
        state.streams = (state.streams / 2).max(1);
        let (rate, streams) = (state.rate, state.streams);
        drop(state);
        tracing::warn!(
            "[UPLINK] Telegram uploads are failing on the network — backing off to {}, {} \
             stream(s)",
            describe_rate(rate),
            streams
        );
    }

    #[cfg(test)]
    fn snapshot(&self) -> (Option<f64>, usize, usize) {
        let state = self.lock();
        (state.rate, state.streams, state.in_flight)
    }
}

/// One stream of the budget, returned when dropped.
pub struct StreamSlot<'a> {
    uplink: &'a Uplink,
}

impl Drop for StreamSlot<'_> {
    fn drop(&mut self) {
        self.uplink.lock().in_flight -= 1;
        self.uplink.slot_freed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(model: Option<&str>, in_container: bool, uplink: Option<(&str, bool)>) -> HostFacts {
        HostFacts {
            model: model.map(str::to_owned),
            in_container,
            uplink: uplink.map(|(iface, wireless)| (iface.to_owned(), wireless)),
        }
    }

    const PI: &str = "Raspberry Pi 4 Model B Rev 1.4";

    /// The production box: a Pi 4 on Wi-Fi, running in Docker, where all it
    /// can see of itself is the model line in `/proc/cpuinfo`.
    #[test]
    fn a_pi_seen_from_a_container_is_constrained() {
        let limits =
            resolve_limits(&facts(Some(PI), true, Some(("eth0", false))), None, None, None);
        assert_eq!(limits.profile, Profile::Constrained);
        assert_eq!(limits.rate, Some(CONSTRAINED_RATE_MIB * MIB));
        assert_eq!(limits.streams, CONSTRAINED_STREAMS);
        assert!(limits.reason.contains("container"), "{}", limits.reason);
    }

    /// On the host itself the route decides, not the board: a Pi on Ethernet
    /// has nothing to protect, and a laptop on Wi-Fi does.
    #[test]
    fn on_the_host_the_uplink_interface_decides() {
        let wired_pi =
            resolve_limits(&facts(Some(PI), false, Some(("eth0", false))), None, None, None);
        assert_eq!(wired_pi.profile, Profile::Standard);
        assert_eq!(wired_pi.rate, None);

        let wifi_pc = resolve_limits(&facts(None, false, Some(("wlp2s0", true))), None, None, None);
        assert_eq!(wifi_pc.profile, Profile::Constrained);
        assert!(wifi_pc.reason.contains("wlp2s0"), "{}", wifi_pc.reason);
    }

    #[test]
    fn an_ordinary_server_is_standard() {
        let limits = resolve_limits(&facts(None, true, None), None, None, None);
        assert_eq!(limits.profile, Profile::Standard);
        assert_eq!(limits.rate, None);
        assert_eq!(limits.streams, STANDARD_STREAMS);
    }

    #[test]
    fn overrides_win_over_detection() {
        let pi = facts(Some(PI), true, None);

        let forced = resolve_limits(&pi, Some(" Standard "), None, None);
        assert_eq!(forced.profile, Profile::Standard);
        assert_eq!(forced.rate, None);

        let tuned = resolve_limits(&pi, None, Some("12.5"), Some("3"));
        assert_eq!(tuned.profile, Profile::Constrained);
        assert_eq!(tuned.rate, Some(12.5 * MIB));
        assert_eq!(tuned.streams, 3);

        let uncapped = resolve_limits(&pi, None, Some("0"), None);
        assert_eq!(uncapped.rate, None, "0 means no cap");
    }

    /// A typo must not stall uploads or turn the limits off.
    #[test]
    fn nonsense_overrides_fall_back_or_clamp() {
        let pi = facts(Some(PI), true, None);
        let limits = resolve_limits(&pi, Some("fast"), Some("lots"), Some("0"));
        assert_eq!(limits.profile, Profile::Constrained);
        assert_eq!(limits.rate, Some(CONSTRAINED_RATE_MIB * MIB));
        assert_eq!(limits.streams, 1, "zero streams would stop every upload");

        let tiny = resolve_limits(&pi, None, Some("0.001"), Some("500"));
        assert_eq!(tiny.rate, Some(MIN_OVERRIDE_RATE_MIB * MIB));
        assert_eq!(tiny.streams, MAX_STREAMS);

        let negative = resolve_limits(&pi, None, Some("-3"), None);
        assert_eq!(negative.rate, Some(CONSTRAINED_RATE_MIB * MIB));
    }

    #[test]
    fn reads_the_model_line_from_cpuinfo() {
        let cpuinfo = "processor\t: 3\nBogoMIPS\t: 108.00\n\nRevision\t: d03114\nSerial\t\t: \
                       10000000abcd\nModel\t\t: Raspberry Pi 4 Model B Rev 1.4\n";
        assert_eq!(parse_cpuinfo_model(cpuinfo).as_deref(), Some(PI));
        assert_eq!(parse_cpuinfo_model("processor\t: 0\nmodel name\t: Intel(R) Xeon(R)\n"), None);
    }

    #[test]
    fn finds_the_default_route_with_the_lowest_metric() {
        let routes = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                      wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
                      eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
                      eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\t0\t0\t0\n";
        assert_eq!(parse_default_route(routes).as_deref(), Some("eth0"));

        let wifi_only = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
                         wlan0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\n";
        assert_eq!(parse_default_route(wifi_only).as_deref(), Some("wlan0"));

        let no_default = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
                          eth0\t0001A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF\n";
        assert_eq!(parse_default_route(no_default), None);
    }

    fn capped(rate_mib: f64, streams: usize) -> Uplink {
        Uplink::new(Limits {
            profile: Profile::Constrained,
            rate: Some(rate_mib * MIB),
            streams,
            reason: "test".to_owned(),
        })
    }

    fn uncapped(streams: usize) -> Uplink {
        Uplink::new(Limits {
            profile: Profile::Standard,
            rate: None,
            streams,
            reason: "test".to_owned(),
        })
    }

    /// Bytes are released at the configured rate, with only [`BURST`] of
    /// slack ahead of it.
    #[test]
    fn the_pacer_releases_bytes_at_the_configured_rate() {
        let uplink = capped(1.0, 2);
        let start = Instant::now();
        let quarter = (MIB / 4.0) as usize;

        // The first quarter-second of data is inside the burst allowance.
        assert_eq!(uplink.reserve(quarter / 2, start), None);
        // A full MiB after it is due one second after the start, released
        // `BURST` early.
        let release = uplink.reserve(quarter * 4, start).expect("must wait");
        let expected = start + Duration::from_millis(1125).checked_sub(BURST).unwrap();
        assert!(release.saturating_duration_since(expected) < Duration::from_millis(2));
        assert!(expected.saturating_duration_since(release) < Duration::from_millis(2));
    }

    /// Idle time is not banked: after a pause the pacer starts from now, so a
    /// quiet minute cannot turn into a minute's worth of burst.
    #[test]
    fn the_pacer_does_not_bank_idle_time() {
        let uplink = capped(1.0, 2);
        let start = Instant::now();
        let mib = MIB as usize;
        uplink.reserve(mib, start);
        let later = start + Duration::from_secs(60);
        let release = uplink.reserve(mib, later).expect("a full MiB still has to wait");
        assert!(release > later + Duration::from_millis(700));
    }

    #[test]
    fn no_cap_means_no_waiting() {
        let uplink = uncapped(6);
        let now = Instant::now();
        for _ in 0..100 {
            assert_eq!(uplink.reserve(20 * MIB as usize, now), None);
        }
    }

    #[test]
    fn a_failure_halves_rate_and_streams_and_a_burst_of_them_counts_once() {
        let uplink = capped(8.0, 4);
        let t0 = Instant::now();

        uplink.back_off_at(t0);
        assert_eq!(uplink.snapshot(), (Some(4.0 * MIB), 2, 0));

        // The other streams on the same dead link, a moment later.
        uplink.back_off_at(t0 + Duration::from_secs(1));
        uplink.back_off_at(t0 + Duration::from_secs(3));
        assert_eq!(uplink.snapshot(), (Some(4.0 * MIB), 2, 0));

        // A separate failure later on is a new event.
        uplink.back_off_at(t0 + BACKOFF_DEBOUNCE + Duration::from_secs(1));
        assert_eq!(uplink.snapshot(), (Some(2.0 * MIB), 1, 0));
    }

    #[test]
    fn the_back_off_has_a_floor() {
        let uplink = capped(8.0, 2);
        let mut at = Instant::now();
        for _ in 0..20 {
            uplink.back_off_at(at);
            at += BACKOFF_DEBOUNCE * 2;
        }
        assert_eq!(uplink.snapshot(), (Some(FLOOR_RATE_MIB * MIB), 1, 0));
    }

    #[test]
    fn clean_sends_climb_back_to_the_ceiling_and_no_further() {
        let uplink = capped(8.0, 2);
        uplink.back_off_at(Instant::now());
        assert_eq!(uplink.snapshot(), (Some(4.0 * MIB), 1, 0));

        for _ in 0..RECOVERY_STREAK - 1 {
            uplink.note_ok();
        }
        assert_eq!(uplink.snapshot(), (Some(4.0 * MIB), 1, 0), "not before a full streak");
        uplink.note_ok();
        assert_eq!(uplink.snapshot(), (Some(5.0 * MIB), 2, 0));

        for _ in 0..RECOVERY_STREAK * 20 {
            uplink.note_ok();
        }
        assert_eq!(uplink.snapshot(), (Some(8.0 * MIB), 2, 0));
    }

    /// An uncapped server gets a cap when its link starts failing, and loses
    /// it again once recovery has climbed far enough.
    #[test]
    fn an_uncapped_host_is_capped_while_failing_and_uncapped_after() {
        let uplink = uncapped(6);
        uplink.back_off_at(Instant::now());
        assert_eq!(
            uplink.snapshot(),
            (Some(UNCAPPED_ASSUMED_RATE_MIB * BACKOFF_FACTOR * MIB), 3, 0)
        );

        for _ in 0..RECOVERY_STREAK * 40 {
            uplink.note_ok();
        }
        assert_eq!(uplink.snapshot(), (None, 6, 0));
    }

    /// A failure breaks the streak: recovery needs clean sends in a row.
    #[test]
    fn a_failure_resets_the_recovery_streak() {
        let uplink = capped(8.0, 2);
        let t0 = Instant::now();
        uplink.back_off_at(t0);
        for _ in 0..RECOVERY_STREAK - 1 {
            uplink.note_ok();
        }
        // Debounced, so the rate stays put — but the streak still restarts.
        uplink.back_off_at(t0 + Duration::from_secs(1));
        uplink.note_ok();
        assert_eq!(uplink.snapshot(), (Some(4.0 * MIB), 1, 0));
    }

    #[tokio::test]
    async fn streams_beyond_the_budget_wait_for_a_free_slot() {
        let uplink: &'static Uplink = Box::leak(Box::new(capped(8.0, 1)));

        let first = uplink.stream().await;
        let second = tokio::spawn(async move {
            let _slot = uplink.stream().await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!second.is_finished(), "the budget allows one stream");

        drop(first);
        tokio::time::timeout(Duration::from_secs(2), second)
            .await
            .expect("a freed slot must wake the waiter")
            .unwrap();
        assert_eq!(uplink.snapshot().2, 0, "every slot came back");
    }

    /// Recovery that raises the stream limit lets a waiter in straight away,
    /// without anyone having to finish first.
    #[tokio::test]
    async fn a_raised_stream_limit_wakes_waiters() {
        let uplink: &'static Uplink = Box::leak(Box::new(capped(8.0, 2)));
        uplink.back_off_at(Instant::now());
        assert_eq!(uplink.snapshot().1, 1);

        let _held = uplink.stream().await;
        let waiter = tokio::spawn(async move {
            let _slot = uplink.stream().await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiter.is_finished());

        for _ in 0..RECOVERY_STREAK {
            uplink.note_ok();
        }
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("the second stream opened up")
            .unwrap();
    }
}
