# social-graph-client

A Rust client for the [Social Graph](https://github.com/coreygirard) service:
the **single source of truth** for its wire API. The request/response shapes
and route paths live here, derived from the real server handlers, so consumers
(Pigeon, Gotta, …) stop re-declaring drifting copies of these DTOs.

## What it does

- Owns the wire types and route-path constants (`ROUTE_*`) for Social Graph's
  internal-service surface.
- Provides a small typed async client over `reqwest`.

## Surfaces

Every route takes a chirp machine token from an allowlisted client, and
app-scoped chirp `sub`s as input. None returns a global user id.

| Endpoint | Returns |
| --- | --- |
| `POST /internal/graph/resolve-self` | the app's pseudonym for its own user, or `null` if the sub isn't bridged |
| `POST /internal/graph/resolve-audience` | the viewer's connections, as this app's pseudonyms |
| `POST /internal/graph/connected` | whether two subs are a current connection |
| `POST /internal/graph/connection-display-name` (0.3.0) | the target's display name, or `null` unless the two subs are a current connection |

## Usage

```rust
use social_graph_client::SocialGraphClient;

let client = SocialGraphClient::new(
    reqwest::Client::new(),
    "https://socialgraph.stackwell.tech",
);
let token = "machine-token";

let me = client.resolve_self(token, "sub_app_alice", "my-app").await?;
let connected = client.connected(token, "sub_app_alice", "sub_app_bob").await?;
let name = client
    .connection_display_name(token, "sub_app_alice", "sub_app_bob")
    .await?; // Some("Bob …") only while they're connected
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
