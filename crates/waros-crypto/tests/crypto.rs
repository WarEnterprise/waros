use std::time::Instant;

use waros_crypto::{
    hash,
    kem::{self, Ciphertext, SecurityLevel},
    qrng,
    sign::{self, Signature, SignatureScheme},
    CryptoError,
};

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn kem_round_trip_level1() {
    let (pk, sk) = kem::keygen_with_level(SecurityLevel::Level1);
    let (ct, shared_secret_enc) = kem::encapsulate(&pk);
    let shared_secret_dec = kem::decapsulate(&sk, &ct).expect("decapsulation succeeds");
    assert_eq!(shared_secret_enc, shared_secret_dec);
}

#[test]
fn kem_round_trip_level3() {
    let (pk, sk) = kem::keygen_with_level(SecurityLevel::Level3);
    let (ct, shared_secret_enc) = kem::encapsulate(&pk);
    let shared_secret_dec = kem::decapsulate(&sk, &ct).expect("decapsulation succeeds");
    assert_eq!(shared_secret_enc, shared_secret_dec);
}

#[test]
fn kem_round_trip_level5() {
    let (pk, sk) = kem::keygen_with_level(SecurityLevel::Level5);
    let (ct, shared_secret_enc) = kem::encapsulate(&pk);
    let shared_secret_dec = kem::decapsulate(&sk, &ct).expect("decapsulation succeeds");
    assert_eq!(shared_secret_enc, shared_secret_dec);
}

#[test]
fn kem_decapsulate_with_wrong_secret_key_fails() {
    let (pk, _) = kem::keygen();
    let (_, wrong_sk) = kem::keygen();
    let (ct, _) = kem::encapsulate(&pk);
    let error = kem::decapsulate(&wrong_sk, &ct).expect_err("wrong key must fail");
    assert!(matches!(error, CryptoError::IntegrityCheckFailed));
}

#[test]
fn kem_ciphertext_tampering_detected() {
    let (pk, sk) = kem::keygen();
    let (ct, _) = kem::encapsulate(&pk);
    let mut tampered = ct.as_bytes().to_vec();
    tampered[0] ^= 0x80;
    let tampered = Ciphertext::from_bytes(ct.security_level(), &tampered, ct.integrity_tag())
        .expect("serialized ciphertext remains structurally valid");
    let error = kem::decapsulate(&sk, &tampered).expect_err("tampering must fail");
    assert!(matches!(
        error,
        CryptoError::IntegrityCheckFailed | CryptoError::InvalidKeyMaterial(_)
    ));
}

#[test]
fn sign_verify_round_trip_mldsa() {
    let (pk, sk) = sign::keygen();
    let message = b"waros";
    let signature = sign::sign(&sk, message);
    assert!(sign::verify(&pk, message, &signature));
}

#[test]
fn sign_verify_round_trip_slhdsa() {
    let (pk, sk) = sign::keygen_with_scheme(SignatureScheme::SlhDsa);
    let message = b"waros";
    let signature = sign::sign(&sk, message);
    assert!(sign::verify(&pk, message, &signature));
}

#[test]
fn verification_fails_with_wrong_public_key() {
    let (_, sk) = sign::keygen();
    let (wrong_pk, _) = sign::keygen();
    let signature = sign::sign(&sk, b"waros");
    assert!(!sign::verify(&wrong_pk, b"waros", &signature));
}

#[test]
fn verification_fails_with_modified_message() {
    let (pk, sk) = sign::keygen();
    let signature = sign::sign(&sk, b"waros");
    assert!(!sign::verify(&pk, b"war0s", &signature));
}

#[test]
fn verification_fails_with_modified_signature() {
    let (pk, sk) = sign::keygen();
    let signature = sign::sign(&sk, b"waros");
    let mut tampered = signature.as_bytes().to_vec();
    tampered[0] ^= 0x01;
    match Signature::from_bytes(signature.scheme(), &tampered) {
        Ok(tampered) => assert!(!sign::verify(&pk, b"waros", &tampered)),
        Err(_) => {}
    }
}

