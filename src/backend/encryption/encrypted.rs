//! Functionality related to the [Encrypted] struct.
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm,
};
use color_eyre::eyre::{self, eyre};

/// A 12-byte nonce used for AES-256 encryption and decryption.
pub type Aes256Nonce = [u8; 12];

/// A 32-byte key used for AES-256 encryption and decryption.
pub type Aes256Key = [u8; 32];

/// An encrypted byte array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encrypted {
    cipherbytes: Box<[u8]>,
    nonce: Aes256Nonce,
}
impl Encrypted {
    /// Encrypt a byte slice using a given [Aes256Key] and [Aes256Nonce].
    pub fn try_encrypt_bytes_key_nonce(
        byte_slice: &[u8],
        key: Aes256Key,
        nonce: Aes256Nonce,
    ) -> eyre::Result<Encrypted> {
        let cipher = Aes256Gcm::new(&key.into());
        match cipher.encrypt(&nonce.into(), &byte_slice[..]) {
            Ok(cipherbytes) => Ok(Self {
                cipherbytes: cipherbytes.into(),
                nonce,
            }),
            Err(err) => Err(eyre!("{err:?}")),
        }
    }

    /// Decrypt this [Encrypted] into a byte vector.
    pub fn try_decrypt_bytes(self, key: Aes256Key) -> eyre::Result<Vec<u8>> {
        let cipher = Aes256Gcm::new(&key.into());
        match cipher.decrypt(&self.nonce.into(), &self.cipherbytes[..]) {
            Ok(byte_vec) => Ok(byte_vec),
            Err(err) => Err(eyre!("{err:?}")),
        }
    }

    /// Return the cipherbytes of this [Encrypted].
    pub fn cipherbytes(&self) -> &[u8] {
        &self.cipherbytes
    }

    /// Return the [Aes256Nonce] of this [Encrypted].
    pub fn nonce(&self) -> Aes256Nonce {
        self.nonce
    }
}

#[cfg(test)]
mod tests {
    use camino::{Utf8Path, Utf8PathBuf};
    use pretty_assertions::assert_eq;

    use super::super::traits::*;

    #[test]
    fn aes_256_consistency() {
        let test_string = String::from("this is a test.");

        let (encrypted_1, key) = test_string.clone().try_encrypt_new_key().unwrap();
        let encrypted_2 = test_string
            .clone()
            .try_encrypt_with_both(key, encrypted_1.nonce())
            .unwrap();

        assert_eq!(encrypted_1, encrypted_2);

        let decrypted_1 = String::try_decrypt(encrypted_1, key).unwrap();
        let decrypted_2 = String::try_decrypt(encrypted_2, key).unwrap();

        assert_eq!(decrypted_1, test_string);
        assert_eq!(decrypted_1, decrypted_2);
    }

    #[test]
    fn check_utf8() {
        let test_str = "您好!";

        let (encrypted, key) = test_str.try_encrypt_new_key().unwrap();
        let decrypted = Vec::<u8>::try_decrypt(encrypted, key).unwrap();

        assert_eq!(test_str.as_bytes(), decrypted);
        assert_eq!(test_str, std::str::from_utf8(&decrypted).unwrap());
    }

    #[test]
    fn utf8_paths() {
        let test_path_buf = Utf8PathBuf::from("src/backend/encryption/encrypted.rs");
        let test_path: &Utf8Path = &test_path_buf;

        let (encrypted_1, key) = test_path_buf.clone().try_encrypt_new_key().unwrap();
        let encrypted_2 = test_path
            .try_encrypt_with_both(key, encrypted_1.nonce())
            .unwrap();

        assert_eq!(encrypted_1, encrypted_2);

        let decrypted_1: &[u8] = &Vec::<u8>::try_decrypt(encrypted_1, key).unwrap();
        let decrypted_2: &[u8] = &Vec::<u8>::try_decrypt(encrypted_2, key).unwrap();

        assert_eq!(decrypted_1, decrypted_2);
    }
}
