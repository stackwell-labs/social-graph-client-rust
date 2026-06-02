//! Shared wire types and a typed async client for the [Social Graph] service.
//!
//! This crate is the **single source of truth** for the Social Graph wire API
//! that peer services consume. Before it existed, the request/response shapes
//! and route paths were re-declared by hand in each consumer (Pigeon, Gotta,
//! …); those copies drift. Anything in this crate is derived from the real
//! server handlers in `social-graph/backend/src/api/social_graph.rs`.
//!
//! Two surfaces are modeled today:
//!
//! - **`GET /api/me`** — the *user-token* surface. The caller presents their
//!   own ChirpAuth bearer (a token Social Graph itself issued, or one from an
//!   audience that adopts Social Graph's identity). Resolves the caller to
//!   their canonical Social Graph `userId`. The server projects a richer
//!   self-view (email, display name, …); consumers that only need the id
//!   deserialize through [`MeResponse`], which ignores the extra fields.
//!
//! - **`GET /internal/graph/is_connected?a=&b=`** — the *internal-service*
//!   surface. The caller presents a shared bearer token
//!   (`SOCIAL_GRAPH_INTERNAL_SERVICE_TOKEN`) out-of-band, not a per-user
//!   token, and asks whether two Social Graph users are graph-connected. The
//!   endpoint returns `503` when the deployment has no internal token
//!   configured and `401` on token mismatch.
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
//!
//! // Resolve the caller (presenting their own user bearer) to their userId.
//! let my_id = client.resolve_me("user-bearer-token").await?;
//!
//! // Ask whether two users are connected, presenting the internal token.
//! let connected = client
//!     .is_connected("internal-service-token", &my_id, "sg_other")
//!     .await?;
//! # let _ = (my_id, connected);
//! # Ok(())
//! # }
//! ```
//!
//! [Social Graph]: https://github.com/coreygirard

use serde::{Deserialize, Serialize};

/// Route path for the user-token self-resolution endpoint.
///
/// Mirrors the `GET /api/me` route registered by the server.
pub const ROUTE_ME: &str = "/api/me";

/// Route path for the internal connectedness lookup.
///
/// Mirrors the `GET /internal/graph/is_connected` route registered by the
/// server. Takes `a` and `b` query parameters (Social Graph `userId`s) and an
/// `Authorization: Bearer <internal-service-token>` header.
pub const ROUTE_IS_CONNECTED: &str = "/internal/graph/is_connected";

/// Response shape of `GET /api/me`, narrowed to the caller's `userId`.
///
/// The server returns a richer object (the self-view also carries `email`,
/// `displayName`, `avatarUrl`); any field not declared here is ignored on
/// deserialize, so this stays valid as the server's self-view grows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeResponse {
    /// The caller's canonical Social Graph `userId`.
    pub id: String,
}

/// Response shape of `GET /internal/graph/is_connected`.
///
/// Field names match the server's `serde::Serialize` output exactly
/// (no rename): `connected`, `user_a`, `user_b`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsConnectedResponse {
    /// Whether `user_a` and `user_b` are graph-connected.
    pub connected: bool,
    /// The `a` query parameter echoed back (trimmed by the server).
    pub user_a: String,
    /// The `b` query parameter echoed back (trimmed by the server).
    pub user_b: String,
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

