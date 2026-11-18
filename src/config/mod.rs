//! Environment-based configuration. Parsed once at startup; never logged in full.

use std::{fmt, net::IpAddr, time::Duration};

use anyhow::{Context, Result, bail};

/// A value that must never end up in logs or debug output.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEnv {
    Development,
    Production,
}

impl AppEnv {
    pub fn is_production(self) -> bool {
        self == AppEnv::Production
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub env: AppEnv,
    pub host: IpAddr,
    pub port: u16,
    pub database_url: Secret,
    pub db_max_connections: u32,
    pub session_secret: Secret,
    pub cookie_secure: bool,
    pub session_ttl: Duration,
    pub bootstrap_admin_email: Option<String>,
    pub bootstrap_admin_password: Option<Secret>,
    /// Exact origins allowed for CORS and accepted by the CSRF origin check.
    pub frontend_origins: Vec<String>,
    pub trust_proxy: bool,
    pub enable_api_docs: bool,
    /// Outgoing email. Without it there is no second sign-in step and no
    /// password reset by email.
    pub smtp: Option<SmtpConfig>,
}

/// How the SMTP connection is secured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls {
    /// Plain connection upgraded with STARTTLS (usually port 587). Required, not opportunistic.
    StartTls,
    /// TLS from the first byte (usually port 465).
    Tls,
    /// No encryption — only for a local relay or a development mail catcher.
    None,
}

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub tls: SmtpTls,
    pub username: Option<String>,
    pub password: Option<Secret>,
    /// Sender, e.g. `CommitVault <vault@example.com>`.
    pub from: String,
}

const MIN_SESSION_SECRET_LEN: usize = 64;

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Builds the configuration from an arbitrary key lookup (testable without
    /// touching the process environment).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let get = |key: &str| lookup(key).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let required = |key: &str| get(key).with_context(|| format!("{key} must be set"));

        let env = match get("APP_ENV").as_deref() {
            None | Some("development") | Some("dev") => AppEnv::Development,
            Some("production") | Some("prod") => AppEnv::Production,
            Some(other) => bail!("APP_ENV must be `development` or `production`, got `{other}`"),
        };

        let host: IpAddr =
            get("APP_HOST").unwrap_or_else(|| "127.0.0.1".into()).parse().context("APP_HOST must be an IP address")?;
        let port: u16 =
            get("APP_PORT").unwrap_or_else(|| "3000".into()).parse().context("APP_PORT must be a port number")?;

        let session_secret = required("SESSION_SECRET")?;
        if session_secret.len() < MIN_SESSION_SECRET_LEN {
            bail!("SESSION_SECRET must be at least {MIN_SESSION_SECRET_LEN} characters");
        }

        let cookie_secure = parse_bool("COOKIE_SECURE", get("COOKIE_SECURE"), env.is_production())?;
        let session_ttl_hours: u64 = get("SESSION_TTL_HOURS")
            .unwrap_or_else(|| "168".into())
            .parse()
            .context("SESSION_TTL_HOURS must be a positive integer")?;
        if session_ttl_hours == 0 || session_ttl_hours > 24 * 90 {
            bail!("SESSION_TTL_HOURS must be between 1 and 2160");
        }

        let frontend_origins: Vec<String> = get("FRONTEND_ORIGIN")
            .unwrap_or_else(|| "http://localhost:5173".into())
            .split(',')
            .map(|origin| normalize_origin(origin.trim()))
            .collect::<Result<_>>()?;

        let config = Self {
            env,
            host,
            port,
            database_url: Secret::new(required("DATABASE_URL")?),
            db_max_connections: get("DATABASE_MAX_CONNECTIONS")
                .unwrap_or_else(|| "10".into())
                .parse()
                .context("DATABASE_MAX_CONNECTIONS must be a positive integer")?,
            session_secret: Secret::new(session_secret),
            cookie_secure,
            session_ttl: Duration::from_secs(session_ttl_hours * 3600),
            bootstrap_admin_email: get("BOOTSTRAP_ADMIN_EMAIL").map(|e| e.to_lowercase()),
            bootstrap_admin_password: get("BOOTSTRAP_ADMIN_PASSWORD").map(Secret::new),
            frontend_origins,
            trust_proxy: parse_bool("TRUST_PROXY", get("TRUST_PROXY"), false)?,
            enable_api_docs: parse_bool("ENABLE_API_DOCS", get("ENABLE_API_DOCS"), !env.is_production())?,
            smtp: smtp_config(&get)?,
        };
        config.validate_for_environment()?;
        Ok(config)
    }

    /// Production refuses to start with settings that are only safe locally.
    fn validate_for_environment(&self) -> Result<()> {
        if !self.env.is_production() {
            return Ok(());
        }
        if !self.cookie_secure {
            bail!("COOKIE_SECURE must be true when APP_ENV=production");
        }
        if self.session_secret.expose().contains("change-me") {
            bail!("SESSION_SECRET still contains the placeholder value");
        }
        if let Some(password) = &self.bootstrap_admin_password
            && password.expose().contains("change-me")
        {
            bail!("BOOTSTRAP_ADMIN_PASSWORD still contains the placeholder value");
        }
        if let Some(origin) = self.frontend_origins.iter().find(|o| !o.starts_with("https://")) {
            bail!("FRONTEND_ORIGIN must use https in production, got `{origin}`");
        }
        if let Some(smtp) = &self.smtp
            && smtp.tls == SmtpTls::None
            && smtp.password.is_some()
        {
            bail!("SMTP_TLS=none would send the SMTP password unencrypted; use starttls or tls in production");
        }
        Ok(())
    }

    pub fn session_cookie_name(&self) -> &'static str {
        // The `__Host-` prefix makes browsers enforce Secure + Path=/ + no Domain.
        if self.cookie_secure { "__Host-cv_session" } else { "cv_session" }
    }

    /// Cookie marking a browser that completed the emailed-code step.
    pub fn device_cookie_name(&self) -> &'static str {
        if self.cookie_secure { "__Host-cv_device" } else { "cv_device" }
    }
}

