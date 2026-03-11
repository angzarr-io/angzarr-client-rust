> **⚠️ Notice:** This repository was recently extracted from the [angzarr monorepo](https://github.com/angzarr-io/angzarr) and has not yet been validated as a standalone project. Expect rough edges.

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
