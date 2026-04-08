use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

/// Generate a deterministic permutation of indices [0..n) using a ChaCha20 PRNG
/// seeded by the given 32-byte seed. Uses the Fisher-Yates shuffle algorithm.
pub fn generate_permutation(n: usize, seed: &[u8; 32]) -> Vec<usize> {
    let mut rng = ChaCha20Rng::from_seed(*seed);
    let mut perm: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = rng.gen_range(0..=i);
        perm.swap(i, j);
    }
    perm
}

/// Scramble bits according to a permutation.
/// `scrambled[perm[i]] = bits[i]` for all i.
pub fn scramble(bits: &[bool], perm: &[usize]) -> Vec<bool> {
    assert_eq!(bits.len(), perm.len());
    let mut scrambled = vec![false; bits.len()];
    for (i, &idx) in perm.iter().enumerate() {
        scrambled[idx] = bits[i];
    }
    scrambled
}

/// Unscramble bits according to a permutation (inverse of scramble).
/// `bits[i] = scrambled[perm[i]]` for all i.
pub fn unscramble(scrambled: &[bool], perm: &[usize]) -> Vec<bool> {
    assert_eq!(scrambled.len(), perm.len());
    let mut bits = vec![false; scrambled.len()];
    for (i, &idx) in perm.iter().enumerate() {
        bits[i] = scrambled[idx];
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scramble_unscramble_round_trip() {
        let seed = [42u8; 32];
        let bits: Vec<bool> = vec![true, false, true, true, false, false, true, false];
        let perm = generate_permutation(bits.len(), &seed);
        let scrambled = scramble(&bits, &perm);
        let recovered = unscramble(&scrambled, &perm);
        assert_eq!(bits, recovered);
    }

    #[test]
    fn test_permutation_is_valid() {
        let seed = [7u8; 32];
        let n = 100;
        let perm = generate_permutation(n, &seed);

        // All indices present exactly once
        let mut sorted = perm.clone();
        sorted.sort();
        let expected: Vec<usize> = (0..n).collect();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn test_permutation_deterministic() {
        let seed = [99u8; 32];
        let n = 50;
        let perm1 = generate_permutation(n, &seed);
        let perm2 = generate_permutation(n, &seed);
        assert_eq!(perm1, perm2);
    }

    #[test]
    fn test_different_seeds_different_permutations() {
        let seed1 = [1u8; 32];
        let seed2 = [2u8; 32];
        let n = 50;
        let perm1 = generate_permutation(n, &seed1);
        let perm2 = generate_permutation(n, &seed2);
        assert_ne!(perm1, perm2);
    }
}
