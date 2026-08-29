use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::digest;
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::model::{BrokerEnvironment, CredentialRef, ProviderId};

const ALGORITHM: &str = "AES-256-GCM";
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialContext {
    pub provider: ProviderId,
    pub environment: BrokerEnvironment,
}

pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, SecretStoreError> {
        let value = value.into();
        if value.is_empty() {
            return Err(SecretStoreError::EmptySecret);
        }
        if value.len() > 64 * 1024 {
            return Err(SecretStoreError::SecretTooLarge);
        }
        Ok(Self(Zeroizing::new(value)))
    }

    #[must_use]
    pub fn expose_secret(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretBytes([REDACTED])")
    }
}

impl fmt::Display for SecretBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

pub struct KeyMaterial(Zeroizing<[u8; KEY_LEN]>);

impl KeyMaterial {
    #[must_use]
    pub fn new(value: [u8; KEY_LEN]) -> Self {
        Self(Zeroizing::new(value))
    }

    fn expose(&self) -> &[u8] {
        &self.0[..]
    }
}

impl fmt::Debug for KeyMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("KeyMaterial([REDACTED])")
    }
}

pub trait KeyProvider: Send + Sync {
    fn active_key_version(&self) -> Result<u32, KeyProviderError>;
    fn resolve_key_material(&self, version: u32) -> Result<KeyMaterial, KeyProviderError>;
    fn rotate_key(&self, version: u32, material: KeyMaterial) -> Result<(), KeyProviderError>;
}

#[derive(Clone)]
pub struct StaticKeyProvider {
    keyring: Arc<RwLock<Keyring>>,
}

struct Keyring {
    active_version: u32,
    keys: BTreeMap<u32, Zeroizing<[u8; KEY_LEN]>>,
}

impl StaticKeyProvider {
    pub fn new(
        current_version: u32,
        keys: BTreeMap<u32, [u8; KEY_LEN]>,
    ) -> Result<Self, KeyProviderError> {
        if current_version == 0 || !keys.contains_key(&current_version) {
            return Err(KeyProviderError::MissingVersion(current_version));
        }
        Ok(Self {
            keyring: Arc::new(RwLock::new(Keyring {
                active_version: current_version,
                keys: keys
                    .into_iter()
                    .map(|(version, key)| (version, Zeroizing::new(key)))
                    .collect(),
            })),
        })
    }

    pub fn from_hex_environment(
        current_version_variable: &str,
        key_variable_prefix: &str,
    ) -> Result<Self, KeyProviderError> {
        let version_text = std::env::var(current_version_variable)
            .map_err(|_| KeyProviderError::MissingExternalKey)?;
        let current_version = version_text
            .parse::<u32>()
            .map_err(|_| KeyProviderError::InvalidExternalKey)?;
        let mut keys = BTreeMap::new();
        for (variable, value) in std::env::vars() {
            let Some(version_text) = variable.strip_prefix(key_variable_prefix) else {
                continue;
            };
            let Ok(version) = version_text.parse::<u32>() else {
                continue;
            };
            let key_text = Zeroizing::new(value);
            keys.insert(version, parse_hex_key(&key_text)?);
        }
        Self::new(current_version, keys)
    }
}

impl KeyProvider for StaticKeyProvider {
    fn active_key_version(&self) -> Result<u32, KeyProviderError> {
        Ok(self
            .keyring
            .read()
            .map_err(|_| KeyProviderError::Unavailable)?
            .active_version)
    }

    fn resolve_key_material(&self, version: u32) -> Result<KeyMaterial, KeyProviderError> {
        self.keyring
            .read()
            .map_err(|_| KeyProviderError::Unavailable)?
            .keys
            .get(&version)
            .map(|value| KeyMaterial::new(**value))
            .ok_or(KeyProviderError::MissingVersion(version))
    }

    fn rotate_key(&self, version: u32, material: KeyMaterial) -> Result<(), KeyProviderError> {
        let mut keyring = self
            .keyring
            .write()
            .map_err(|_| KeyProviderError::Unavailable)?;
        if version == 0 || version <= keyring.active_version || keyring.keys.contains_key(&version)
        {
            return Err(KeyProviderError::NonMonotonicVersion);
        }
        let mut key = [0_u8; KEY_LEN];
        key.copy_from_slice(material.expose());
        keyring.keys.insert(version, Zeroizing::new(key));
        keyring.active_version = version;
        Ok(())
    }
}

