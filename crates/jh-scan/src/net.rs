use std::{
    collections::HashMap,
    error::Error,
    ffi::{CStr, CString},
    fmt,
    future::Future,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, Semaphore};

type Sleep = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";
pub const DEFAULT_HEADERS: [(&str, &str); 3] = [
    ("User-Agent", USER_AGENT),
    ("Accept-Language", "en-CA,en;q=0.9"),
    ("Accept", "*/*"),
];
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

pub trait Clock: Send + Sync {
    fn now(&self) -> Duration;
    fn sleep(&self, duration: Duration) -> Sleep;
}

#[derive(Clone)]
pub struct SystemClock(Arc<Instant>);

impl Default for SystemClock {
    fn default() -> Self {
        Self(Arc::new(Instant::now()))
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }

    fn sleep(&self, duration: Duration) -> Sleep {
        Box::pin(tokio::time::sleep(duration))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NetworkSettings {
    pub max_sockets: usize,
    pub dns_cache_seconds: f64,
    pub dns_nx_seconds: f64,
    pub dns_per_second: f64,
    pub dns_burst: f64,
    pub dns_breaker_failures: u32,
    pub dns_breaker_seconds: f64,
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Self {
            max_sockets: 32,
            dns_cache_seconds: 300.0,
            dns_nx_seconds: 15.0,
            dns_per_second: 10.0,
            dns_burst: 20.0,
            dns_breaker_failures: 12,
            dns_breaker_seconds: 60.0,
        }
    }
}

impl NetworkSettings {
    /// Resolve settings from the YAML `network` mapping, then an injected environment lookup.
    /// Invalid present YAML values use defaults and do not fall through to the environment.
    pub fn from_yaml_and_env(
        network: Option<&serde_yaml::Value>,
        mut env: impl FnMut(&str) -> Option<String>,
    ) -> Self {
        use serde_yaml::Value;
        let get = |key: &str| {
            network
                .and_then(Value::as_mapping)
                .and_then(|m| m.get(Value::String(key.to_owned())))
                .filter(|v| !v.is_null())
                .cloned()
        };
        let mut value = Self::default();
        macro_rules! number {
            ($field:ident, $yaml:literal, $env:literal, $cast:ident, $default:expr, $clamp:expr) => {{
                let raw = get($yaml).or_else(|| env($env).map(Value::String));
                let parsed = raw
                    .as_ref()
                    .and_then(parse_number::<$cast>)
                    .unwrap_or($default);
                value.$field = ($clamp)(parsed);
            }};
        }
        number!(
            max_sockets,
            "max_sockets",
            "JOBSCAN_MAX_SOCKETS",
            i64,
            32,
            |n: i64| n.max(1) as usize
        );
        number!(
            dns_cache_seconds,
            "dns_cache_seconds",
            "JOBSCAN_DNS_TTL",
            f64,
            300.0,
            |n: f64| n.max(0.0)
        );
        number!(
            dns_nx_seconds,
            "dns_nx_seconds",
            "JOBSCAN_DNS_NX_TTL",
            f64,
            15.0,
            |n: f64| n.max(1.0)
        );
        number!(
            dns_per_second,
            "dns_per_second",
            "JOBSCAN_DNS_RATE",
            f64,
            10.0,
            |n: f64| n.max(0.1)
        );
        number!(
            dns_burst,
            "dns_burst",
            "JOBSCAN_DNS_BURST",
            f64,
            20.0,
            |n: f64| n.max(1.0)
        );
        number!(
            dns_breaker_failures,
            "dns_breaker_failures",
            "JOBSCAN_DNS_BREAKER_FAILURES",
            i64,
            12,
            |n: i64| n.max(1) as u32
        );
        number!(
            dns_breaker_seconds,
            "dns_breaker_seconds",
            "JOBSCAN_DNS_BREAKER_SECONDS",
            f64,
            60.0,
            |n: f64| n.max(1.0)
        );
        value
    }

