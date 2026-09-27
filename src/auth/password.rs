use std::sync::OnceLock;

use anyhow::{Context, anyhow};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};

pub const MIN_LENGTH: usize = 8;
pub const MAX_LENGTH: usize = 128;

/// Password rules carried over from the original API: a minimum length, at least one
/// uppercase letter, at least one special character, and no embedded username.
pub fn check_policy(password: &str, username: &str) -> Result<(), &'static str> {
    let length = password.chars().count();
    if length < MIN_LENGTH {
        return Err("Password must be at least 8 characters long");
    }
    if length > MAX_LENGTH {
        return Err("Password must be at most 128 characters long");
    }
    if !password.chars().any(char::is_uppercase) {
        return Err("Password must contain at least one uppercase letter");
    }
    if !password
        .chars()
        .any(|c| !c.is_alphanumeric() && !c.is_whitespace())
    {
        return Err("Password must contain at least one special character");
    }
    let username = username.trim().to_lowercase();
    if !username.is_empty() && password.to_lowercase().contains(&username) {
        return Err("Password cannot contain the username");
    }
    Ok(())
}

/// Hashes `password` with Argon2id on the blocking pool.
pub async fn hash(password: String) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || hash_blocking(&password))
        .await
        .context("password hashing task panicked")?
}

/// Verifies `password` against a PHC hash string on the blocking pool.
pub async fn verify(password: String, phc: String) -> anyhow::Result<bool> {
    tokio::task::spawn_blocking(move || verify_blocking(&password, &phc))
        .await
        .context("password verification task panicked")?
}

/// Runs a verification against a fixed hash so unknown logins cost as much as wrong
/// passwords, which keeps response timing from revealing which accounts exist.
pub async fn verify_dummy(password: String) -> anyhow::Result<()> {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let phc = match DUMMY.get() {
        Some(phc) => phc.clone(),
        None => {
            let phc = hash("dummy-password-for-timing".to_owned()).await?;
            DUMMY.get_or_init(|| phc).clone()
        }
    };
    verify(password, phc).await.map(|_| ())
}

fn hash_blocking(password: &str) -> anyhow::Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|err| anyhow!("failed to hash password: {err}"))
}

fn verify_blocking(password: &str, phc: &str) -> anyhow::Result<bool> {
    let parsed =
        PasswordHash::new(phc).map_err(|err| anyhow!("stored password hash is invalid: {err}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_strong_password() {
        assert_eq!(check_policy("Barbell#2026", "oskar"), Ok(()));
    }

    #[test]
    fn enforces_each_rule() {
        let cases = [
            ("Sh0rt!", "Password must be at least 8 characters long"),
            (
                "lowercase!only",
                "Password must contain at least one uppercase letter",
            ),
            (
                "NoSpecial123",
                "Password must contain at least one special character",
            ),
            ("MyOskar!pass", "Password cannot contain the username"),
        ];
        for (password, expected) in cases {
            assert_eq!(
                check_policy(password, "oskar"),
                Err(expected),
                "password: {password}"
            );
        }
    }

    #[test]
    fn rejects_overlong_password() {
        let password = format!("A!{}", "a".repeat(MAX_LENGTH));
        assert!(check_policy(&password, "user").is_err());
    }

    #[tokio::test]
    async fn hash_round_trips() -> anyhow::Result<()> {
        let phc = hash("Barbell#2026".to_owned()).await?;
        assert!(phc.starts_with("$argon2id$"));
        assert!(verify("Barbell#2026".to_owned(), phc.clone()).await?);
        assert!(!verify("barbell#2026".to_owned(), phc).await?);
        Ok(())
    }
}
