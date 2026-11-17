use super::*;
use crate::net::CacheKey;
use std::{
    error::Error,
    fmt,
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

#[derive(Default)]
struct TestClock {
    nanos: std::sync::atomic::AtomicU64,
    waits: Mutex<Vec<Duration>>,
}
impl TestClock {
    fn advance(&self, duration: Duration) {
        self.nanos
            .fetch_add(duration.as_nanos() as u64, Ordering::SeqCst);
    }
    fn waits(&self) -> Vec<Duration> {
        self.waits.lock().unwrap().clone()
    }
}
impl Clock for TestClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(self.nanos.load(Ordering::SeqCst))
    }
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        self.waits.lock().unwrap().push(duration);
        self.advance(duration);
        Box::pin(std::future::ready(()))
    }
}

type ResolverCall = (String, u16, i32, i32, i32, i32);

#[derive(Default)]
struct FakeResolver {
    calls: Mutex<Vec<ResolverCall>>,
    stale_fails: AtomicBool,
    delay: AtomicBool,
}
impl FakeResolver {
    fn names(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.0.clone())
            .collect()
    }
    fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }
}
impl Resolver for FakeResolver {
    fn getaddrinfo(
        &self,
        host: &str,
        port: u16,
        family: i32,
        sock_type: i32,
        proto: i32,
        flags: i32,
    ) -> Result<Vec<SocketAddr>, ResolveError> {
        self.calls
            .lock()
            .unwrap()
            .push((host.to_owned(), port, family, sock_type, proto, flags));
        if self.delay.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(15));
        }
        if host.starts_with("bad")
            || (host.starts_with("stale") && self.stale_fails.load(Ordering::SeqCst))
        {
            return Err(ResolveError::Dns(DnsError::new(
                libc::EAI_AGAIN,
                "temporary failure in name resolution",
            )));
        }
        if host.starts_with("gone") {
            return Err(ResolveError::Dns(DnsError::new(
                libc::EAI_NONAME,
                "no such host",
            )));
        }
        Ok(vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)])
    }
}

fn settings() -> NetworkSettings {
    NetworkSettings {
        max_sockets: 3,
        dns_cache_seconds: 300.0,
        dns_nx_seconds: 15.0,
        dns_per_second: 1_000_000.0,
        dns_burst: 1_000_000.0,
        dns_breaker_failures: 3,
        dns_breaker_seconds: 30.0,
    }
}
fn setup() -> (Arc<NetworkGuard>, Arc<FakeResolver>, Arc<TestClock>) {
    let resolver = Arc::new(FakeResolver::default());
    let clock = Arc::new(TestClock::default());
    let guard = Arc::new(NetworkGuard::new(
        settings(),
        resolver.clone(),
        clock.clone(),
    ));
    (guard, resolver, clock)
}
fn resolve<'a>(
    guard: &'a NetworkGuard,
    host: &'a str,
    port: u16,
) -> impl Future<Output = Result<Vec<SocketAddr>, ResolveError>> + 'a {
    guard.resolve(host, port, 0, 0, 0, 0)
}
fn code(result: Result<Vec<SocketAddr>, ResolveError>) -> i32 {
    match result {
        Err(ResolveError::Dns(e)) => e.code,
        other => panic!("expected DNS error, got {other:?}"),
    }
}
fn addr(port: u16) -> Vec<SocketAddr> {
    vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)]
}