fn parse_hex_key(value: &str) -> Result<[u8; KEY_LEN], KeyProviderError> {
    if value.len() != KEY_LEN * 2 {
        return Err(KeyProviderError::InvalidExternalKey);
    }
    let mut output = [0_u8; KEY_LEN];
    for (index, byte) in output.iter_mut().enumerate() {
        let offset = index * 2;
        *byte = u8::from_str_radix(&value[offset..offset + 2], 16)
            .map_err(|_| KeyProviderError::InvalidExternalKey)?;
    }
    Ok(output)
}

pub trait SecretStore: Send + Sync {
    fn put(
        &self,
        credential_ref: &CredentialRef,
        context: &CredentialContext,
        secret: SecretBytes,
        now_unix_ms: i64,
    ) -> Result<String, SecretStoreError>;

    fn get(
        &self,
        credential_ref: &CredentialRef,
        expected_context: &CredentialContext,
    ) -> Result<SecretBytes, SecretStoreError>;

    fn rotate(
        &self,
        credential_ref: &CredentialRef,
        expected_context: &CredentialContext,
        secret: SecretBytes,
        now_unix_ms: i64,
    ) -> Result<String, SecretStoreError>;

    fn rewrap(
        &self,
        credential_ref: &CredentialRef,
        expected_context: &CredentialContext,
        now_unix_ms: i64,
    ) -> Result<(), SecretStoreError>;

    fn disable(&self, credential_ref: &CredentialRef) -> Result<(), SecretStoreError>;

    fn delete(&self, credential_ref: &CredentialRef) -> Result<(), SecretStoreError>;
}

#[derive(Clone)]
pub struct SqliteSecretStore<K> {
    path: PathBuf,
    key_provider: K,
}

impl<K: KeyProvider> SqliteSecretStore<K> {
    pub fn open(path: impl AsRef<Path>, key_provider: K) -> Result<Self, SecretStoreError> {
        let store = Self {
            path: path.as_ref().to_path_buf(),
            key_provider,
        };
        let connection = store.connection()?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS encrypted_credentials (
                credential_ref TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                environment TEXT NOT NULL,
                algorithm TEXT NOT NULL,
                key_version INTEGER NOT NULL,
                data_nonce BLOB NOT NULL,
                wrapped_dek BLOB NOT NULL,
                wrap_nonce BLOB NOT NULL,
                ciphertext BLOB NOT NULL,
                fingerprint TEXT NOT NULL,
                created_at_unix_ms INTEGER NOT NULL,
                rotated_at_unix_ms INTEGER,
                disabled_at_unix_ms INTEGER
            ) STRICT;",
        )?;
        Ok(store)
    }

    fn connection(&self) -> Result<Connection, SecretStoreError> {
        let connection = Connection::open(&self.path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(connection)
    }

    fn encrypt(
        &self,
        credential_ref: &CredentialRef,
        context: &CredentialContext,
        secret: &SecretBytes,
    ) -> Result<EncryptedEnvelope, SecretStoreError> {
        let key_version = self.key_provider.active_key_version()?;
        let kek = self.key_provider.resolve_key_material(key_version)?;
        let random = SystemRandom::new();
        let mut dek = Zeroizing::new([0_u8; KEY_LEN]);
        random
            .fill(dek.as_mut())
            .map_err(|_| SecretStoreError::CryptographicFailure)?;
        let data_cipher = LessSafeKey::new(
            UnboundKey::new(&aead::AES_256_GCM, dek.as_ref())
                .map_err(|_| SecretStoreError::CryptographicFailure)?,
        );
        let mut data_nonce = [0_u8; NONCE_LEN];
        random
            .fill(&mut data_nonce)
            .map_err(|_| SecretStoreError::CryptographicFailure)?;
        let aad = associated_data(credential_ref, context, key_version);
        let mut ciphertext = secret.expose_secret().to_vec();
        data_cipher
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(data_nonce),
                Aad::from(payload_aad(&aad).as_bytes()),
                &mut ciphertext,
            )
            .map_err(|_| SecretStoreError::CryptographicFailure)?;

        let wrap_cipher = LessSafeKey::new(
            UnboundKey::new(&aead::AES_256_GCM, kek.expose())
                .map_err(|_| SecretStoreError::CryptographicFailure)?,
        );
        let mut wrap_nonce = [0_u8; NONCE_LEN];
        random
            .fill(&mut wrap_nonce)
            .map_err(|_| SecretStoreError::CryptographicFailure)?;
        let mut wrapped_dek = dek.to_vec();
        wrap_cipher
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(wrap_nonce),
                Aad::from(dek_aad(&aad).as_bytes()),
                &mut wrapped_dek,
            )
            .map_err(|_| SecretStoreError::CryptographicFailure)?;
        Ok(EncryptedEnvelope {
            key_version,
            data_nonce,
            wrapped_dek,
            wrap_nonce,
            ciphertext,
        })
    }
}