/// Typed async client for the Social Graph service.
///
/// Holds a [`reqwest::Client`] (share/clone one across the process so the
/// connection pool is reused) and the service base URL. Bearer tokens are
/// passed per call, not stored, because the two surfaces use different
/// credentials: a per-user token for [`resolve_me`](Self::resolve_me) and the
/// shared internal-service token for
/// [`is_connected`](Self::is_connected).
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

    /// `GET /api/me` — resolve the caller's Social Graph `userId` from their
    /// own bearer token.
    ///
    /// Returns [`Error::Status`] on any non-success response (e.g. `401` for a
    /// token Social Graph won't accept).
    pub async fn resolve_me(&self, user_token: &str) -> Result<String, Error> {
        let response = self
            .http
            .get(self.url(ROUTE_ME))
            .bearer_auth(user_token)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status {
                path: ROUTE_ME,
                status: status.as_u16(),
            });
        }
        Ok(response.json::<MeResponse>().await?.id)
    }

    /// `GET /internal/graph/is_connected` — ask whether Social Graph users
    /// `a` and `b` are graph-connected, presenting the shared internal-service
    /// token.
    ///
    /// Returns [`Error::Status`] on any non-success response: `503` when the
    /// deployment has no internal token configured, `401` on token mismatch,
    /// `400` when `a`/`b` are empty or equal.
    pub async fn is_connected(
        &self,
        internal_service_token: &str,
        a: &str,
        b: &str,
    ) -> Result<bool, Error> {
        let response = self
            .http
            .get(self.url(ROUTE_IS_CONNECTED))
            .bearer_auth(internal_service_token)
            .query(&[("a", a), ("b", b)])
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status {
                path: ROUTE_IS_CONNECTED,
                status: status.as_u16(),
            });
        }
        Ok(response.json::<IsConnectedResponse>().await?.connected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> SocialGraphClient {
        SocialGraphClient::new(reqwest::Client::new(), server.uri())
    }

    #[test]
    fn route_constants_match_server() {
        assert_eq!(ROUTE_ME, "/api/me");
        assert_eq!(ROUTE_IS_CONNECTED, "/internal/graph/is_connected");
    }

    #[test]
    fn me_response_ignores_extra_server_fields() {
        // The server's self-view carries email/displayName/avatarUrl too;
        // MeResponse must tolerate them.
        let body = serde_json::json!({
            "id": "sg_alice",
            "email": "a@x.test",
            "displayName": "Alice",
            "avatarUrl": null
        });
        let parsed: MeResponse = serde_json::from_value(body).unwrap();
        assert_eq!(parsed.id, "sg_alice");
    }

    #[test]
    fn is_connected_response_matches_server_field_names() {
        // Server uses plain serde::Serialize: connected / user_a / user_b.
        let body = serde_json::json!({
            "connected": true,
            "user_a": "sg_alice",
            "user_b": "sg_bob"
        });
        let parsed: IsConnectedResponse = serde_json::from_value(body).unwrap();
        assert_eq!(
            parsed,
            IsConnectedResponse {
                connected: true,
                user_a: "sg_alice".to_owned(),
                user_b: "sg_bob".to_owned(),
            }
        );
    }

    #[test]
    fn base_url_trailing_slash_is_normalized() {
        let c = SocialGraphClient::new(reqwest::Client::new(), "https://sg.example/");
        assert_eq!(c.url(ROUTE_ME), "https://sg.example/api/me");
    }

    #[tokio::test]
    async fn resolve_me_sends_bearer_and_returns_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/me"))
            .and(header("authorization", "Bearer usertok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "sg_alice",
                "email": "a@x.test",
                "displayName": "Alice"
            })))
            .mount(&server)
            .await;
        let id = client(&server).resolve_me("usertok").await.unwrap();
        assert_eq!(id, "sg_alice");
    }

    #[tokio::test]
    async fn resolve_me_surfaces_non_success_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/me"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        match client(&server).resolve_me("bad").await {
            Err(Error::Status { path, status }) => {
                assert_eq!(path, ROUTE_ME);
                assert_eq!(status, 401);
            }
            other => panic!("expected Status error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn is_connected_uses_internal_token_and_parses_true() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/internal/graph/is_connected"))
            .and(header("authorization", "Bearer secret-internal"))
            .and(query_param("a", "sg_alice"))
            .and(query_param("b", "sg_bob"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "connected": true,
                "user_a": "sg_alice",
                "user_b": "sg_bob"
            })))
            .mount(&server)
            .await;
        assert!(client(&server)
            .is_connected("secret-internal", "sg_alice", "sg_bob")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn is_connected_false_is_respected() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/internal/graph/is_connected"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "connected": false,
                "user_a": "sg_alice",
                "user_b": "sg_carol"
            })))
            .mount(&server)
            .await;
        assert!(!client(&server)
            .is_connected("secret-internal", "sg_alice", "sg_carol")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn is_connected_surfaces_503_when_unconfigured() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/internal/graph/is_connected"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        match client(&server)
            .is_connected("secret-internal", "sg_alice", "sg_bob")
            .await
        {
            Err(Error::Status { path, status }) => {
                assert_eq!(path, ROUTE_IS_CONNECTED);
                assert_eq!(status, 503);
            }
            other => panic!("expected Status error, got {other:?}"),
        }
    }
}
