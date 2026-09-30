use rand::Rng;
use argon2::{
    password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
    Argon2, PasswordHash, PasswordVerifier,
};

pub fn generate_code() -> String {
    let mut rng = rand::rng();
    let n: u32 = rng.random_range(0..1_000_000);
    format!("{n:06}")
}



#[derive(Debug, thiserror::Error)]
pub enum HashError {
    #[error("hashing failed: {0}")]
    Hash(String),
}

pub fn hash_code(code: &str) -> Result<String, HashError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(code.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| HashError::Hash(e.to_string()))
}

pub fn verify_code(code: &str, hash: &str) -> Result<bool, HashError> {
    let parsed = PasswordHash::new(hash).map_err(|e| HashError::Hash(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(code.as_bytes(), &parsed)
        .is_ok())
}
