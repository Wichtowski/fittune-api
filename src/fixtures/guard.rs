//! Keeps `seed-dev` away from everything but a dedicated, disposable local database.

use std::str::FromStr;

use anyhow::{Context, Result, bail};
use sqlx::postgres::PgConnectOptions;

/// Must be `development`; nothing else reads it, so production never sets it by accident
pub const ENV_VAR: &str = "FITTUNE_ENV";
/// The dedicated fixture database. `FITTUNE_DATABASE_URL` is never used for fixtures
pub const DATABASE_URL_VAR: &str = "FITTUNE_FIXTURES_DATABASE_URL";

const LOCAL_HOSTS: [&str; 4] = ["localhost", "127.0.0.1", "::1", "[::1]"];
const NAME_SUFFIX: &str = "_dev";

/// A fixture database that passed every check.
#[derive(Debug, Clone)]
pub struct Target {
    pub database: String,
    pub options: PgConnectOptions,
    /// The same server's `postgres` database, for creating and dropping the fixture database
    pub maintenance: PgConnectOptions,
}

/// `test_url` is `DATABASE_URL`, the server `sqlx::test` creates its throwaway databases from.
pub fn check(
    env: Option<&str>,
    fixtures_url: Option<&str>,
    test_url: Option<&str>,
) -> Result<Target> {
    if env.map(str::trim) != Some("development") {
        bail!("seed-dev only runs with {ENV_VAR}=development");
    }
    let url = fixtures_url
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .with_context(|| {
            format!(
                "set {DATABASE_URL_VAR} to a dedicated local database such as postgres://fittune:fittune@localhost:5432/fittune_dev"
            )
        })?;
    let options = PgConnectOptions::from_str(url)
        .with_context(|| format!("{DATABASE_URL_VAR} is not a valid Postgres URL"))?;

    let host = options.get_host();
    // A socket path is a local server by definition
    if !host.starts_with('/') && !LOCAL_HOSTS.contains(&host) {
        bail!("{DATABASE_URL_VAR} must point at a local server, not `{host}`");
    }

    let database = options
        .get_database()
        .with_context(|| format!("{DATABASE_URL_VAR} must name a database"))?
        .to_owned();
    if !is_fixture_name(&database) {
        bail!(
            "the fixture database must be a dedicated one named like `fittune{NAME_SUFFIX}` (lowercase letters, digits and `_`, ending in `{NAME_SUFFIX}`), got `{database}`"
        );
    }
    let test_database = test_url
        .and_then(|url| PgConnectOptions::from_str(url).ok())
        .and_then(|test| test.get_database().map(str::to_owned));
    if test_database.as_deref() == Some(database.as_str()) {
        bail!(
            "the fixture database must differ from the integration-test database in DATABASE_URL"
        );
    }

    Ok(Target {
        maintenance: options.clone().database("postgres"),
        options,
        database,
    })
}

fn is_fixture_name(name: &str) -> bool {
    name.len() > NAME_SUFFIX.len()
        && name.ends_with(NAME_SUFFIX)
        && !name.starts_with("_sqlx_test")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "postgres://fittune:fittune@localhost:5432/fittune_dev";
    const TEST_URL: &str = "postgres://fittune:fittune@localhost:5432/fittune";

    fn error(env: Option<&str>, url: &str) -> String {
        match check(env, Some(url), Some(TEST_URL)) {
            Ok(target) => panic!("expected {url} to be rejected, got {target:?}"),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn accepts_a_local_dev_database() -> Result<()> {
        let target = check(Some("development"), Some(URL), Some(TEST_URL))?;
        assert_eq!(target.database, "fittune_dev");
        assert_eq!(target.maintenance.get_database(), Some("postgres"));
        check(
            Some("development"),
            Some("postgres://u:p@127.0.0.1/app_dev"),
            None,
        )?;
        Ok(())
    }

    #[test]
    fn requires_development_mode() {
        assert!(error(None, URL).contains("FITTUNE_ENV=development"));
        assert!(error(Some("production"), URL).contains("FITTUNE_ENV=development"));
        assert!(check(Some("development"), None, None).is_err());
    }

    #[test]
    fn rejects_remote_and_shared_databases() {
        let dev = Some("development");
        for (url, reason) in [
            (
                "postgres://u:p@db.example.com:5432/fittune_dev",
                "local server",
            ),
            (
                "postgres://u:p@fittune-postgres:5432/fittune_dev",
                "local server",
            ),
            ("postgres://u:p@localhost:5432/fittune", "dedicated"),
            ("postgres://u:p@localhost:5432/Fittune_dev", "dedicated"),
            ("postgres://u:p@localhost:5432/_dev", "dedicated"),
            (
                "postgres://u:p@localhost:5432/_sqlx_test_1_dev",
                "dedicated",
            ),
        ] {
            assert!(error(dev, url).contains(reason), "{url}");
        }
        assert!(check(dev, Some(URL), Some(URL)).is_err());
    }
}
