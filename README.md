# angzarr-client-rust

Rust client library for Angzarr event sourcing framework.

## Installation

```
cargo add angzarr-client
```

## Usage

```
use angzarr_client::Client;

let client = Client::connect("http://localhost:50051").await?;
```

## License

BSD-3-Clause
