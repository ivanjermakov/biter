# Biter

BitTorrent client written in Rust

## Features

| Description                   | BEP                                                       | Status                    |
| ---                           | ---                                                       | ---                       |
| BitTorrent Protocol           | [BEP-3](https://www.bittorrent.org/beps/bep_0003.html)    | ✅[^1]                    |
| DHT Protocol                  | [BEP-5](https://www.bittorrent.org/beps/bep_0005.html)    | ✅[^2]                    |
| Peers to Send Metadata Files  | [BEP-9](https://www.bittorrent.org/beps/bep_0009.html)    | 🚧                        |
| Extension Protocol            | [BEP-10](https://www.bittorrent.org/beps/bep_0010.html)   | ✅                        |
| UDP Tracker Protocol          | [BEP-15](https://www.bittorrent.org/beps/bep_0015.html)   | ✅                        |
| Magnet URI                    | [BEP-53](https://www.bittorrent.org/beps/bep_0035.html)   | 🚧                        |

[^1]: no seeding, requesting only
[^2]: no routing, `find_peers` only

## Reference

Specs: https://www.bittorrent.org/beps
