# Biter

BitTorrent client written in Rust

## Features

| Description                               | BEP                                                       | Status                    |
| ---                                       | ---                                                       | ---                       |
| BitTorrent Protocol                       | [BEP-3](https://www.bittorrent.org/beps/bep_0003.html)    | ✅[^1]                    |
| DHT Protocol                              | [BEP-5](https://www.bittorrent.org/beps/bep_0005.html)    | ✅[^2]                    |
| Metadata from peers and magnet URLs       | [BEP-9](https://www.bittorrent.org/beps/bep_0009.html)    | ✅[^3][^4][^5]            |
| Extension Protocol                        | [BEP-10](https://www.bittorrent.org/beps/bep_0010.html)   | ✅                        |
| Peer Exchange (PEX)                       | [BEP-55](https://www.bittorrent.org/beps/bep_0011.html)   | 🚧                        |
| UDP Tracker Protocol                      | [BEP-15](https://www.bittorrent.org/beps/bep_0015.html)   | ✅                        |
| Holepunch extension                       | [BEP-55](https://www.bittorrent.org/beps/bep_0055.html)   | 🚧                        |

[^1]: no seeding, requesting only
[^2]: no routing, `find_peers` only
[^3]: no metadata seeding
[^4]: only reading `info_hash` from magnet
[^5]: v1 magnets only

## Reference

Specs: https://www.bittorrent.org/beps
