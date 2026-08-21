use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuthUser {
    pub id: String,
    pub email: String,
    pub avatar_url: Option<String>,
}

pub fn avatar_content_url(asset_id: &str) -> String {
    format!("/v1/assets/{asset_id}/content")
}

pub fn normalize_email(email: &str) -> anyhow::Result<String> {
    let email = email.trim().to_ascii_lowercase();
    if email.len() > 254 || !email.contains('@') || email.starts_with('@') || email.ends_with('@') {
        anyhow::bail!("请输入有效的邮箱地址");
    }
    Ok(email)
}

pub fn validate_password(password: &str) -> anyhow::Result<()> {
    if password.chars().count() < 8 {
        anyhow::bail!("密码至少需要 8 个字符");
    }
    if password.len() > 256 {
        anyhow::bail!("密码过长");
    }
    Ok(())
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    validate_password(password)?;
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!("密码加密失败: {error}"))?
        .to_string())
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded).ok().is_some_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

pub fn new_access_token() -> String {
    format!("rpl_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn secret_hash(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_and_verifies_passwords() {
        let hash = hash_password("correct-horse").unwrap();
        assert!(verify_password("correct-horse", &hash));
        assert!(!verify_password("wrong-horse", &hash));
    }

    #[test]
    fn normalizes_email_addresses() {
        assert_eq!(
            normalize_email(" User@Example.COM ").unwrap(),
            "user@example.com"
        );
        assert!(normalize_email("invalid").is_err());
    }
}
