use sha2::{Digest, Sha256};

/// Hash a password string into a 32-byte seed for the ChaCha20 PRNG.
pub fn password_to_seed(password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&result);
    seed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic() {
        let s1 = password_to_seed("d1ng0");
        let s2 = password_to_seed("d1ng0");
        assert_eq!(s1, s2);
    }

    #[test]
    fn test_different_passwords() {
        let s1 = password_to_seed("d1ng0");
        let s2 = password_to_seed("wrong");
        assert_ne!(s1, s2);
    }
}
