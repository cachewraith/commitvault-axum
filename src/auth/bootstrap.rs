//! First-run administrator creation from `BOOTSTRAP_ADMIN_*`.

use anyhow::{Context, bail};
use sqlx::PgPool;

use super::{dto::MIN_PASSWORD_LEN, password, repository};
use crate::config::Config;

/// Creates the administrator account when the `users` table is empty.
///
/// Once any user exists the bootstrap variables are ignored, so changing them
/// later never overwrites a password that was changed inside the application.
pub async fn ensure_admin(pool: &PgPool, config: &Config) -> anyhow::Result<()> {
    if repository::count(pool).await? > 0 {
        return Ok(());
    }
    let (Some(email), Some(admin_password)) = (&config.bootstrap_admin_email, &config.bootstrap_admin_password) else {
        tracing::warn!(
            "no user exists and BOOTSTRAP_ADMIN_EMAIL / BOOTSTRAP_ADMIN_PASSWORD are not set; nobody can sign in"
        );
        return Ok(());
    };
    if !email.contains('@') || email.len() > 254 {
        bail!("BOOTSTRAP_ADMIN_EMAIL is not a valid email address");
    }
    if (admin_password.expose().chars().count() as u64) < MIN_PASSWORD_LEN {
        bail!("BOOTSTRAP_ADMIN_PASSWORD must be at least {MIN_PASSWORD_LEN} characters");
    }

    let hash = password::hash(admin_password.expose().to_owned()).await?;

    // The advisory lock makes "check empty, then insert" atomic even if two
    // instances start at the same moment.
    let mut tx = pool.begin().await?;
    sqlx::query!("SELECT pg_advisory_xact_lock(hashtext('commitvault.bootstrap_admin'))").execute(&mut *tx).await?;
    if repository::count(&mut *tx).await? == 0 {
        repository::insert(&mut *tx, email, &hash).await.context("failed to create the bootstrap administrator")?;
        tracing::info!(%email, "created bootstrap administrator");
    }
    tx.commit().await?;
    Ok(())
}
