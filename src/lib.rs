//! Shared wire types and a typed async client for the [Social Graph] service.
//!
//! This crate is the **single source of truth** for the Social Graph wire API
//! that peer services consume. Before it existed, the request/response shapes
//! and route paths were re-declared by hand in each consumer (Pigeon, Gotta,
//! …); those copies drift. Anything in this crate is derived from the real
//! server handlers in `social-graph/backend/src/api/social_graph.rs`.
//!
//! # The internal surface
//!
//! All modeled surfaces are **internal-service** endpoints: the caller
//! presents a chirp machine token from an allowlisted client, passes
//! **app-scoped chirp `sub`s** (pairwise per app: a different sub per app for
//! the same human) as input, and gets back a relationship answer. None returns
//! a person's id, global or pseudonymous, so two apps can't correlate the same
//! human through this surface.
//!
//! - **`POST /internal/graph/connected`** → whether two subs are graph-connected
//!   ([`connected`](SocialGraphClient::connected)); resolves both via the
//!   `(chirp_sub -> users.id)` bridge.
//! - **`POST /internal/graph/connection-display-name`** (since 0.3.0) → the
//!   display name of one of the viewer's connections
//!   ([`connection_display_name`](SocialGraphClient::connection_display_name)),
//!   for showing the viewer who did something without handing them an id.
//!   `None` unless the two subs are a current mutual connection.
//!
//! # Retired surfaces
//!
//! - **0.4.0** dropped `resolve_self` and `resolve_audience`
//!   (`/internal/graph/resolve-self`, `/resolve-audience`). They minted
//!   pseudonyms in whatever namespace the request body named, so any
//!   allowlisted caller could read any app's pseudonyms, and Social Graph
//!   retired both routes (social-graph#39). An app gets its own pseudonyms
//!   through Social Graph's graph-access plane, whose namespace is the app's
//!   own grant. They are removed rather than kept, so a stale caller fails to
//!   compile instead of failing at runtime (social-graph#62).
//! - **0.2.0** dropped `GET /api/me` (`resolve_me`) and
//!   `GET /internal/graph/is_connected` (`is_connected`), which handed apps
//!   the global `users.id` and so let two apps correlate a shared user.
//!
//! # Example
//!
//! ```no_run
//! use social_graph_client::SocialGraphClient;
//!
//! # async fn run() -> Result<(), social_graph_client::Error> {
//! let client = SocialGraphClient::new(
//!     reqwest::Client::new(),
//!     "https://socialgraph.example",
//! );
//! let token = "machine-token";
//!
//! // Whether the two app-scoped subs are a current connection.
//! let connected = client.connected(token, "sub_app_alice", "sub_app_bob").await?;
//!
//! // Bob's display name as Alice sees it; `None` unless they're connected.
//! let name = client
//!     .connection_display_name(token, "sub_app_alice", "sub_app_bob")
//!     .await?;
//! # let _ = (connected, name);
//! # Ok(())
//! # }
//! ```
//!
//! [Social Graph]: https://github.com/stackwell-labs

use serde::{Deserialize, Serialize};

/// `POST` route asking whether two app-scoped subs are graph-connected.
pub const ROUTE_CONNECTED: &str = "/internal/graph/connected";

/// `POST` route naming one of the viewer's current connections.
pub const ROUTE_CONNECTION_DISPLAY_NAME: &str = "/internal/graph/connection-display-name";

/// Request body for [`connected`](SocialGraphClient::connected).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectedRequest {
    /// One party's app-scoped chirp `sub`.
    pub viewer_sub: String,
    /// The other party's app-scoped chirp `sub`.
    pub target_sub: String,
}

/// Response of [`connected`](SocialGraphClient::connected).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectedResponse {
    /// Whether the two subjects are graph-connected. `false` if either sub is
    /// unbridged (the server resolves both via the bridge first).
    pub connected: bool,
}

/// Request body for
/// [`connection_display_name`](SocialGraphClient::connection_display_name).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionDisplayNameRequest {
    /// The viewer's app-scoped chirp `sub`: the person the name is shown to.
    pub viewer_sub: String,
    /// The app-scoped chirp `sub` of the person to name.
    pub target_sub: String,
}

/// Response of
/// [`connection_display_name`](SocialGraphClient::connection_display_name).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionDisplayNameResponse {
    /// The target's display name; `None` (server: `null`) unless viewer and
    /// target are a current mutual connection. A stranger, an unbridged sub,
    /// the viewer themselves and a terminated account all get `None`.
    pub display_name: Option<String>,
}