/// `SMTP_HOST` switches outgoing email on; everything else then has defaults
/// except the sender address.
fn smtp_config(get: &impl Fn(&str) -> Option<String>) -> Result<Option<SmtpConfig>> {
    let Some(host) = get("SMTP_HOST") else {
        return Ok(None);
    };
    let tls = match get("SMTP_TLS").map(|v| v.to_ascii_lowercase()).as_deref() {
        None | Some("starttls") => SmtpTls::StartTls,
        Some("tls" | "ssl" | "implicit") => SmtpTls::Tls,
        Some("none" | "off") => SmtpTls::None,
        Some(other) => bail!("SMTP_TLS must be `starttls`, `tls` or `none`, got `{other}`"),
    };
    let default_port = match tls {
        SmtpTls::StartTls => 587,
        SmtpTls::Tls => 465,
        SmtpTls::None => 25,
    };
    let port = match get("SMTP_PORT") {
        Some(port) => port.parse().context("SMTP_PORT must be a port number")?,
        None => default_port,
    };
    let username = get("SMTP_USERNAME");
    let password = get("SMTP_PASSWORD").map(Secret::new);
    if username.is_some() != password.is_some() {
        bail!("set both SMTP_USERNAME and SMTP_PASSWORD, or neither");
    }
    let from = get("SMTP_FROM").context("SMTP_FROM must be set when SMTP_HOST is set")?;
    if !from.contains('@') {
        bail!("SMTP_FROM must contain an email address, e.g. `CommitVault <vault@example.com>`");
    }
    Ok(Some(SmtpConfig { host, port, tls, username, password, from }))
}

fn parse_bool(key: &str, value: Option<String>, default: bool) -> Result<bool> {
    match value.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None => Ok(default),
        Some("true" | "1" | "yes" | "on") => Ok(true),
        Some("false" | "0" | "no" | "off") => Ok(false),
        Some(other) => bail!("{key} must be a boolean, got `{other}`"),
    }
}