    pub fn from_sources_file(
        path: &std::path::Path,
        mut env: impl FnMut(&str) -> Option<String>,
    ) -> Self {
        let network = std::fs::read_to_string(path)
            .ok()
            .and_then(|contents| serde_yaml::from_str::<serde_yaml::Value>(&contents).ok())
            .and_then(|root| root.get("network").cloned());
        Self::from_yaml_and_env(network.as_ref(), |key| env(key))
    }

    pub fn from_environment(path: &std::path::Path) -> Self {
        Self::from_sources_file(path, |key| std::env::var(key).ok())
    }
}

fn parse_number<T: std::str::FromStr>(value: &serde_yaml::Value) -> Option<T> {
    match value {
        serde_yaml::Value::Number(n) => n.to_string().parse().ok().or_else(|| {
            // Python int(float) truncates, while float parsing accepts YAML's integer numbers.
            n.as_f64()
                .map(|f| f.trunc().to_string())
                .and_then(|s| s.parse().ok())
        }),
        serde_yaml::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DnsError {
    pub code: i32,
    pub message: String,
}

impl DnsError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for DnsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl Error for DnsError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    Dns(DnsError),
    Other(String),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dns(e) => e.fmt(f),
            Self::Other(e) => f.write_str(e),
        }
    }
}
impl Error for ResolveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Dns(e) => Some(e),
            Self::Other(_) => None,
        }
    }
}

pub trait Resolver: Send + Sync {
    fn getaddrinfo(
        &self,
        host: &str,
        port: u16,
        family: i32,
        sock_type: i32,
        proto: i32,
        flags: i32,
    ) -> Result<Vec<SocketAddr>, ResolveError>;
}

#[derive(Default)]
pub struct SystemResolver;