#[tokio::test]
async fn parity_part_a_34_assertions() {
    let (guard, resolver, clock) = setup();
    assert_eq!(
        resolve(&guard, "api.smartrecruiters.com", 443)
            .await
            .unwrap(),
        addr(443)
    ); // A1
    for _ in 0..5 {
        assert_eq!(
            resolve(&guard, "api.smartrecruiters.com", 443)
                .await
                .unwrap(),
            addr(443)
        );
    } // A2
    assert_eq!(resolver.names(), ["api.smartrecruiters.com"]); // A3
    assert_eq!(
        resolve(&guard, "boards-api.greenhouse.io", 443)
            .await
            .unwrap(),
        addr(443)
    ); // A4
    assert_eq!(
        resolver.names(),
        ["api.smartrecruiters.com", "boards-api.greenhouse.io"]
    ); // A5
    assert_eq!(guard.stats().hits, 5); // A6
    assert_eq!(
        resolve(&guard, "api.smartrecruiters.com", 8443)
            .await
            .unwrap(),
        addr(8443)
    ); // A7
    let hits = guard.stats().hits;
    assert_eq!(resolve(&guard, "127.0.0.1", 443).await.unwrap(), addr(443)); // A8
    assert_eq!(guard.stats().hits, hits);

    guard.reset();
    resolver.clear();
    assert_eq!(
        code(resolve(&guard, "bad-a.example", 443).await),
        libc::EAI_AGAIN
    ); // A9
    assert_eq!(
        code(resolve(&guard, "bad-a.example", 443).await),
        libc::EAI_AGAIN
    ); // A10
    assert_eq!(
        resolver
            .names()
            .iter()
            .filter(|n| n.as_str() == "bad-a.example")
            .count(),
        1
    ); // A11
    guard.reset();
    resolver.clear();
    assert_eq!(
        code(resolve(&guard, "gone.example", 443).await),
        libc::EAI_NONAME
    ); // A12
    assert_eq!(
        code(resolve(&guard, "gone.example", 443).await),
        libc::EAI_NONAME
    ); // A13
    assert_eq!(
        resolver
            .names()
            .iter()
            .filter(|n| n.as_str() == "gone.example")
            .count(),
        1
    ); // A14

    guard.reset();
    resolver.clear();
    assert_eq!(
        resolve(&guard, "stale.example", 443).await.unwrap(),
        addr(443)
    ); // A15
    clock.advance(Duration::from_secs(300));
    resolver.stale_fails.store(true, Ordering::SeqCst);
    assert_eq!(
        resolve(&guard, "stale.example", 443).await.unwrap(),
        addr(443)
    ); // A16
    assert_eq!(guard.stats().stale, 1); // A17
    let key = CacheKey {
        host: "stale.example".into(),
        port: 443,
        family: 0,
        sock_type: 0,
        proto: 0,
    };
    {
        let state = guard.state.lock().unwrap();
        let entry = state.cache.get(&key).unwrap();
        assert_eq!(entry.addresses, Some(addr(443)));
        assert_eq!(entry.errno, None); // A18
        assert_eq!(entry.expires, clock.now() + Duration::from_secs(15));
    }

    guard.reset();
    resolver.clear();
    resolver.stale_fails.store(false, Ordering::SeqCst);
    assert_eq!(
        code(resolve(&guard, "bad-1.example", 443).await),
        libc::EAI_AGAIN
    ); // A19
    assert_eq!(
        code(resolve(&guard, "bad-2.example", 443).await),
        libc::EAI_AGAIN
    ); // A20
    assert_eq!(
        code(resolve(&guard, "bad-3.example", 443).await),
        libc::EAI_AGAIN
    ); // A21
    assert_eq!(guard.stats().breaker_trips, 1); // A22
    let before = resolver.names().len();
    let paused = resolve(&guard, "new.example", 443).await.unwrap_err(); // A23
    match paused {
        ResolveError::Dns(e) => {
            assert_eq!(e.code, libc::EAI_AGAIN);
            assert_eq!(
                e.message,
                "name resolution paused: the resolver was refusing lookups"
            );
        }
        _ => panic!("not DNS"),
    }
    assert_eq!(resolver.names().len(), before); // A24
    assert_eq!(guard.stats().breaker_skips, 1); // A25
    assert!(is_dns_error(&DnsError::new(libc::EAI_AGAIN, "x"))); // A26

    #[derive(Debug)]
    struct Transport(Box<dyn Error + Send + Sync>);
    impl fmt::Display for Transport {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "transport: {}", self.0)
        }
    }
    impl Error for Transport {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(self.0.as_ref())
        }
    }
    assert!(is_dns_error(&Transport(Box::new(DnsError::new(
        libc::EAI_AGAIN,
        "x"
    ))))); // A27
    #[derive(Debug)]
    struct Timeout;
    impl fmt::Display for Timeout {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("timeout")
        }
    }
    impl Error for Timeout {}
    assert!(!is_dns_error(&Transport(Box::new(Timeout)))); // A28
    assert!(!is_dns_error(&std::io::Error::other("boom"))); // A29

    #[derive(Default)]
    struct Response {
        closes: Arc<AtomicUsize>,
        value: bool,
    }
    impl HttpResponse for Response {
        fn close(&mut self) {
            self.closes.fetch_add(1, Ordering::SeqCst);
        }
    }
    let releases = Arc::new(AtomicUsize::new(0));
    let closes = Arc::new(AtomicUsize::new(0));
    let body = Response {
        closes: closes.clone(),
        value: true,
    };
    let release_count = releases.clone();
    let mut gated = GatedResponse::new(body, move || {
        release_count.fetch_add(1, Ordering::SeqCst);
    });
    assert!(gated.response().value); // A30
    gated.close(); // A31
    assert_eq!(releases.load(Ordering::SeqCst), 1);
    gated.close();
    assert_eq!(releases.load(Ordering::SeqCst), 1); // A32
    assert_eq!(closes.load(Ordering::SeqCst), 1); // A33: close was delegated exactly once
    let release_count = releases.clone();
    drop(GatedResponse::new(Response::default(), move || {
        release_count.fetch_add(1, Ordering::SeqCst);
    })); // A34
    assert_eq!(releases.load(Ordering::SeqCst), 2);
    let _ = clock;
}