struct EncryptedEnvelope {
    key_version: u32,
    data_nonce: [u8; NONCE_LEN],
    wrapped_dek: Vec<u8>,
    wrap_nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
}

impl<K: KeyProvider> SecretStore for SqliteSecretStore<K> {
    fn put(
        &self,
        credential_ref: &CredentialRef,
        context: &CredentialContext,
        secret: SecretBytes,
        now_unix_ms: i64,
    ) -> Result<String, SecretStoreError> {
        let fingerprint = fingerprint(&secret);
        let envelope = self.encrypt(credential_ref, context, &secret)?;
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO encrypted_credentials (
                    credential_ref, provider, environment, algorithm, key_version, data_nonce,
                    wrapped_dek, wrap_nonce, ciphertext, fingerprint, created_at_unix_ms,
                    rotated_at_unix_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)",
                params![
                    credential_ref.as_str(),
                    context.provider.as_str(),
                    environment_text(context.environment),
                    ALGORITHM,
                    envelope.key_version,
                    envelope.data_nonce.as_slice(),
                    envelope.wrapped_dek,
                    envelope.wrap_nonce.as_slice(),
                    envelope.ciphertext,
                    fingerprint,
                    now_unix_ms,
                ],
            )
            .map_err(|error| match error {
                rusqlite::Error::SqliteFailure(code, _)
                    if code.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    SecretStoreError::AlreadyExists
                }
                other => SecretStoreError::Persistence(other.to_string()),
            })?;
        Ok(fingerprint)
    }

    fn get(
        &self,
        credential_ref: &CredentialRef,
        expected_context: &CredentialContext,
    ) -> Result<SecretBytes, SecretStoreError> {
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT provider, environment, algorithm, key_version, data_nonce, ciphertext,
                        wrapped_dek, wrap_nonce,
                        disabled_at_unix_ms
                 FROM encrypted_credentials WHERE credential_ref = ?1",
                [credential_ref.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, u32>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                        row.get::<_, Vec<u8>>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                    ))
                },
            )
            .optional()?
            .ok_or(SecretStoreError::NotFound)?;
        if row.8.is_some() {
            return Err(SecretStoreError::Disabled);
        }
        if row.0 != expected_context.provider.as_str()
            || row.1 != environment_text(expected_context.environment)
        {
            return Err(SecretStoreError::ContextMismatch);
        }
        if row.2 != ALGORITHM || row.4.len() != NONCE_LEN || row.7.len() != NONCE_LEN {
            return Err(SecretStoreError::UnsupportedEnvelope);
        }
        let kek = self.key_provider.resolve_key_material(row.3)?;
        let wrap_cipher = LessSafeKey::new(
            UnboundKey::new(&aead::AES_256_GCM, kek.expose())
                .map_err(|_| SecretStoreError::CryptographicFailure)?,
        );
        let aad = associated_data(credential_ref, expected_context, row.3);
        let wrap_nonce: [u8; NONCE_LEN] = row
            .7
            .as_slice()
            .try_into()
            .map_err(|_| SecretStoreError::UnsupportedEnvelope)?;
        let mut dek = Zeroizing::new(row.6);
        let dek_len = wrap_cipher
            .open_in_place(
                Nonce::assume_unique_for_key(wrap_nonce),
                Aad::from(dek_aad(&aad).as_bytes()),
                dek.as_mut(),
            )
            .map_err(|_| SecretStoreError::AuthenticationFailed)?
            .len();
        dek.truncate(dek_len);
        if dek.len() != KEY_LEN {
            return Err(SecretStoreError::UnsupportedEnvelope);
        }
        let data_cipher = LessSafeKey::new(
            UnboundKey::new(&aead::AES_256_GCM, dek.as_ref())
                .map_err(|_| SecretStoreError::CryptographicFailure)?,
        );
        let data_nonce: [u8; NONCE_LEN] = row
            .4
            .as_slice()
            .try_into()
            .map_err(|_| SecretStoreError::UnsupportedEnvelope)?;
        let mut plaintext = Zeroizing::new(row.5);
        let plaintext_len = data_cipher
            .open_in_place(
                Nonce::assume_unique_for_key(data_nonce),
                Aad::from(payload_aad(&aad).as_bytes()),
                plaintext.as_mut(),
            )
            .map_err(|_| SecretStoreError::AuthenticationFailed)?
            .len();
        plaintext.truncate(plaintext_len);
        SecretBytes::new(std::mem::take(&mut *plaintext))
    }

    fn rotate(
        &self,
        credential_ref: &CredentialRef,
        expected_context: &CredentialContext,
        secret: SecretBytes,
        now_unix_ms: i64,
    ) -> Result<String, SecretStoreError> {
        // Validate context and disabled state before replacing capital-affecting credential.
        drop(self.get(credential_ref, expected_context)?);
        let fingerprint = fingerprint(&secret);
        let envelope = self.encrypt(credential_ref, expected_context, &secret)?;
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE encrypted_credentials SET algorithm = ?1, key_version = ?2, data_nonce = ?3,
             wrapped_dek = ?4, wrap_nonce = ?5, ciphertext = ?6, fingerprint = ?7,
             rotated_at_unix_ms = ?8
             WHERE credential_ref = ?9 AND provider = ?10 AND environment = ?11
             AND disabled_at_unix_ms IS NULL",
            params![
                ALGORITHM,
                envelope.key_version,
                envelope.data_nonce.as_slice(),
                envelope.wrapped_dek,
                envelope.wrap_nonce.as_slice(),
                envelope.ciphertext,
                fingerprint,
                now_unix_ms,
                credential_ref.as_str(),
                expected_context.provider.as_str(),
                environment_text(expected_context.environment),
            ],
        )?;
        if changed != 1 {
            return Err(SecretStoreError::NotFoundOrContextMismatch);
        }
        Ok(fingerprint)
    }

    fn rewrap(
        &self,
        credential_ref: &CredentialRef,
        expected_context: &CredentialContext,
        now_unix_ms: i64,
    ) -> Result<(), SecretStoreError> {
        let secret = self.get(credential_ref, expected_context)?;
        self.rotate(credential_ref, expected_context, secret, now_unix_ms)?;
        Ok(())
    }

    fn disable(&self, credential_ref: &CredentialRef) -> Result<(), SecretStoreError> {
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE encrypted_credentials SET disabled_at_unix_ms =
             COALESCE(disabled_at_unix_ms, CAST(strftime('%s', 'now') AS INTEGER) * 1000)
             WHERE credential_ref = ?1",
            [credential_ref.as_str()],
        )?;
        if changed != 1 {
            return Err(SecretStoreError::NotFound);
        }
        Ok(())
    }

    fn delete(&self, credential_ref: &CredentialRef) -> Result<(), SecretStoreError> {
        let connection = self.connection()?;
        connection.execute(
            "DELETE FROM encrypted_credentials WHERE credential_ref = ?1",
            [credential_ref.as_str()],
        )?;
        Ok(())
    }
}

