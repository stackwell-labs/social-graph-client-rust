# social-graph-client

A Rust client for the [Social Graph](https://github.com/coreygirard) service:
the **single source of truth** for its wire API. The request/response shapes
and route paths live here, derived from the real server handlers, so consumers
(Pigeon, Gotta, …) stop re-declaring drifting copies of these DTOs.

## What it does

- Owns the wire types: [`MeResponse`] (`GET /api/me`) and
  [`IsConnectedResponse`] (`GET /internal/graph/is_connected`).
- Owns the route-path constants `ROUTE_ME` and `ROUTE_IS_CONNECTED`.
- Provides a small typed async client over `reqwest`.

## Surfaces

| Endpoint | Credential | Returns |
| --- | --- | --- |
| `GET /api/me` | the caller's own user bearer | the caller's Social Graph `userId` |
| `GET /internal/graph/is_connected?a=&b=` | shared internal-service token | whether two users are connected |

## Usage

```rust
use social_graph_client::SocialGraphClient;

let client = SocialGraphClient::new(
    reqwest::Client::new(),
    "https://socialgraph.example",
);

// Resolve the caller (presenting their own user bearer) to their userId.
let my_id = client.resolve_me("user-bearer-token").await?;

// Ask whether two users are connected, presenting the shared internal token.
let connected = client
    .is_connected("internal-service-token", &my_id, "sg_other")
    .await?;
```

## Error policy

The client does **not** fail-closed: transport and non-success-status errors
are returned as [`Error`], not swallowed. Callers choose their own policy —
e.g. Pigeon treats every error as "not connected" so a Social Graph outage can
never auto-approve a DM. That policy lives at the call site, not here, so
failures stay visible to callers who need them.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