#[tokio::test]
async fn b1_b2_ttl_boundaries_are_strict() {
    let (guard, resolver, clock) = setup();
    resolve(&guard, "ttl.example", 443).await.unwrap();
    clock.advance(Duration::from_micros(299_999_000));
    resolve(&guard, "ttl.example", 443).await.unwrap();
    assert_eq!(resolver.names().len(), 1);
    clock.advance(Duration::from_micros(1_000));
    resolve(&guard, "ttl.example", 443).await.unwrap();
    assert_eq!(resolver.names().len(), 2);
    assert_eq!(
        code(resolve(&guard, "bad-a.example", 443).await),
        libc::EAI_AGAIN
    );
    clock.advance(Duration::from_micros(14_999_000));
    assert_eq!(
        code(resolve(&guard, "bad-a.example", 443).await),
        libc::EAI_AGAIN
    );
    assert_eq!(
        resolver
            .names()
            .iter()
            .filter(|n| n.as_str() == "bad-a.example")
            .count(),
        1
    );
    clock.advance(Duration::from_micros(1_000));
    assert_eq!(
        code(resolve(&guard, "bad-a.example", 443).await),
        libc::EAI_AGAIN
    );
    assert_eq!(
        resolver
            .names()
            .iter()
            .filter(|n| n.as_str() == "bad-a.example")
            .count(),
        2
    );
}

#[tokio::test]
async fn b3_breaker_closes_and_reopens_only_on_third_failure() {
    let (guard, resolver, clock) = setup();
    for n in 1..=3 {
        assert_eq!(
            code(resolve(&guard, &format!("bad-{n}.example"), 443).await),
            libc::EAI_AGAIN
        );
    }
    assert_eq!(guard.stats().breaker_trips, 1);
    clock.advance(Duration::from_micros(29_999_000));
    assert!(matches!(
        resolve(&guard, "too-soon.example", 443).await,
        Err(ResolveError::Dns(_))
    ));
    let count = resolver.names().len();
    assert_eq!(count, 3);
    clock.advance(Duration::from_micros(1_000));
    assert_eq!(
        resolve(&guard, "now-open.example", 443).await.unwrap(),
        addr(443)
    );
    for n in 1..=2 {
        assert_eq!(
            code(resolve(&guard, &format!("bad-again-{n}.example"), 443).await),
            libc::EAI_AGAIN
        );
    }
    assert_eq!(guard.stats().breaker_trips, 1);
    assert_eq!(
        code(resolve(&guard, "bad-again-3.example", 443).await),
        libc::EAI_AGAIN
    );
    assert_eq!(guard.stats().breaker_trips, 2);
}

#[tokio::test]
async fn b4_cached_positive_is_available_while_paused() {
    let (guard, resolver, _) = setup();
    resolve(&guard, "ok.example", 443).await.unwrap();
    for n in 1..=3 {
        let _ = resolve(&guard, &format!("bad-{n}"), 443).await;
    }
    let before = resolver.names().len();
    let hits = guard.stats().hits;
    assert_eq!(resolve(&guard, "ok.example", 443).await.unwrap(), addr(443));
    assert_eq!(resolver.names().len(), before);
    assert_eq!(guard.stats().hits, hits + 1);
}