impl Resolver for SystemResolver {
    fn getaddrinfo(
        &self,
        host: &str,
        port: u16,
        family: i32,
        sock_type: i32,
        proto: i32,
        flags: i32,
    ) -> Result<Vec<SocketAddr>, ResolveError> {
        let host = CString::new(host).map_err(|e| ResolveError::Other(e.to_string()))?;
        let service = CString::new(port.to_string()).expect("numeric service has no NUL");
        let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
        hints.ai_family = family;
        hints.ai_socktype = sock_type;
        hints.ai_protocol = proto;
        hints.ai_flags = flags;
        let mut result = std::ptr::null_mut();
        let code =
            unsafe { libc::getaddrinfo(host.as_ptr(), service.as_ptr(), &hints, &mut result) };
        if code != 0 {
            let message = unsafe { CStr::from_ptr(libc::gai_strerror(code)) }
                .to_string_lossy()
                .into_owned();
            return Err(ResolveError::Dns(DnsError::new(code, message)));
        }
        let mut addresses = Vec::new();
        let mut current = result;
        while !current.is_null() {
            let item = unsafe { &*current };
            if !item.ai_addr.is_null() {
                let addr = unsafe {
                    match item.ai_family {
                        libc::AF_INET
                            if item.ai_addrlen as usize
                                >= std::mem::size_of::<libc::sockaddr_in>() =>
                        {
                            let sa = &*(item.ai_addr.cast::<libc::sockaddr_in>());
                            Some(SocketAddr::new(
                                IpAddr::V4(std::net::Ipv4Addr::from(u32::from_be(
                                    sa.sin_addr.s_addr,
                                ))),
                                u16::from_be(sa.sin_port),
                            ))
                        }
                        libc::AF_INET6
                            if item.ai_addrlen as usize
                                >= std::mem::size_of::<libc::sockaddr_in6>() =>
                        {
                            let sa = &*(item.ai_addr.cast::<libc::sockaddr_in6>());
                            Some(SocketAddr::new(
                                IpAddr::V6(std::net::Ipv6Addr::from(sa.sin6_addr.s6_addr)),
                                u16::from_be(sa.sin6_port),
                            ))
                        }
                        _ => None,
                    }
                };
                if let Some(addr) = addr {
                    addresses.push(addr);
                }
            }
            current = item.ai_next;
        }
        unsafe { libc::freeaddrinfo(result) };
        Ok(addresses)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub hits: u64,
    pub misses: u64,
    pub stale: u64,
    pub failures: u64,
    pub breaker_trips: u64,
    pub breaker_skips: u64,
    pub max_sockets: usize,
    pub dns_per_second: f64,
}

impl Stats {
    pub fn line(self) -> String {
        format!(
            "net: {} lookups, {} cached, {} stale, {} failed, {} resolver pauses (<= {} sockets, {:.0} DNS/s)",
            self.misses,
            self.hits,
            self.stale,
            self.failures,
            self.breaker_trips,
            self.max_sockets,
            self.dns_per_second
        )
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
pub(crate) struct CacheKey {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) family: i32,
    pub(crate) sock_type: i32,
    pub(crate) proto: i32,
}
#[derive(Clone)]
pub(crate) struct CacheEntry {
    pub(crate) expires: Duration,
    pub(crate) addresses: Option<Vec<SocketAddr>>,
    pub(crate) errno: Option<i32>,
}
#[derive(Default)]
pub(crate) struct State {
    pub(crate) cache: HashMap<CacheKey, CacheEntry>,
    pub(crate) stats: Stats,
    pub(crate) fail_streak: u32,
    pub(crate) breaker_until: Duration,
}

pub(crate) struct Bucket {
    rate: f64,
    capacity: f64,
    pub(crate) state: Mutex<(f64, Duration)>,
}

impl Bucket {
    fn new(rate: f64, capacity: f64, now: Duration) -> Self {
        Self {
            rate,
            capacity,
            state: Mutex::new((capacity, now)),
        }
    }
    async fn take(&self, clock: &dyn Clock) {
        loop {
            let wait = {
                let mut state = self.state.lock().expect("bucket mutex poisoned");
                let now = clock.now();
                let elapsed = now.saturating_sub(state.1).as_secs_f64();
                state.0 = (state.0 + elapsed * self.rate).min(self.capacity);
                state.1 = now;
                if state.0 >= 1.0 {
                    state.0 -= 1.0;
                    return;
                }
                ((1.0 - state.0) / self.rate).min(1.0)
            };
            clock.sleep(Duration::from_secs_f64(wait)).await;
        }
    }
    fn refill(&self, now: Duration) {
        let mut state = self.state.lock().expect("bucket mutex poisoned");
        *state = (self.capacity, now);
    }
}

pub struct NetworkGuard {
    pub(crate) settings: NetworkSettings,
    resolver: Arc<dyn Resolver>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) state: Mutex<State>,
    host_locks: Mutex<HashMap<CacheKey, Arc<AsyncMutex<()>>>>,
    pub(crate) bucket: Bucket,
}

impl NetworkGuard {
    pub fn new(
        settings: NetworkSettings,
        resolver: Arc<dyn Resolver>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let bucket = Bucket::new(settings.dns_per_second, settings.dns_burst, clock.now());
        Self {
            settings,
            resolver,
            clock,
            state: Mutex::new(State::default()),
            host_locks: Mutex::new(HashMap::new()),
            bucket,
        }
    }
    pub fn settings(&self) -> &NetworkSettings {
        &self.settings
    }
    pub fn stats(&self) -> Stats {
        let mut stats = self.state.lock().expect("guard mutex poisoned").stats;
        stats.max_sockets = self.settings.max_sockets;
        stats.dns_per_second = self.settings.dns_per_second;
        stats
    }
    pub fn line(&self) -> String {
        self.stats().line()
    }
    pub fn log_line_if_used(&self) -> bool {
        let stats = self.stats();
        if stats.misses == 0 && stats.hits == 0 {
            return false;
        }
        eprintln!("[scan] {}", stats.line());
        true
    }
    pub fn reset(&self) {
        *self.state.lock().expect("guard mutex poisoned") = State::default();
        self.host_locks
            .lock()
            .expect("host-lock mutex poisoned")
            .clear();
        self.bucket.refill(self.clock.now());
    }
    pub async fn resolve(
        &self,
        host: &str,
        port: u16,
        family: i32,
        sock_type: i32,
        proto: i32,
        flags: i32,
    ) -> Result<Vec<SocketAddr>, ResolveError> {
        if host.is_empty() || flags != 0 || host.parse::<IpAddr>().is_ok() {
            return self
                .real_resolve(host.to_owned(), port, family, sock_type, proto, flags)
                .await;
        }
        let key = CacheKey {
            host: host.to_owned(),
            port,
            family,
            sock_type,
            proto,
        };
        if let Some(entry) = self.fresh(&key) {
            return Self::serve(entry, &self.state);
        }
        let lock = {
            let mut locks = self.host_locks.lock().expect("host-lock mutex poisoned");
            locks
                .entry(key.clone())
                .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                .clone()
        };
        let _singleflight = lock.lock().await;
        let now = self.clock.now();
        if let Some(entry) = self.fresh_at(&key, now) {
            return Self::serve(entry, &self.state);
        }
        let stale = self
            .state
            .lock()
            .expect("guard mutex poisoned")
            .cache
            .get(&key)
            .cloned();
        let paused = {
            let state = self.state.lock().expect("guard mutex poisoned");
            now < state.breaker_until
        };
        if paused {
            let stale_addresses = stale.and_then(|e| e.addresses);
            let mut state = self.state.lock().expect("guard mutex poisoned");
            state.stats.breaker_skips += 1;
            if let Some(addresses) = stale_addresses {
                state.stats.stale += 1;
                state.cache.insert(
                    key,
                    CacheEntry {
                        expires: self.clock.now() + secs(self.settings.dns_nx_seconds),
                        addresses: Some(addresses.clone()),
                        errno: None,
                    },
                );
                return Ok(addresses);
            }
            return Err(ResolveError::Dns(DnsError::new(
                libc::EAI_AGAIN,
                "name resolution paused: the resolver was refusing lookups",
            )));
        }
        self.bucket.take(self.clock.as_ref()).await;
        self.state
            .lock()
            .expect("guard mutex poisoned")
            .stats
            .misses += 1;
        match self
            .real_resolve(host.to_owned(), port, family, sock_type, proto, 0)
            .await
        {
            Ok(addresses) => {
                let mut state = self.state.lock().expect("guard mutex poisoned");
                state.cache.insert(
                    key,
                    CacheEntry {
                        expires: self.clock.now() + secs(self.settings.dns_cache_seconds),
                        addresses: Some(addresses.clone()),
                        errno: None,
                    },
                );
                state.fail_streak = 0;
                state.breaker_until = Duration::ZERO;
                Ok(addresses)
            }
            Err(ResolveError::Dns(dns)) => {
                let now = self.clock.now();
                let stale_addresses = stale.and_then(|e| e.addresses);
                let mut state = self.state.lock().expect("guard mutex poisoned");
                state.stats.failures += 1;
                if is_temporary(dns.code) {
                    state.fail_streak += 1;
                    if state.fail_streak >= self.settings.dns_breaker_failures {
                        state.fail_streak = 0;
                        state.breaker_until = now + secs(self.settings.dns_breaker_seconds);
                        state.stats.breaker_trips += 1;
                        eprintln!(
                            "[scan] net: the resolver refused {} lookups in a row; pausing DNS for {:.0}s so the rate limit can clear",
                            self.settings.dns_breaker_failures, self.settings.dns_breaker_seconds
                        );
                    }
                } else {
                    state.fail_streak = 0;
                }
                if let Some(addresses) = stale_addresses {
                    state.stats.stale += 1;
                    state.cache.insert(
                        key,
                        CacheEntry {
                            expires: now + secs(self.settings.dns_nx_seconds),
                            addresses: Some(addresses.clone()),
                            errno: None,
                        },
                    );
                    Ok(addresses)
                } else {
                    state.cache.insert(
                        key,
                        CacheEntry {
                            expires: now + secs(self.settings.dns_nx_seconds),
                            addresses: None,
                            errno: Some(dns.code),
                        },
                    );
                    Err(ResolveError::Dns(dns))
                }
            }
            Err(other) => Err(other),
        }
    }
    fn fresh(&self, key: &CacheKey) -> Option<CacheEntry> {
        self.fresh_at(key, self.clock.now())
    }
    fn fresh_at(&self, key: &CacheKey, now: Duration) -> Option<CacheEntry> {
        self.state
            .lock()
            .expect("guard mutex poisoned")
            .cache
            .get(key)
            .filter(|e| e.expires > now)
            .cloned()
    }
    fn serve(entry: CacheEntry, state: &Mutex<State>) -> Result<Vec<SocketAddr>, ResolveError> {
        state.lock().expect("guard mutex poisoned").stats.hits += 1;
        match entry.addresses {
            Some(addresses) => Ok(addresses),
            None => Err(ResolveError::Dns(DnsError::new(
                entry.errno.unwrap_or(libc::EAI_AGAIN),
                "cached name-resolution failure",
            ))),
        }
    }
    async fn real_resolve(
        &self,
        host: String,
        port: u16,
        family: i32,
        sock_type: i32,
        proto: i32,
        flags: i32,
    ) -> Result<Vec<SocketAddr>, ResolveError> {
        let resolver = self.resolver.clone();
        tokio::task::spawn_blocking(move || {
            resolver.getaddrinfo(&host, port, family, sock_type, proto, flags)
        })
        .await
        .map_err(|e| ResolveError::Other(format!("resolver task failed: {e}")))?
    }
}

fn secs(value: f64) -> Duration {
    Duration::from_secs_f64(value.max(0.0))
}
fn is_temporary(code: i32) -> bool {
    code == libc::EAI_AGAIN || code == libc::EAI_FAIL || code == libc::EAI_SYSTEM
}

pub fn is_dns_error(error: &(dyn Error + 'static)) -> bool {
    let mut current = Some(error);
    let mut depth = 0;
    while let Some(err) = current {
        if err.downcast_ref::<DnsError>().is_some() {
            return true;
        }
        current = err.source();
        depth += 1;
        if depth >= 32 {
            return false;
        }
    }
    false
}

/// Python `core.http_get` retry policy: DNS errors stop immediately; other failures use
/// 1.5 s, 3.0 s, ... delays and report exactly once after exhausting attempts.
pub async fn get_with_retries<E, F, Fut>(
    url: &str,
    retries: usize,
    clock: &dyn Clock,
    mut fetch: F,
    mut report: impl FnMut(&str, &E),
) -> Option<String>
where
    E: Error + Send + Sync + 'static,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<String, E>>,
{
    let mut last = None;
    for attempt in 0..=retries {
        match fetch().await {
            Ok(body) => return Some(body),
            Err(error) => {
                let stop = is_dns_error(&error) || attempt >= retries;
                last = Some(error);
                if stop {
                    break;
                }
                clock
                    .sleep(Duration::from_secs_f64(1.5 * (attempt + 1) as f64))
                    .await;
            }
        }
    }
    if let Some(error) = last.as_ref() {
        report(url, error);
    }
    None
}

pub fn fetch_failure_line(url: &str, error: &impl fmt::Display) -> String {
    format!("[scan] fetch failed: {url} ({error})")
}

struct GuardResolver {
    guard: Arc<NetworkGuard>,
}

impl reqwest::dns::Resolve for GuardResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        // Resolver calls use flags=0: non-zero flags intentionally bypass the cache.
        let guard = self.guard.clone();
        Box::pin(async move {
            let addresses = guard
                .resolve(&host, 0, libc::AF_UNSPEC, libc::SOCK_STREAM, 0, 0)
                .await
                .map_err(|e| -> Box<dyn Error + Send + Sync> { Box::new(e) })?;
            let addrs: reqwest::dns::Addrs = Box::new(addresses.into_iter());
            Ok(addrs)
        })
    }
}

