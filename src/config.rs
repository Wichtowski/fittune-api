use std::{env, net::SocketAddr, time::Duration};

use anyhow::{Context, Result, bail};

/// Runtime configuration, read from `FITTUNE_*` environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub db_max_connections: u32,
    pub bind_addr: SocketAddr,
    pub cors_origins: Vec<String>,
    pub session_ttl: Duration,
    pub app_version: String,
    pub log_format: LogFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Pretty,
    Json,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let database_url =
            env::var("FITTUNE_DATABASE_URL").context("FITTUNE_DATABASE_URL must be set")?;

        let bind_addr = var_or("FITTUNE_BIND_ADDR", "0.0.0.0:4733")
            .parse()
            .context("FITTUNE_BIND_ADDR must be a socket address such as 0.0.0.0:4733")?;

        let db_max_connections = var_or("FITTUNE_DB_MAX_CONNECTIONS", "10")
            .parse()
            .context("FITTUNE_DB_MAX_CONNECTIONS must be a positive integer")?;

        let session_ttl_hours: u64 = var_or("FITTUNE_SESSION_TTL_HOURS", "720")
            .parse()
            .context("FITTUNE_SESSION_TTL_HOURS must be a positive integer")?;
        if session_ttl_hours == 0 {
            bail!("FITTUNE_SESSION_TTL_HOURS must be greater than zero");
        }

        let log_format = match var_or("FITTUNE_LOG_FORMAT", "pretty").as_str() {
            "pretty" => LogFormat::Pretty,
            "json" => LogFormat::Json,
            other => bail!("FITTUNE_LOG_FORMAT must be `pretty` or `json`, got `{other}`"),
        };

        Ok(Self {
            database_url,
            db_max_connections,
            bind_addr,
            cors_origins: parse_origins(&var_or("FITTUNE_CORS_ORIGINS", "http://localhost:5173")),
            session_ttl: Duration::from_secs(session_ttl_hours * 60 * 60),
            app_version: var_or("FITTUNE_APP_VERSION", "dev"),
            log_format,
        })
    }
}

fn var_or(key: &str, default: &str) -> String {
    env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn parse_origins(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|origin| origin.trim().trim_end_matches('/'))
        .filter(|origin| !origin.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_separated_origins() {
        assert_eq!(
            parse_origins(" https://a.example/ , ,http://localhost:5173"),
            vec!["https://a.example", "http://localhost:5173"]
        );
    }
}
