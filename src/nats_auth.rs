//! Credentialed NATS connections.
//!
//! Both NATS call sites used `async_nats::connect(url)`, which sends **no
//! credentials**. Against a broker with an `authorization` block that lands the
//! node on the anonymous user, and anonymous is not permitted to publish
//! `scada.*`. The failure was silent in the worst way: the connection succeeded,
//! telemetry reported "NATS connected", and every
//! `scada.consensus.<peer>.status` publish was refused server-side with a
//! Permissions Violation that the node never saw. Measured on the live swarm
//! 2026-08-23, that was a steady stream of refusals and zero telemetry
//! delivered, for months.
//!
//! Embedding `user:pass@` in the URL did not help either — that userinfo is not
//! applied by the client, so it has to be pulled out and passed explicitly.
//!
//! Credentials are resolved from, in order:
//!   1. userinfo in the URL — `nats://user:pass@host:4222`
//!   2. the `NATS_USER` / `NATS_PASSWORD` environment variables
//!
//! The env vars are the convention the rest of this fleet already uses, so a
//! systemd unit can point `EnvironmentFile=` at the same secrets file every
//! other service reads — which keeps the password out of a world-readable
//! config file.

/// Resolved credentials, plus the URL to dial with (userinfo stripped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NatsAuth {
    /// The URL to connect to, with any `user:pass@` removed.
    pub url: String,
    pub user: Option<String>,
    pub password: Option<String>,
}

impl NatsAuth {
    /// True when both a user and a password are present.
    pub fn is_credentialed(&self) -> bool {
        self.user.is_some() && self.password.is_some()
    }
}

/// Treat a set-but-empty environment variable as absent — an exported
/// `NATS_USER=` is how a half-written env file presents, and connecting as the
/// empty user is never what was meant.
fn non_empty(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}

/// Resolve credentials for a NATS URL. Pure: the environment is passed in, so
/// this is unit-testable without touching the process environment.
pub fn resolve(nats_url: &str, env_user: Option<String>, env_password: Option<String>) -> NatsAuth {
    let (scheme, rest) = match nats_url.split_once("://") {
        Some((s, r)) => (Some(s), r),
        None => (None, nats_url),
    };

    // Split on the LAST '@': a password may legitimately contain '@', a host
    // may not, so the final '@' is the true userinfo/host boundary.
    if let Some(idx) = rest.rfind('@') {
        let (userinfo, host_with_at) = rest.split_at(idx);
        let host = &host_with_at[1..];
        let (user, password) = match userinfo.split_once(':') {
            Some((u, p)) => (u.to_string(), Some(p.to_string())),
            None => (userinfo.to_string(), None),
        };
        if !user.is_empty() {
            return NatsAuth {
                url: match scheme {
                    Some(s) => format!("{s}://{host}"),
                    None => host.to_string(),
                },
                user: Some(user),
                password,
            };
        }
    }

    NatsAuth {
        url: nats_url.to_string(),
        user: non_empty(env_user),
        password: non_empty(env_password),
    }
}

/// Connect to NATS, applying credentials when any are available.
///
/// Falls back to an anonymous connect when none are configured, so a local
/// broker with no `authorization` block keeps working untouched.
pub async fn connect(nats_url: &str) -> Result<async_nats::Client, async_nats::ConnectError> {
    let auth = resolve(
        nats_url,
        std::env::var("NATS_USER").ok(),
        std::env::var("NATS_PASSWORD").ok(),
    );
    match (&auth.user, &auth.password) {
        (Some(user), Some(password)) => {
            async_nats::ConnectOptions::with_user_and_password(user.clone(), password.clone())
                .connect(&auth.url)
                .await
        }
        _ => async_nats::connect(&auth.url).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_userinfo_is_extracted_and_stripped() {
        // The URL must be dialled WITHOUT the userinfo — the credentials travel
        // in the CONNECT frame instead, which is the part that was missing.
        let a = resolve("nats://svc:s3cret@swarm.example.com:4222", None, None);
        assert_eq!(a.url, "nats://swarm.example.com:4222");
        assert_eq!(a.user.as_deref(), Some("svc"));
        assert_eq!(a.password.as_deref(), Some("s3cret"));
        assert!(a.is_credentialed());
    }

    #[test]
    fn env_vars_are_used_when_the_url_carries_no_userinfo() {
        let a = resolve(
            "nats://swarm.example.com:4222",
            Some("svc".into()),
            Some("s3cret".into()),
        );
        assert_eq!(a.url, "nats://swarm.example.com:4222");
        assert_eq!(a.user.as_deref(), Some("svc"));
        assert_eq!(a.password.as_deref(), Some("s3cret"));
    }

    #[test]
    fn url_userinfo_wins_over_the_environment() {
        let a = resolve(
            "nats://fromurl:pw1@host:4222",
            Some("fromenv".into()),
            Some("pw2".into()),
        );
        assert_eq!(a.user.as_deref(), Some("fromurl"));
        assert_eq!(a.password.as_deref(), Some("pw1"));
    }

    #[test]
    fn no_credentials_anywhere_leaves_the_url_untouched() {
        // A local broker with no authorization block must keep working.
        let a = resolve("nats://127.0.0.1:4222", None, None);
        assert_eq!(a.url, "nats://127.0.0.1:4222");
        assert!(a.user.is_none());
        assert!(!a.is_credentialed());
    }

    #[test]
    fn empty_env_vars_count_as_absent() {
        // `NATS_USER=` in a half-written env file must not connect as "".
        let a = resolve("nats://host:4222", Some("".into()), Some("   ".into()));
        assert!(a.user.is_none());
        assert!(a.password.is_none());
    }

    #[test]
    fn a_password_containing_an_at_sign_still_splits_on_the_host_boundary() {
        let a = resolve("nats://svc:p@ss@host:4222", None, None);
        assert_eq!(a.url, "nats://host:4222");
        assert_eq!(a.user.as_deref(), Some("svc"));
        assert_eq!(a.password.as_deref(), Some("p@ss"));
    }

    #[test]
    fn a_schemeless_url_is_handled() {
        let a = resolve("svc:pw@host:4222", None, None);
        assert_eq!(a.url, "host:4222");
        assert_eq!(a.user.as_deref(), Some("svc"));
    }
}