#[tokio::test]
async fn b5_permanent_failure_breaks_the_temporary_streak() {
    let (guard, _, _) = setup();
    for host in ["bad-1", "bad-2"] {
        let _ = resolve(&guard, host, 443).await;
    }
    assert_eq!(
        code(resolve(&guard, "gone.example", 443).await),
        libc::EAI_NONAME
    );
    assert_eq!(code(resolve(&guard, "bad-3", 443).await), libc::EAI_AGAIN);
    assert_eq!(guard.stats().failures, 4);
    assert_eq!(guard.stats().breaker_trips, 0);
}

#[tokio::test]
async fn b6_stale_is_reused_after_each_failed_refresh_and_can_trip_breaker() {
    let (guard, resolver, clock) = setup();
    resolve(&guard, "stale.example", 443).await.unwrap();
    resolver.stale_fails.store(true, Ordering::SeqCst);
    for step in 1..=3 {
        clock.advance(Duration::from_secs(300));
        assert_eq!(
            resolve(&guard, "stale.example", 443).await.unwrap(),
            addr(443)
        );
        if step < 3 {
            clock.advance(Duration::from_secs(15));
        }
    }
    assert_eq!(guard.stats().failures, 3);
    assert_eq!(guard.stats().stale, 3);
    assert_eq!(guard.stats().breaker_trips, 1);
}

#[tokio::test]
async fn b7_token_bucket_paces_aggregate_queries() {
    let resolver = Arc::new(FakeResolver::default());
    let clock = Arc::new(TestClock::default());
    let mut configured = settings();
    configured.dns_per_second = 10.0;
    configured.dns_burst = 2.0;
    let guard = NetworkGuard::new(configured, resolver.clone(), clock.clone());
    for n in 0..12 {
        resolve(&guard, &format!("rate-{n}.example"), 443)
            .await
            .unwrap();
    }
    let total: f64 = clock.waits().iter().map(Duration::as_secs_f64).sum();
    assert!(
        (total - 1.0).abs() <= 0.01,
        "aggregate pacing time was {total}"
    );
    assert_eq!(resolver.names().len(), 12);
}

#[tokio::test]
async fn b8_b9_socket_permits_are_held_until_close_and_released_on_error() {
    #[derive(Default)]
    struct Response;
    impl HttpResponse for Response {
        fn close(&mut self) {}
    }
    let gate = SocketGate::new(3);
    let p1 = gate.acquire().await;
    let p2 = gate.acquire().await;
    let p3 = gate.acquire().await;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let other = gate.clone();
    tokio::spawn(async move {
        let _p = other.acquire().await;
        let _ = tx.send(());
    });
    tokio::pin!(rx);
    tokio::select! { biased; _ = &mut rx => panic!("fourth permit passed the cap"), _ = tokio::task::yield_now() => {} }
    drop(gate.wrap(Response, p1));
    tokio::time::timeout(Duration::from_secs(1), &mut rx)
        .await
        .unwrap()
        .unwrap();
    drop((p2, p3));
    let single = SocketGate::new(1);
    let p = single.acquire().await;
    drop(p); // request error releases its RAII permit
    let _second = tokio::time::timeout(Duration::from_secs(1), single.acquire())
        .await
        .unwrap();
}

#[derive(Debug)]
enum MockError {
    Dns(DnsError),
    Timeout,
}
impl fmt::Display for MockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dns(error) => error.fmt(f),
            Self::Timeout => f.write_str("timed out"),
        }
    }
}
impl Error for MockError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Dns(e) => Some(e),
            _ => None,
        }
    }
}

#[tokio::test]
async fn b10_deep_dns_and_noname_classify_by_error_type() {
    #[derive(Debug)]
    struct Wrap(Box<dyn Error + Send + Sync>);
    impl fmt::Display for Wrap {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }
    impl Error for Wrap {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(self.0.as_ref())
        }
    }
    let deep = Wrap(Box::new(Wrap(Box::new(DnsError::new(
        libc::EAI_AGAIN,
        "x",
    )))));
    assert!(is_dns_error(&deep));
    assert!(is_dns_error(&DnsError::new(
        libc::EAI_NONAME,
        "no such host"
    )));
}