/// Owns the single guard and the only HTTP client used by all scan sources.
pub struct HttpClient {
    pub guard: Arc<NetworkGuard>,
    client: reqwest::Client,
    sockets: SocketGate,
}

/// Shared scan dependencies. Sources receive clones of this context, never new clients.
#[derive(Clone)]
pub struct ScanContext {
    pub http: Arc<HttpClient>,
}

impl ScanContext {
    pub fn new(http: HttpClient) -> Self {
        Self {
            http: Arc::new(http),
        }
    }
}

impl HttpClient {
    pub fn new(
        settings: NetworkSettings,
        resolver: Arc<dyn Resolver>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, reqwest::Error> {
        let guard = Arc::new(NetworkGuard::new(settings.clone(), resolver, clock));
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(GuardResolver {
                guard: guard.clone(),
            }))
            .timeout(DEFAULT_TIMEOUT)
            .pool_max_idle_per_host(0)
            .user_agent(USER_AGENT)
            .build()?;
        Ok(Self {
            guard,
            client,
            sockets: SocketGate::new(settings.max_sockets),
        })
    }
    pub fn from_sources_file(path: &std::path::Path) -> Result<Self, reqwest::Error> {
        Self::new(
            NetworkSettings::from_environment(path),
            Arc::new(SystemResolver),
            Arc::new(SystemClock::default()),
        )
    }
    pub async fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<GuardedHttpResponse, reqwest::Error> {
        let permit = self.sockets.acquire().await;
        let mut request = self
            .client
            .get(url)
            .header("Accept-Language", DEFAULT_HEADERS[1].1)
            .header("Accept", DEFAULT_HEADERS[2].1);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.send().await?.error_for_status()?;
        Ok(GuardedHttpResponse {
            response: Some(response),
            _permit: Some(permit),
        })
    }
    pub async fn get_text(&self, url: &str, headers: &[(&str, &str)]) -> Option<String> {
        self.get_text_with_retries(url, headers, 2).await
    }
    pub async fn get_text_with_retries(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        retries: usize,
    ) -> Option<String> {
        get_with_retries::<reqwest::Error, _, _>(
            url,
            retries,
            self.guard.clock.as_ref(),
            || async {
                let mut response = self.get(url, headers).await?;
                let body = response.bytes().await?;
                Ok(String::from_utf8_lossy(&body).into_owned())
            },
            |url, error| eprintln!("{}", fetch_failure_line(url, error)),
        )
        .await
    }
    pub fn settings(&self) -> &NetworkSettings {
        self.guard.settings()
    }
}

