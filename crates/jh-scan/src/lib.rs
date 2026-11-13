//! The scan process's shared DNS and outbound request guard.

mod net;

pub use net::{
    Clock, DEFAULT_HEADERS, DEFAULT_TIMEOUT, DnsError, GatedResponse, HttpClient, HttpResponse,
    NetworkGuard, NetworkSettings, ResolveError, Resolver, ScanContext, SocketGate, Stats,
    SystemClock, SystemResolver, USER_AGENT, fetch_failure_line, get_with_retries, is_dns_error,
};

#[cfg(test)]
mod tests;
