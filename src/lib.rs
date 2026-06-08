//! Shared wire types and a typed async client for the [Social Graph] service.
//!
//! This crate is the **single source of truth** for the Social Graph wire API
//! that peer services consume. Before it existed, the request/response shapes
//! and route paths were re-declared by hand in each consumer (Pigeon, Gotta,
//! …); those copies drift. Anything in this crate is derived from the real
//! server handlers in `social-graph/backend/src/api/social_graph.rs`.
//!
//! # The pairwise / pseudonym surface (since 0.2.0)
//!
//! All three modeled surfaces are **internal-service** endpoints: the caller
//! presents the shared bearer token (`SOCIAL_GRAPH_INTERNAL_SERVICE_TOKEN`)
//! out-of-band, passes an **app-scoped chirp `sub`** (pairwise per app — a
//! different sub per app for the same human) as input, and gets back the app's
//! own opaque **per-(app, user) pseudonym** or a connectedness bool — **never a
//! global `users.id`**, so two apps can't correlate the same human.
//!
//! - **`POST /internal/graph/resolve-self`** → the app's own pseudonym for its
//!   own user ([`resolve_self`](SocialGraphClient::resolve_self)). `None` when
//!   the sub isn't bridged yet (fail-visible).
//! - **`POST /internal/graph/resolve-audience`** → the user's connections, as
//!   *this app's* pseudonyms ([`resolve_audience`](SocialGraphClient::resolve_audience)).
//! - **`POST /internal/graph/connected`** → whether two subs are graph-connected
//!   ([`connected`](SocialGraphClient::connected)); resolves both via the
//!   `(chirp_sub -> users.id)` bridge.
//!
//! The legacy `GET /api/me` (`resolve_me`) and `GET /internal/graph/is_connected`
//! (`is_connected`) surfaces — which handed apps the global `users.id` and so
//! let two apps correlate a shared user — were **retired in 0.2.0**.
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
//! let token = "internal-service-token";
//!
//! // Resolve the caller's app-scoped sub to this app's pseudonym for them.
//! let me = client.resolve_self(token, "sub_app_alice", "my-app").await?;
//!
//! // Their connections, as this app's pseudonyms (the valid send targets).
//! let audience = client.resolve_audience(token, "sub_app_alice", "my-app").await?;
//! # let _ = (me, audience);
//! # Ok(())
//! # }
//! ```
//!
//! [Social Graph]: https://github.com/stackwell-labs

use serde::{Deserialize, Serialize};

/// `POST` route resolving an app-scoped sub to the app's own pseudonym.
pub const ROUTE_RESOLVE_SELF: &str = "/internal/graph/resolve-self";

/// `POST` route resolving a viewer's connections to the app's pseudonyms.
pub const ROUTE_RESOLVE_AUDIENCE: &str = "/internal/graph/resolve-audience";

/// `POST` route asking whether two app-scoped subs are graph-connected.
pub const ROUTE_CONNECTED: &str = "/internal/graph/connected";

/// Request body for [`resolve_self`](SocialGraphClient::resolve_self).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveSelfRequest {
    /// The caller's app-scoped chirp `sub` (pairwise per app).
    pub subject_sub: String,
    /// The app's client id — the namespace its pseudonyms are minted under.
    pub client_id: String,
}

/// Response of [`resolve_self`](SocialGraphClient::resolve_self).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveSelfResponse {
    /// The app's own pseudonym for its own user; `None` (server: `null`) when
    /// the sub is not yet bridged into Social Graph (fail-visible).
    pub app_user_id: Option<String>,
}

/// Request body for [`resolve_audience`](SocialGraphClient::resolve_audience).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveAudienceRequest {
    /// The viewer's app-scoped chirp `sub`.
    pub viewer_sub: String,
    /// The app's client id (pseudonym namespace).
    pub client_id: String,
}