/// Reduces a configured origin to `scheme://host[:port]` and rejects wildcards.
fn normalize_origin(raw: &str) -> Result<String> {
    if raw.contains('*') {
        bail!("FRONTEND_ORIGIN must be an exact origin, wildcards are not allowed");
    }
    let url = url::Url::parse(raw).with_context(|| format!("FRONTEND_ORIGIN `{raw}` is not a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("FRONTEND_ORIGIN `{raw}` must be an http(s) origin");
    }
    Ok(url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn base() -> HashMap<&'static str, String> {
        HashMap::from([("DATABASE_URL", "postgres://u:p@localhost/db".to_owned()), ("SESSION_SECRET", "s".repeat(64))])
    }

    fn load(vars: &HashMap<&'static str, String>) -> Result<Config> {
        Config::from_lookup(|key| vars.get(key).cloned())
    }

    #[test]
    fn development_defaults() {
        let config = load(&base()).unwrap();
        assert_eq!(config.env, AppEnv::Development);
        assert_eq!(config.port, 3000);
        assert!(!config.cookie_secure);
        assert!(config.enable_api_docs);
        assert_eq!(config.frontend_origins, vec!["http://localhost:5173"]);
        assert_eq!(config.session_cookie_name(), "cv_session");
    }

    #[test]
    fn rejects_short_session_secret() {
        let mut vars = base();
        vars.insert("SESSION_SECRET", "short".into());
        assert!(load(&vars).unwrap_err().to_string().contains("SESSION_SECRET"));
    }

    #[test]
    fn production_requires_secure_settings() {
        let mut vars = base();
        vars.insert("APP_ENV", "production".into());
        vars.insert("FRONTEND_ORIGIN", "https://vault.example.com".into());
        let config = load(&vars).unwrap();
        assert!(config.cookie_secure, "secure cookies default to on in production");
        assert!(!config.enable_api_docs);
        assert_eq!(config.session_cookie_name(), "__Host-cv_session");

        vars.insert("COOKIE_SECURE", "false".into());
        assert!(load(&vars).is_err());

        vars.insert("COOKIE_SECURE", "true".into());
        vars.insert("FRONTEND_ORIGIN", "http://vault.example.com".into());
        assert!(load(&vars).is_err());
    }

    #[test]
    fn rejects_wildcard_origin_and_normalizes_paths() {
        let mut vars = base();
        vars.insert("FRONTEND_ORIGIN", "*".into());
        assert!(load(&vars).is_err());

        vars.insert("FRONTEND_ORIGIN", "http://localhost:5173/, https://a.example.com/app".into());
        let config = load(&vars).unwrap();
        assert_eq!(config.frontend_origins, vec!["http://localhost:5173", "https://a.example.com"]);
    }

    #[test]
    fn smtp_is_optional_and_validated() {
        assert!(load(&base()).unwrap().smtp.is_none());

        let mut vars = base();
        vars.insert("SMTP_HOST", "smtp.example.com".into());
        assert!(load(&vars).unwrap_err().to_string().contains("SMTP_FROM"));

        vars.insert("SMTP_FROM", "CommitVault <vault@example.com>".into());
        let smtp = load(&vars).unwrap().smtp.unwrap();
        assert_eq!((smtp.port, smtp.tls), (587, SmtpTls::StartTls));

        vars.insert("SMTP_TLS", "tls".into());
        assert_eq!(load(&vars).unwrap().smtp.unwrap().port, 465);

        vars.insert("SMTP_USERNAME", "vault".into());
        assert!(load(&vars).is_err(), "username without password");
        vars.insert("SMTP_PASSWORD", "smtp-secret-value".into());
        let config = load(&vars).unwrap();
        assert!(!format!("{config:?}").contains("smtp-secret-value"));

        vars.insert("SMTP_TLS", "none".into());
        vars.insert("APP_ENV", "production".into());
        vars.insert("FRONTEND_ORIGIN", "https://vault.example.com".into());
        assert!(load(&vars).unwrap_err().to_string().contains("SMTP_TLS"));
    }

    #[test]
    fn secrets_are_redacted_in_debug_output() {
        let config = load(&base()).unwrap();
        let debug = format!("{config:?}");
        assert!(!debug.contains("postgres://"));
        assert!(!debug.contains(&"s".repeat(64)));
    }
}