#[test]
fn sha3_256_known_vector() {
    assert_eq!(
        hex_encode(&hash::sha3_256(b"")),
        "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a"
    );
}

#[test]
fn sha3_512_known_vector() {
    assert_eq!(
        hex_encode(&hash::sha3_512(b"")),
        concat!(
            "a69f73cca23a9ac5c8b567dc185a756e97c982164fe25859e0d1dcc1475c80a6",
            "15b2123af1f5f94c11e3e9402c3ac558f500199d95b6d3e301758586281dcd26"
        )
    );
}

#[test]
fn shake128_known_vector() {
    assert_eq!(
        hex_encode(&hash::shake128(b"", 32)),
        "7f9c2ba4e88f827d616045507605853ed73b8093f6efbc88eb1a6eacfa66ef26"
    );
}

#[test]
fn shake256_known_vector() {
    assert_eq!(
        hex_encode(&hash::shake256(b"", 64)),
        concat!(
            "46b9dd2b0ba88d13233b3feb743eeb243fcd52ea62b81b82b50c27646ed5762f",
            "d75dc4ddd8c0f200cb05019d67b592f6fc821c49479ab48640292eacb3b7c4be"
        )
    );
}

#[test]
fn hash_empty_input_lengths() {
    assert_eq!(hash::sha3_256(b"").len(), 32);
    assert_eq!(hash::sha3_512(b"").len(), 64);
    assert_eq!(hash::shake128(b"", 17).len(), 17);
    assert_eq!(hash::shake256(b"", 23).len(), 23);
}

#[test]
fn hash_large_input_is_stable() {
    let message = vec![0xa5; 1_048_576];
    assert_eq!(hash::sha3_256(&message), hash::sha3_256(&message));
    assert_eq!(hash::shake256(&message, 64), hash::shake256(&message, 64));
}

#[test]
fn qrng_random_bytes_returns_correct_length() {
    assert_eq!(qrng::random_bytes(32).len(), 32);
}

#[test]
fn qrng_random_bytes_differ_between_calls() {
    assert_ne!(qrng::random_bytes(32), qrng::random_bytes(32));
}

#[test]
fn qrng_random_bits_distribution_is_balanced() {
    let bits = qrng::random_bits(10_000);
    let ones = bits.iter().filter(|bit| **bit).count() as f64;
    let zeros = bits.len() as f64 - ones;
    let expected = bits.len() as f64 / 2.0;
    let chi_squared =
        ((zeros - expected).powi(2) / expected) + ((ones - expected).powi(2) / expected);
    assert!(chi_squared < 12.0, "chi-squared was {chi_squared:.4}");
}

#[test]
fn qrng_random_seed_has_expected_size() {
    assert_eq!(qrng::random_seed().len(), 32);
}

#[test]
fn integration_sign_kem_verify_workflow() {
    let message = b"WarOS hybrid quantum-classical stack";
    let (sign_pk, sign_sk) = sign::keygen();
    let signature = sign::sign(&sign_sk, message);
    assert!(sign::verify(&sign_pk, message, &signature));

    let (kem_pk, kem_sk) = kem::keygen();
    let (ciphertext, shared_secret_enc) = kem::encapsulate(&kem_pk);
    let shared_secret_dec = kem::decapsulate(&kem_sk, &ciphertext).expect("decapsulation succeeds");
    assert_eq!(shared_secret_enc, shared_secret_dec);
}

#[test]
fn benchmark_keygen_encapsulate_and_sign_latencies() {
    let start = Instant::now();
    let (pk, sk) = kem::keygen();
    let keygen_elapsed = start.elapsed();

    let start = Instant::now();
    let (_ct, _ss) = kem::encapsulate(&pk);
    let encapsulate_elapsed = start.elapsed();

    let (_sign_pk, sign_sk) = sign::keygen();
    let start = Instant::now();
    let _signature = sign::sign(&sign_sk, b"waros");
    let sign_elapsed = start.elapsed();

    assert!(keygen_elapsed.as_nanos() > 0);
    assert!(encapsulate_elapsed.as_nanos() > 0);
    assert!(sign_elapsed.as_nanos() > 0);
    let _ = sk;
}
