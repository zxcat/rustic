use crate::backend::{FileType, ReadBackend};
use crate::crypto::Key;
use crate::id::Id;

use anyhow::{anyhow, Result};
use scrypt::Params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct KeyFile {
    //    hostname: String,
    //    username: String,
    kdf: String,
    #[serde(rename = "N")]
    n: u32,
    r: u32,
    p: u32,
    //    created: String,
    data: String,
    salt: String,
}

impl KeyFile {
    /// Generate a Key using the key derivation function from KeyFile and a given password
    pub fn kdf_key(&self, passwd: &str) -> Result<Key> {
        let params = Params::new(log_2(self.n), self.r, self.p)
            .map_err(|_| anyhow!("invalid scrypt paramters"))?;
        let salt = base64::decode(&self.salt)?;

        let mut key = [0; 64];
        scrypt::scrypt(passwd.as_bytes(), &salt, &params, &mut key)
            .expect("output length invalid?");

        Ok(Key::from_slice(&key))
    }

    /// Extract a key from the data of the KeyFile using the given key.
    /// The key usually should be the key generated by kdf_key
    pub fn key_from_data(&self, key: &Key) -> Result<Key> {
        let dec_data = key
            .decrypt_data(&base64::decode(&self.data)?)
            .map_err(|_| anyhow!("decryption failed"))?;
        serde_json::from_slice::<MasterKey>(&dec_data)?.key()
    }

    /// Extract a key from the data of the KeyFile using the key
    /// from the derivation function in combination with the given password.
    pub fn key_from_password(&self, passwd: &str) -> Result<Key> {
        self.key_from_data(&self.kdf_key(passwd)?)
    }
}

impl KeyFile {
    /// Get a KeyFile from the backend
    pub fn from_backend<B: ReadBackend>(be: &B, id: Id) -> Result<Self> {
        let data = be.read_full(FileType::Key, id)?;
        Ok(serde_json::from_slice(&data)?)
    }
}

const fn num_bits<T>() -> usize {
    std::mem::size_of::<T>() * 8
}

fn log_2(x: u32) -> u8 {
    assert!(x > 0);
    (num_bits::<u32>() as u32 - x.leading_zeros() - 1)
        .try_into()
        .unwrap()
}

#[derive(Debug, Deserialize)]
struct Mac {
    k: String,
    r: String,
}

#[derive(Debug, Deserialize)]
struct MasterKey {
    mac: Mac,
    encrypt: String,
}

impl MasterKey {
    fn key(&self) -> Result<Key> {
        Ok(Key::from_keys(
            &base64::decode(&self.encrypt)?,
            &base64::decode(&self.mac.k)?,
            &base64::decode(&self.mac.r)?,
        ))
    }
}

/// Find a KeyFile in the backend that fits to the given password and return the contained key
/// If a key hint is given, only this key is tested (recommended for a large number of keys)
pub fn find_key_in_backend<B: ReadBackend>(be: &B, passwd: &str, hint: Option<Id>) -> Result<Key> {
    match hint {
        Some(id) => KeyFile::from_backend(be, id)?.key_from_password(passwd),
        None => be
            .list(FileType::Key)?
            .iter()
            .find_map(|&id| {
                KeyFile::from_backend(be, id)
                    .ok()?
                    .key_from_password(passwd)
                    .ok()
            })
            .ok_or(anyhow!("no suitable key found!")),
    }
}