pub struct GuardedHttpResponse {
    response: Option<reqwest::Response>,
    _permit: Option<OwnedSemaphorePermit>,
}
impl GuardedHttpResponse {
    pub fn status(&self) -> reqwest::StatusCode {
        self.response
            .as_ref()
            .expect("response already closed")
            .status()
    }
    pub async fn bytes(&mut self) -> Result<Vec<u8>, reqwest::Error> {
        let response = self.response.take().expect("response already closed");
        let result = response.bytes().await.map(|body| body.to_vec());
        self._permit.take();
        result
    }
    pub fn close(&mut self) {
        self.response.take();
        self._permit.take();
    }
}

/// Process-wide ceiling for requests whose responses are still in use.
#[derive(Clone)]
pub struct SocketGate(Arc<Semaphore>);
impl SocketGate {
    pub fn new(maximum: usize) -> Self {
        Self(Arc::new(Semaphore::new(maximum.max(1))))
    }
    pub async fn acquire(&self) -> OwnedSemaphorePermit {
        self.0
            .clone()
            .acquire_owned()
            .await
            .expect("socket gate is never closed")
    }
    pub fn wrap<R: HttpResponse>(
        &self,
        response: R,
        permit: OwnedSemaphorePermit,
    ) -> GatedResponse<R> {
        GatedResponse::new(response, move || drop(permit))
    }
}

pub trait HttpResponse: Send {
    fn close(&mut self);
}
pub struct GatedResponse<R: HttpResponse> {
    response: Option<R>,
    release: Option<Box<dyn FnOnce() + Send>>,
}
impl<R: HttpResponse> GatedResponse<R> {
    pub fn new(response: R, release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            response: Some(response),
            release: Some(Box::new(release)),
        }
    }
    pub fn response(&self) -> &R {
        self.response.as_ref().expect("response closed")
    }
    pub fn close(&mut self) {
        if let Some(mut response) = self.response.take() {
            response.close();
        }
        if let Some(release) = self.release.take() {
            release();
        }
    }
}
impl<R: HttpResponse> Drop for GatedResponse<R> {
    fn drop(&mut self) {
        self.close();
    }
}
