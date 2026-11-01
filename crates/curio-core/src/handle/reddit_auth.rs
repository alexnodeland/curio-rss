//! Why reddit refused a full-post request: the [`RedditAuthReason`] a
//! [`CoreError::RedditAuth`](super::CoreError::RedditAuth) carries, and
//! the user-facing guidance built from it.

/// Why reddit refused a full-post request
/// ([`CoreError::RedditAuth`](super::CoreError::RedditAuth)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RedditAuthReason {
    /// No API credentials are installed, and reddit blocks anonymous
    /// full-post requests. Fixed by adding the user's own (free) Reddit
    /// API credentials.
    CredentialsMissing,
    /// The installed credentials were refused at the token endpoint — a
    /// wrong client id or secret, or an app that was deleted. Fixed by
    /// correcting the credentials or re-creating the app.
    CredentialsRejected,
    /// The credentials were accepted, but reddit refused the access token
    /// they were traded for — access revoked, or the app lost its
    /// permissions. The token is dropped, so the next request re-grants;
    /// if reddit keeps refusing, the app needs re-authorizing or
    /// re-creating.
    TokenRevoked,
    /// Credentials are installed, but no access token could be had for a
    /// reason other than a refusal — the token endpoint failed (a 5xx
    /// during an outage), was unreachable or timed out, or answered with
    /// an unreadable payload — and reddit blocks the anonymous fallback.
    /// Nothing is wrong with the credentials: trying again later is the
    /// fix.
    TokenUnavailable,
}

/// The user-facing guidance for a
/// [`CoreError::RedditAuth`](super::CoreError::RedditAuth).
pub(super) fn reddit_auth_message(reason: RedditAuthReason, status: u16) -> String {
    match reason {
        RedditAuthReason::CredentialsMissing => format!(
            "reddit blocks anonymous full-post requests (HTTP {status}) — \
             add your own free Reddit API credentials to load full posts"
        ),
        RedditAuthReason::CredentialsRejected => format!(
            "reddit refused the stored API credentials (HTTP {status}) — \
             check the client id and secret, or re-create the app"
        ),
        RedditAuthReason::TokenRevoked => format!(
            "reddit refused the access token for the stored API credentials (HTTP {status}) — \
             the app's access may have been revoked; try again, and if it keeps failing, \
             re-create the app"
        ),
        RedditAuthReason::TokenUnavailable => format!(
            "reddit could not issue an access token for the stored API credentials, and it \
             blocks anonymous full-post requests (HTTP {status}) — reddit may be having an \
             outage, or is unreachable; try again later"
        ),
    }
}