fn environment_text(environment: BrokerEnvironment) -> &'static str {
    match environment {
        BrokerEnvironment::Production => "PRODUCTION",
        BrokerEnvironment::Sandbox => "SANDBOX",
    }
}

fn associated_data(
    credential_ref: &CredentialRef,
    context: &CredentialContext,
    key_version: u32,
) -> String {
    format!(
        "vox-credential-v1|{}|{}|{}|{}|{}",
        credential_ref.as_str(),
        context.provider.as_str(),
        environment_text(context.environment),
        ALGORITHM,
        key_version
    )
}

fn payload_aad(envelope_aad: &str) -> String {
    format!("{envelope_aad}|PAYLOAD")
}

fn dek_aad(envelope_aad: &str) -> String {
    format!("{envelope_aad}|DEK")
}

fn fingerprint(secret: &SecretBytes) -> String {
    let digest = digest::digest(&digest::SHA256, secret.expose_secret());
    digest.as_ref()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum KeyProviderError {
    #[error("key version {0} unavailable")]
    MissingVersion(u32),
    #[error("external key unavailable")]
    MissingExternalKey,
    #[error("external key must be a 32-byte hexadecimal value")]
    InvalidExternalKey,
    #[error("key version must increase monotonically and remain unique")]
    NonMonotonicVersion,
    #[error("key provider unavailable")]
    Unavailable,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SecretStoreError {
    #[error("secret must not be empty")]
    EmptySecret,
    #[error("secret exceeds maximum size")]
    SecretTooLarge,
    #[error("credential already exists")]
    AlreadyExists,
    #[error("credential not found")]
    NotFound,
    #[error("credential context mismatch")]
    ContextMismatch,
    #[error("credential not found or context mismatch")]
    NotFoundOrContextMismatch,
    #[error("credential disabled")]
    Disabled,
    #[error("unsupported credential envelope")]
    UnsupportedEnvelope,
    #[error("credential envelope authentication failed")]
    AuthenticationFailed,
    #[error("credential cryptographic operation failed")]
    CryptographicFailure,
    #[error("credential persistence failed: {0}")]
    Persistence(String),
    #[error(transparent)]
    KeyProvider(#[from] KeyProviderError),
}

impl From<rusqlite::Error> for SecretStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Persistence(error.to_string())
    }
}