#[tokio::test]
async fn b11_b12_b13_get_retry_policy_matches_python() {
    let clock = TestClock::default();
    let attempts = AtomicUsize::new(0);
    let reports = Mutex::new(Vec::new());
    let result = get_with_retries(
        "https://host",
        2,
        &clock,
        || {
            attempts.fetch_add(1, Ordering::SeqCst);
            std::future::ready::<Result<String, MockError>>(Err(MockError::Dns(DnsError::new(
                libc::EAI_AGAIN,
                "refused",
            ))))
        },
        |url, e| reports.lock().unwrap().push(fetch_failure_line(url, e)),
    )
    .await;
    assert_eq!(result, None);
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert!(clock.waits().is_empty());
    assert_eq!(
        reports.lock().unwrap().as_slice(),
        ["[scan] fetch failed: https://host (refused)"]
    );

    clock.waits.lock().unwrap().clear();
    attempts.store(0, Ordering::SeqCst);
    let result = get_with_retries(
        "https://host",
        2,
        &clock,
        || {
            attempts.fetch_add(1, Ordering::SeqCst);
            std::future::ready::<Result<String, MockError>>(Err(MockError::Timeout))
        },
        |_, _| {},
    )
    .await;
    assert_eq!(result, None);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(
        clock.waits(),
        [Duration::from_millis(1500), Duration::from_secs(3)]
    );

    clock.waits.lock().unwrap().clear();
    attempts.store(0, Ordering::SeqCst);
    let result = get_with_retries(
        "https://host",
        2,
        &clock,
        || {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            std::future::ready(if attempt == 0 {
                Err(MockError::Timeout)
            } else {
                Err(MockError::Dns(DnsError::new(libc::EAI_NONAME, "gone")))
            })
        },
        |_, _| {},
    )
    .await;
    assert_eq!(result, None);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(clock.waits(), [Duration::from_millis(1500)]);
}

#[test]
fn b14_settings_precedence_casts_and_clamps() {
    fn load(yaml: &str, envs: &[(&str, &str)]) -> NetworkSettings {
        let root: serde_yaml::Value = serde_yaml::from_str(yaml).unwrap();
        let network = root
            .as_mapping()
            .and_then(|m| m.get(serde_yaml::Value::String("network".into())));
        NetworkSettings::from_yaml_and_env(network, |key| {
            envs.iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned())
        })
    }
    assert_eq!(load("network: {max_sockets: 0}", &[]).max_sockets, 1);
    assert_eq!(load("network: {max_sockets: -5}", &[]).max_sockets, 1);
    assert_eq!(load("network: {max_sockets: 32.9}", &[]).max_sockets, 32);
    assert_eq!(load("network: {max_sockets: '16'}", &[]).max_sockets, 16);
    assert_eq!(
        load(
            "network: {max_sockets: abc}",
            &[("JOBSCAN_MAX_SOCKETS", "8")]
        )
        .max_sockets,
        32
    );
    assert_eq!(
        load("network: {}", &[("JOBSCAN_MAX_SOCKETS", "8")]).max_sockets,
        8
    );
    assert_eq!(
        load(
            "network: {max_sockets: 16}",
            &[("JOBSCAN_MAX_SOCKETS", "8")]
        )
        .max_sockets,
        16
    );
    assert_eq!(
        load("network: {dns_per_second: 0}", &[]).dns_per_second,
        0.1
    );
    assert_eq!(
        load("network: {dns_per_second: -2}", &[]).dns_per_second,
        0.1
    );
    assert_eq!(
        load("network: {dns_cache_seconds: -1}", &[]).dns_cache_seconds,
        0.0
    );
    assert_eq!(
        load("network: {dns_nx_seconds: 0}", &[]).dns_nx_seconds,
        1.0
    );
    assert_eq!(load("network: {dns_burst: 0}", &[]).dns_burst, 1.0);
    assert_eq!(
        load("network: {dns_breaker_failures: 0}", &[]).dns_breaker_failures,
        1
    );
    assert_eq!(
        load("network: {dns_breaker_seconds: 0}", &[]).dns_breaker_seconds,
        1.0
    );
    assert_eq!(load("{}", &[]), NetworkSettings::default());
    assert_eq!(
        load("network: {}", &[("JOBSCAN_DNS_RATE", "25")]).dns_per_second,
        25.0
    );
    assert_eq!(
        load("network: {}", &[("JOBSCAN_DNS_RATE", "")]).dns_per_second,
        10.0
    );
    assert_eq!(
        load(
            "network: {dns_per_second: null}",
            &[("JOBSCAN_DNS_RATE", "25")]
        )
        .dns_per_second,
        25.0
    );
}

