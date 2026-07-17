//! Шифрование offline-токенов: AES-256-GCM, ключ из ENCRYPTION_KEY (base64, 32 байта).
//! Формат блоба: nonce(12) || ciphertext+tag.

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;

#[derive(Clone)]
pub struct TokenCipher(Aes256Gcm);

impl TokenCipher {
    pub fn from_base64(key_b64: &str) -> anyhow::Result<Self> {
        let key = base64::engine::general_purpose::STANDARD
            .decode(key_b64.trim())
            .map_err(|_| anyhow::anyhow!("ENCRYPTION_KEY: не base64"))?;
        if key.len() != 32 {
            anyhow::bail!("ENCRYPTION_KEY: нужно 32 байта, а не {}", key.len());
        }
        Ok(Self(Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key))))
    }

    pub fn encrypt(&self, plaintext: &str) -> Vec<u8> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self
            .0
            .encrypt(&nonce, plaintext.as_bytes())
            .expect("AES-GCM encrypt не падает на корректном ключе");
        let mut out = nonce.to_vec();
        out.extend(ct);
        out
    }

    pub fn decrypt(&self, blob: &[u8]) -> anyhow::Result<String> {
        if blob.len() < 12 {
            anyhow::bail!("шифроблоб короче nonce");
        }
        let (nonce, ct) = blob.split_at(12);
        let pt = self
            .0
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| anyhow::anyhow!("не расшифровалось (сменился ENCRYPTION_KEY?)"))?;
        Ok(String::from_utf8(pt)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher() -> TokenCipher {
        // 32 нулевых байта — только для тестов
        TokenCipher::from_base64(&base64::engine::general_purpose::STANDARD.encode([0u8; 32]))
            .unwrap()
    }

    #[test]
    fn roundtrip() {
        let c = cipher();
        let blob = c.encrypt("секретный-офлайн-токен");
        assert_eq!(c.decrypt(&blob).unwrap(), "секретный-офлайн-токен");
        // nonce случайный — два шифрования различаются
        assert_ne!(blob, c.encrypt("секретный-офлайн-токен"));
    }

    #[test]
    fn порченый_блоб_не_расшифровывается() {
        let c = cipher();
        let mut blob = c.encrypt("x");
        *blob.last_mut().unwrap() ^= 1;
        assert!(c.decrypt(&blob).is_err());
        assert!(c.decrypt(&[1, 2, 3]).is_err());
    }

    #[test]
    fn плохой_ключ() {
        assert!(TokenCipher::from_base64("не base64!").is_err());
        assert!(TokenCipher::from_base64(
            &base64::engine::general_purpose::STANDARD.encode([0u8; 16])
        )
        .is_err());
    }
}
