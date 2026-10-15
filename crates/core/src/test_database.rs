//! Shared gate for adapter tests that need a disposable PostgreSQL database.
//!
//! Without a database URL such tests used to `return` early, which the test
//! harness reports as passed. In CI that turned a lost database configuration
//! into silent PostgreSQL coverage loss, so CI now fails instead of skipping.

pub const TEST_DATABASE_URL_ENV: &str = "MURIARC_TEST_DATABASE_URL";
pub const REQUIRE_POSTGRES_TESTS_ENV: &str = "MURIARC_REQUIRE_POSTGRES_TESTS";
const DISPOSABLE_DATABASE_MARKER: &str = "muriarc_test";

/// Returns the disposable PostgreSQL URL, or `None` after printing an explicit
/// SKIP when it is not configured locally.
///
/// # Panics
///
/// Panics when the URL is missing while `CI` or
/// `MURIARC_REQUIRE_POSTGRES_TESTS` is enabled, or when the URL does not name a
/// disposable `muriarc_test` database.
pub fn postgres_test_url() -> Option<String> {
    let required = env_flag("CI") || env_flag(REQUIRE_POSTGRES_TESTS_ENV);
    match resolve(std::env::var(TEST_DATABASE_URL_ENV).ok(), required) {
        Ok(url) => url,
        Err(message) => panic!("{message}"),
    }
}

fn resolve(url: Option<String>, required: bool) -> Result<Option<String>, String> {
    match url.filter(|url| !url.trim().is_empty()) {
        Some(url) if url.contains(DISPOSABLE_DATABASE_MARKER) => Ok(Some(url)),
        Some(_) => Err(format!(
            "{TEST_DATABASE_URL_ENV} must point to a disposable {DISPOSABLE_DATABASE_MARKER} database"
        )),
        None if required => Err(format!(
            "{TEST_DATABASE_URL_ENV} is required when CI or {REQUIRE_POSTGRES_TESTS_ENV} is set; \
             PostgreSQL tests must not pass without running"
        )),
        None => {
            eprintln!("SKIP PostgreSQL test: {TEST_DATABASE_URL_ENV} is not set");
            Ok(None)
        }
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| {
        let value = value.trim();
        !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
    })
}

#[cfg(test)]
mod tests {
    use super::resolve;

    const URL: &str = "postgresql://user:pass@127.0.0.1:5432/muriarc_test";

    #[test]
    fn configured_disposable_url_runs_whether_or_not_required() {
        for required in [false, true] {
            assert_eq!(
                resolve(Some(URL.to_owned()), required),
                Ok(Some(URL.to_owned()))
            );
        }
    }

    #[test]
    fn missing_url_skips_locally_but_fails_when_required() {
        assert_eq!(resolve(None, false), Ok(None));
        assert_eq!(resolve(Some("  ".to_owned()), false), Ok(None));
        assert!(resolve(None, true).is_err());
        assert!(resolve(Some(String::new()), true).is_err());
    }

    #[test]
    fn non_disposable_database_is_rejected() {
        let production = "postgresql://user:pass@db:5432/muriarc".to_owned();
        assert!(resolve(Some(production.clone()), false).is_err());
        assert!(resolve(Some(production), true).is_err());
    }
}