#[test]
fn b15_stats_line_format_is_exact() {
    let stats = Stats {
        misses: 1234,
        hits: 5678,
        stale: 9,
        failures: 10,
        breaker_trips: 1,
        breaker_skips: 0,
        max_sockets: 32,
        dns_per_second: 10.0,
    };
    assert_eq!(
        stats.line(),
        "net: 1234 lookups, 5678 cached, 9 stale, 10 failed, 1 resolver pauses (<= 32 sockets, 10 DNS/s)"
    );
}

#[tokio::test]
async fn b16_single_flight_resolves_a_contended_key_once() {
    let (guard, resolver, _) = setup();
    resolver.delay.store(true, Ordering::SeqCst);
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let g = guard.clone();
        tasks.push(tokio::spawn(async move {
            resolve(&g, "contended.example", 443).await.unwrap()
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap(), addr(443));
    }
    assert_eq!(resolver.names().len(), 1);
    assert_eq!(guard.stats().hits, 7);
    assert_eq!(guard.stats().misses, 1);
}

#[tokio::test]
async fn b17_bypasses_ip_flags_and_empty_host_without_counters() {
    let (guard, resolver, _) = setup();
    resolve(&guard, "2001:db8::1", 443).await.unwrap();
    guard
        .resolve("flags.example", 443, 0, 0, 0, libc::AI_NUMERICHOST)
        .await
        .unwrap();
    guard.resolve("", 443, 0, 0, 0, 0).await.unwrap();
    assert_eq!(resolver.names(), ["2001:db8::1", "flags.example", ""]);
    let stats = guard.stats();
    assert_eq!(
        (
            stats.hits,
            stats.misses,
            stats.stale,
            stats.failures,
            stats.breaker_trips,
            stats.breaker_skips
        ),
        (0, 0, 0, 0, 0, 0)
    );
    assert_eq!((stats.max_sockets, stats.dns_per_second), (3, 1_000_000.0));
}

#[tokio::test]
async fn b18_key_preserves_family_and_hostname_case() {
    let (guard, resolver, _) = setup();
    guard
        .resolve("same.example", 443, libc::AF_INET, 0, 0, 0)
        .await
        .unwrap();
    guard
        .resolve("same.example", 443, libc::AF_INET6, 0, 0, 0)
        .await
        .unwrap();
    resolve(&guard, "Example.COM", 443).await.unwrap();
    resolve(&guard, "example.com", 443).await.unwrap();
    assert_eq!(
        resolver.names(),
        ["same.example", "same.example", "Example.COM", "example.com"]
    );
}

#[tokio::test]
async fn b19_reset_clears_cache_counters_breaker_and_refills_bucket() {
    let (guard, _, _) = setup();
    resolve(&guard, "reset.example", 443).await.unwrap();
    for n in 1..=3 {
        let _ = resolve(&guard, &format!("bad-{n}"), 443).await;
    }
    guard.bucket.state.lock().unwrap().0 = 0.0;
    guard.reset();
    let stats = guard.stats();
    assert_eq!(
        (
            stats.hits,
            stats.misses,
            stats.stale,
            stats.failures,
            stats.breaker_trips,
            stats.breaker_skips
        ),
        (0, 0, 0, 0, 0, 0)
    );
    assert_eq!((stats.max_sockets, stats.dns_per_second), (3, 1_000_000.0));
    assert!(guard.state.lock().unwrap().cache.is_empty());
    assert_eq!(guard.state.lock().unwrap().breaker_until, Duration::ZERO);
    assert_eq!(guard.bucket.state.lock().unwrap().0, settings().dns_burst);
}

#[test]
fn b20_default_get_headers_and_timeout_are_exact() {
    assert_eq!(
        DEFAULT_HEADERS,
        [
            (
                "User-Agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36"
            ),
            ("Accept-Language", "en-CA,en;q=0.9"),
            ("Accept", "*/*"),
        ]
    );
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(30));
}
