//! Provider credentials: PEM keys and the two JWTs, signed with aws-lc-rs.
//! Errors never include key material.
use aws_lc_rs::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, RSA_PKCS1_SHA256, RsaKeyPair},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde_json::Value;

/// PKCS#8 DER from a `-----BEGIN PRIVATE KEY-----` block. Secrets stored as
/// JSON often carry literal `\n` escapes instead of newlines; both work.
pub(super) fn pkcs8_der(pem: &str) -> Result<Vec<u8>, &'static str> {
    let pem = pem.replace("\\n", "\n");
    let body = pem
        .trim()
        .strip_prefix("-----BEGIN PRIVATE KEY-----")
        .and_then(|rest| rest.strip_suffix("-----END PRIVATE KEY-----"))
        .ok_or("must be a PEM PKCS#8 private key")?;
    let encoded: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    STANDARD
        .decode(encoded)
        .map_err(|_| "must be a PEM PKCS#8 private key")
}

pub(super) fn ecdsa_key(pem: &str) -> Result<EcdsaKeyPair, &'static str> {
    EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8_der(pem)?)
        .map_err(|_| "must be a P-256 key")
}

pub(super) fn rsa_key(pem: &str) -> Result<RsaKeyPair, &'static str> {
    RsaKeyPair::from_pkcs8(&pkcs8_der(pem)?).map_err(|_| "must be an RSA key")
}

fn signing_input(header: &Value, claims: &Value) -> String {
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    )
}

/// ES256 JWT (fixed-size `r || s` signature), as APNs requires.
pub(super) fn es256(key: &EcdsaKeyPair, header: &Value, claims: &Value) -> Result<String, ()> {
    let input = signing_input(header, claims);
    let signature = key
        .sign(&SystemRandom::new(), input.as_bytes())
        .map_err(|_| ())?;
    Ok(format!(
        "{input}.{}",
        URL_SAFE_NO_PAD.encode(signature.as_ref())
    ))
}

/// RS256 JWT, as Google's OAuth JWT-bearer grant requires.
pub(super) fn rs256(key: &RsaKeyPair, header: &Value, claims: &Value) -> Result<String, ()> {
    let input = signing_input(header, claims);
    let mut signature = vec![0; key.public_modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        input.as_bytes(),
        &mut signature,
    )
    .map_err(|_| ())?;
    Ok(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

#[cfg(test)]
pub(super) mod testing {
    //! Throwaway keys generated per test run, so no key is ever committed.
    use super::*;
    use aws_lc_rs::{
        encoding::AsDer,
        rsa::KeySize,
        signature::{
            ECDSA_P256_SHA256_FIXED, KeyPair, RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey,
        },
    };

    pub(crate) fn pem(der: &[u8]) -> String {
        let body = STANDARD.encode(der);
        let lines: Vec<&str> = body
            .as_bytes()
            .chunks(64)
            .map(|line| std::str::from_utf8(line).unwrap())
            .collect();
        format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
            lines.join("\n")
        )
    }

    /// A P-256 key as a `.p8` PEM, and its public key for verification.
    pub(crate) fn p8() -> (String, Vec<u8>) {
        let document =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .unwrap();
        let key =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, document.as_ref()).unwrap();
        (pem(document.as_ref()), key.public_key().as_ref().to_vec())
    }

    /// An RSA key as a service-account PEM, and its public key.
    pub(crate) fn rsa() -> (String, Vec<u8>) {
        let key = RsaKeyPair::generate(KeySize::Rsa2048).unwrap();
        let der = AsDer::<aws_lc_rs::encoding::Pkcs8V1Der>::as_der(&key).unwrap();
        (pem(der.as_ref()), key.public_key().as_ref().to_vec())
    }

    /// Verifies a JWT's signature and returns its header and claims.
    pub(crate) fn verify(jwt: &str, public_key: &[u8], ecdsa: bool) -> Option<(Value, Value)> {
        let (input, signature) = jwt.rsplit_once('.')?;
        let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
        let algorithm: &dyn aws_lc_rs::signature::VerificationAlgorithm = if ecdsa {
            &ECDSA_P256_SHA256_FIXED
        } else {
            &RSA_PKCS1_2048_8192_SHA256
        };
        UnparsedPublicKey::new(algorithm, public_key)
            .verify(input.as_bytes(), &signature)
            .ok()?;
        let (header, claims) = input.split_once('.')?;
        let decode = |part: &str| serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).ok()?).ok();
        Some((decode(header)?, decode(claims)?))
    }
}

#[cfg(test)]
mod tests {
    use super::{testing::*, *};
    use serde_json::json;

    #[test]
    fn pem_keys_load_with_real_newlines_or_json_escapes() {
        let (p8, public) = p8();
        let escaped = p8.replace('\n', "\\n");
        for pem in [p8.as_str(), escaped.as_str()] {
            let key = ecdsa_key(pem).unwrap();
            let jwt = es256(
                &key,
                &json!({"alg":"ES256","kid":"KEY"}),
                &json!({"iss":"TEAM"}),
            )
            .unwrap();
            let (header, claims) = verify(&jwt, &public, true).unwrap();
            assert_eq!(header["kid"], "KEY");
            assert_eq!(claims["iss"], "TEAM");
        }
        let (service, public) = rsa();
        let key = rsa_key(&service.replace('\n', "\\n")).unwrap();
        let jwt = rs256(&key, &json!({"alg":"RS256"}), &json!({"aud":"x"})).unwrap();
        assert_eq!(verify(&jwt, &public, false).unwrap().1["aud"], "x");
        assert!(verify(&jwt, &public, true).is_none());
    }

    #[test]
    fn rejects_wrong_or_malformed_keys_without_echoing_them() {
        let (p8, _) = p8();
        let (service, _) = rsa();
        assert_eq!(
            ecdsa_key("not a key").err(),
            Some("must be a PEM PKCS#8 private key")
        );
        assert_eq!(
            ecdsa_key("-----BEGIN PRIVATE KEY-----\n!!\n-----END PRIVATE KEY-----").err(),
            Some("must be a PEM PKCS#8 private key")
        );
        assert_eq!(ecdsa_key(&service).err(), Some("must be a P-256 key"));
        assert_eq!(rsa_key(&p8).err(), Some("must be an RSA key"));
    }
}