/// Response of [`resolve_audience`](SocialGraphClient::resolve_audience).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveAudienceResponse {
    /// `false` when the viewer sub isn't bridged — distinguishes "unknown
    /// viewer" from "viewer with no connections".
    pub resolved: bool,
    /// The viewer's connections, as *this app's* pseudonyms (never raw ids).
    pub audience: Vec<String>,
}

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

/// Typed async client for the Social Graph internal resolution surface.
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

    /// `POST /internal/graph/resolve-self` — resolve the caller's app-scoped
    /// `sub` to this app's own pseudonym for them. `Ok(None)` when the sub is
    /// not yet bridged into Social Graph.
    pub async fn resolve_self(
        &self,
        internal_service_token: &str,
        subject_sub: &str,
        client_id: &str,
    ) -> Result<Option<String>, Error> {
        let body = ResolveSelfRequest {
            subject_sub: subject_sub.to_owned(),
            client_id: client_id.to_owned(),
        };
        let response = self
            .http
            .post(self.url(ROUTE_RESOLVE_SELF))
            .bearer_auth(internal_service_token)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status {
                path: ROUTE_RESOLVE_SELF,
                status: status.as_u16(),
            });
        }
        Ok(response.json::<ResolveSelfResponse>().await?.app_user_id)
    }

    /// `POST /internal/graph/resolve-audience` — the viewer's connections, as
    /// this app's pseudonyms. These are the valid targets the viewer may act
    /// on; membership in this set IS the connection check.
    pub async fn resolve_audience(
        &self,
        internal_service_token: &str,
        viewer_sub: &str,
        client_id: &str,
    ) -> Result<ResolveAudienceResponse, Error> {
        let body = ResolveAudienceRequest {
            viewer_sub: viewer_sub.to_owned(),
            client_id: client_id.to_owned(),
        };
        let response = self
            .http
            .post(self.url(ROUTE_RESOLVE_AUDIENCE))
            .bearer_auth(internal_service_token)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status {
                path: ROUTE_RESOLVE_AUDIENCE,
                status: status.as_u16(),
            });
        }
        Ok(response.json::<ResolveAudienceResponse>().await?)
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
            c.url(ROUTE_RESOLVE_SELF),
            "https://sg.example/internal/graph/resolve-self"
        );
    }

    #[tokio::test]
    async fn resolve_self_posts_body_and_returns_pseudonym() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/internal/graph/resolve-self"))
            .and(header("authorization", "Bearer secret-internal"))
            .and(body_json(serde_json::json!({
                "subject_sub": "sub_app_alice", "client_id": "my-app"
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "app_user_id": "psd_alice" })),
            )
            .mount(&server)
            .await;
        let got = client(&server)
            .resolve_self("secret-internal", "sub_app_alice", "my-app")
            .await
            .unwrap();
        assert_eq!(got, Some("psd_alice".to_owned()));
    }

    #[tokio::test]
    async fn resolve_self_unbridged_is_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/internal/graph/resolve-self"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "app_user_id": null })),
            )
            .mount(&server)
            .await;
        let got = client(&server)
            .resolve_self("secret-internal", "sub_unbridged", "my-app")
            .await
            .unwrap();
        assert_eq!(got, None);
    }

    #[tokio::test]
    async fn resolve_audience_returns_pseudonyms() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/internal/graph/resolve-audience"))
            .and(header("authorization", "Bearer secret-internal"))
            .and(body_json(serde_json::json!({
                "viewer_sub": "sub_app_alice", "client_id": "my-app"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "resolved": true, "audience": ["psd_bob", "psd_carol"]
            })))
            .mount(&server)
            .await;
        let got = client(&server)
            .resolve_audience("secret-internal", "sub_app_alice", "my-app")
            .await
            .unwrap();
        assert!(got.resolved);
        assert_eq!(got.audience, vec!["psd_bob".to_owned(), "psd_carol".to_owned()]);
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
