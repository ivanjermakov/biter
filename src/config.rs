use std::time::Duration;

#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
pub struct Config {
    pub port: u16,
    pub respect_choke: bool,
    pub choke_wait: Duration,
    pub reconnect_wait: Duration,
    pub downloaded_check_wait: Duration,
    pub peer_connect_timeout: Duration,
    pub piece_request_wait: Duration,
    pub dht_chunk: usize,
    pub dht_min_peers: usize,
}
