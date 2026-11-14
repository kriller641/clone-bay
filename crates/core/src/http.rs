//! How the app talks to web services: the system's own TLS and trusted
//! certificates, and a User-Agent naming the app.

use std::time::Duration;

/// The app's name and version, and a way to reach its makers: the link to
/// the source code.
pub(crate) fn user_agent(app_version: &str) -> String {
    format!(
        "clone-bay/{app_version} (+{})",
        env!("CARGO_PKG_REPOSITORY")
    )
}

/// `status_as_error` makes any answer but success an error; without it the
/// caller reads the status itself.
pub(crate) fn agent(app_version: &str, timeout: Duration, status_as_error: bool) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        // The operating system's own trusted certificates.
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .user_agent(user_agent(app_version))
        .timeout_global(Some(timeout))
        .http_status_as_error(status_as_error)
        .tls_config(tls)
        .build()
        .new_agent()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_agent_names_the_app_version_and_source() {
        assert_eq!(
            user_agent("1.2.3"),
            "clone-bay/1.2.3 (+https://github.com/kriller641/clone-bay)"
        );
    }
}