/// Errors surfaced by [`SocialGraphClient`].
///
/// The client does **not** fail-closed on transport or status errors — it
/// returns them. Callers decide their own policy: Pigeon, for instance,
/// treats every error as "not connected" so a Social Graph outage can never
/// auto-approve a DM. Encoding that policy here would hide failures from
/// callers who need to see them, so it is intentionally left to the caller.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The HTTP request could not be sent or the body could not be read.
    #[error("social-graph request failed: {0}")]
    Transport(#[from] reqwest::Error),

    /// The server returned a non-success status.
    #[error("social-graph {path} returned HTTP {status}")]
    Status {
        /// The request path that produced the error.
        path: &'static str,
        /// The HTTP status code returned.
        status: u16,
    },
}

/// Typed async client for the Social Graph internal surface.
///
/// Holds a [`reqwest::Client`] (share/clone one across the process so the
/// connection pool is reused) and the service base URL. The shared
/// internal-service token is passed per call, not stored.
#[derive(Clone, Debug)]
pub struct SocialGraphClient {
    http: reqwest::Client,
    base_url: String,
}

impl SocialGraphClient {
    /// Build a client. A trailing slash on `base_url` is normalized away so
    /// that joining it with a leading-slash route never doubles the slash.
    pub fn new(http: reqwest::Client, base_url: impl Into<String>) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        Self { http, base_url }
    }

    fn url(&self, route: &str) -> String {
        format!("{}{}", self.base_url, route)
    }

    /// `POST /internal/graph/connected` — whether two app-scoped subs are
    /// graph-connected. The server resolves both subs via the bridge first, so
    /// an unbridged sub yields `false` (fail-visible), never an error.
    pub async fn connected(
        &self,
        internal_service_token: &str,
        viewer_sub: &str,
        target_sub: &str,
    ) -> Result<bool, Error> {
        let body = ConnectedRequest {
            viewer_sub: viewer_sub.to_owned(),
            target_sub: target_sub.to_owned(),
        };
        let response = self
            .http
            .post(self.url(ROUTE_CONNECTED))
            .bearer_auth(internal_service_token)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status {
                path: ROUTE_CONNECTED,
                status: status.as_u16(),
            });
        }
        Ok(response.json::<ConnectedResponse>().await?.connected)
    }

    /// `POST /internal/graph/connection-display-name` — the display name of
    /// `target_sub`, as the viewer sees it in their own connection list.
    /// `Ok(None)` unless the two subs are a current mutual connection, so it
    /// names no strangers and can't enumerate users.
    pub async fn connection_display_name(
        &self,
        internal_service_token: &str,
        viewer_sub: &str,
        target_sub: &str,
    ) -> Result<Option<String>, Error> {
        let body = ConnectionDisplayNameRequest {
            viewer_sub: viewer_sub.to_owned(),
            target_sub: target_sub.to_owned(),
        };
        let response = self
            .http
            .post(self.url(ROUTE_CONNECTION_DISPLAY_NAME))
            .bearer_auth(internal_service_token)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status {
                path: ROUTE_CONNECTION_DISPLAY_NAME,
                status: status.as_u16(),
            });
        }
        Ok(response
            .json::<ConnectionDisplayNameResponse>()
            .await?
            .display_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> SocialGraphClient {
        SocialGraphClient::new(reqwest::Client::new(), server.uri())
    }

    #[tokio::test]
    async fn url_joins_without_doubling_slash() {
        let c = SocialGraphClient::new(reqwest::Client::new(), "https://sg.example/");
        assert_eq!(
            c.url(ROUTE_CONNECTED),
            "https://sg.example/internal/graph/connected"
        );
    }

    #[tokio::test]
    async fn connected_parses_bool() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/internal/graph/connected"))
            .and(header("authorization", "Bearer secret-internal"))
            .and(body_json(serde_json::json!({
                "viewer_sub": "sub_a", "target_sub": "sub_b"
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "connected": true })),
            )
            .mount(&server)
            .await;
        assert!(client(&server)
            .connected("secret-internal", "sub_a", "sub_b")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn connection_display_name_posts_body_and_returns_name() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/internal/graph/connection-display-name"))
            .and(header("authorization", "Bearer secret-internal"))
            .and(body_json(serde_json::json!({
                "viewer_sub": "sub_a", "target_sub": "sub_b"
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "display_name": "Bob Builder" })),
            )
            .mount(&server)
            .await;
        let got = client(&server)
            .connection_display_name("secret-internal", "sub_a", "sub_b")
            .await
            .unwrap();
        assert_eq!(got, Some("Bob Builder".to_owned()));
    }

    #[tokio::test]
    async fn connection_display_name_of_a_non_connection_is_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/internal/graph/connection-display-name"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "display_name": null })),
            )
            .mount(&server)
            .await;
        let got = client(&server)
            .connection_display_name("secret-internal", "sub_a", "sub_stranger")
            .await
            .unwrap();
        assert_eq!(got, None);
    }

    #[tokio::test]
    async fn non_success_status_is_surfaced() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/internal/graph/connected"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let err = client(&server)
            .connected("secret-internal", "sub_a", "sub_b")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Status { status: 503, .. }));
    }
}
