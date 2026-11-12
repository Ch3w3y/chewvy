//! XDG Secret Service backend for [`TokenStore`] (zbus 5.19.0).
//!
//! The account token lives in the desktop secrets daemon (KDE Wallet / GNOME
//! Keyring) as one item per [`TokenKey`] in the **default** collection, found
//! again by its attributes, the way libsecret clients do. The pre-keying item
//! (this client's `application` attribute but no `account` attribute) is kept
//! readable so an existing single Plex token migrates without a re-login.
//!
//! Verification status: compile-verified in the offline worker; the D-Bus call
//! shapes follow the freedesktop Secret Service API and were checked against a
//! live KDE `ksecretd`. A locked collection that needs a prompt is reported as
//! [`StorageError::Locked`]; prompts are not driven here.

use std::collections::HashMap;

use crate::client_info::is_uuid;
use crate::error::{CoreError, StorageError};
use crate::secret::SecretString;
use crate::tokens::{Backend, TokenKey, TokenStore};

/// XDG Secret Service bus name.
pub const SECRET_SERVICE_DEST: &str = "org.freedesktop.secrets";
/// XDG Secret Service root object path.
pub const SECRET_SERVICE_PATH: &str = "/org/freedesktop/secrets";
/// `org.freedesktop.Secret.Service` interface.
pub const SECRET_IFACE_SERVICE: &str = "org.freedesktop.Secret.Service";
/// `org.freedesktop.Secret.Collection` interface.
pub const SECRET_IFACE_COLLECTION: &str = "org.freedesktop.Secret.Collection";
/// `org.freedesktop.Secret.Item` interface.
pub const SECRET_IFACE_ITEM: &str = "org.freedesktop.Secret.Item";
/// The collection alias the token is stored in (the user's default keyring).
pub const SECRET_COLLECTION_ALIAS: &str = "default";

/// Marker attribute written into Secret Service items so this client can find
/// its own items again. Not a secret.
pub const SECRET_ATTRIBUTES_KEY: &str = "application";

/// Value of [`SECRET_ATTRIBUTES_KEY`] used by this client.
pub const SECRET_ATTRIBUTES_VALUE: &str = "chewvy";

/// The value written before the app was named Chewvy. Still searched, so a
/// sign-in stored by an earlier build is found and can be deleted.
const FORMER_ATTRIBUTES_VALUE: &str = "plex-rust-client";

/// Attribute separating one [`TokenKey`]'s item from another's. Absent on the
/// pre-keying item, which is how the legacy slot is recognised.
pub const SECRET_ACCOUNT_ATTRIBUTE: &str = "account";

/// Label of our Secret Service item (visible in KDE Wallet / seahorse UIs).
pub const SECRET_ITEM_LABEL: &str = "Chewvy account token";

/// Sanity-check a client identifier before it is used as a Secret Service
/// attribute value (attributes are searchable by other applications).
#[must_use]
pub fn attribute_safe(client_identifier: &str) -> bool {
    is_uuid(client_identifier)
}

type ObjPath = zbus::zvariant::OwnedObjectPath;
/// `(session, parameters, value, content_type)`: the spec's `Secret` struct.
type SecretWire = (ObjPath, Vec<u8>, Vec<u8>, String);

/// The primary backend: the account token lives in the desktop secrets daemon
/// (KDE Wallet / GNOME Keyring), keyed by [`TokenKey`] attributes.
pub struct SecretServiceStore {
    connection: zbus::blocking::Connection,
}

impl SecretServiceStore {
    /// Connect to the session bus and confirm a secrets daemon answers.
    /// Side-effect free: lists bus names only, never creates or unlocks.
    pub fn probe() -> Result<Self, CoreError> {
        let connection = zbus::blocking::Connection::session()
            .map_err(|_| CoreError::SecretServiceUnavailable)?;
        let bus = zbus::blocking::Proxy::new_owned(
            connection.clone(),
            String::from("org.freedesktop.DBus"),
            String::from("/org/freedesktop/DBus"),
            String::from("org.freedesktop.DBus"),
        )
        .map_err(|_| CoreError::SecretServiceUnavailable)?;
        let names: Vec<String> = bus
            .call("ListNames", &())
            .map_err(|_| CoreError::SecretServiceUnavailable)?;
        if !names.iter().any(|name| name == SECRET_SERVICE_DEST) {
            return Err(CoreError::SecretServiceUnavailable);
        }
        Ok(Self { connection })
    }

    fn proxy(
        &self,
        path: &str,
        interface: &str,
    ) -> Result<zbus::blocking::Proxy<'static>, CoreError> {
        zbus::blocking::Proxy::new_owned(
            self.connection.clone(),
            SECRET_SERVICE_DEST.to_owned(),
            path.to_owned(),
            interface.to_owned(),
        )
        .map_err(|_| CoreError::SecretServiceUnavailable)
    }

    fn service(&self) -> Result<zbus::blocking::Proxy<'static>, CoreError> {
        self.proxy(SECRET_SERVICE_PATH, SECRET_IFACE_SERVICE)
    }

    /// `OpenSession(s algorithm, v input) -> (v output, o session)`, plain.
    fn open_session(&self) -> Result<ObjPath, CoreError> {
        let input = zbus::zvariant::Value::from("");
        self.service()?
            .call::<_, _, (zbus::zvariant::OwnedValue, ObjPath)>("OpenSession", &("plain", input))
            .map(|(_, session)| session)
            .map_err(map_secret_error)
    }

    /// `ReadAlias("default")`; `/` means no default collection exists.
    fn default_collection(&self) -> Result<ObjPath, CoreError> {
        let path: ObjPath = self
            .service()?
            .call("ReadAlias", &(SECRET_COLLECTION_ALIAS,))
            .map_err(map_secret_error)?;
        if path.as_str() == "/" {
            return Err(CoreError::Storage(StorageError::NotFound));
        }
        Ok(path)
    }

    /// `Unlock(ao) -> (ao unlocked, o prompt)`. A required prompt is `Locked`.
    fn unlock(&self, objects: Vec<ObjPath>) -> Result<(), CoreError> {
        let (_, prompt): (Vec<ObjPath>, ObjPath) = self
            .service()?
            .call("Unlock", &(objects,))
            .map_err(map_secret_error)?;
        if prompt.as_str() == "/" {
            Ok(())
        } else {
            Err(CoreError::Storage(StorageError::Locked))
        }
    }

    fn item_attributes(application: &str, key: Option<&TokenKey>) -> HashMap<String, String> {
        let mut attributes = HashMap::new();
        attributes.insert(SECRET_ATTRIBUTES_KEY.to_owned(), application.to_owned());
        if let Some(key) = key {
            attributes.insert(SECRET_ACCOUNT_ATTRIBUTE.to_owned(), key.as_str().to_owned());
        }
        attributes
    }

    /// `SearchItems(a{ss}) -> (ao unlocked, ao locked)`; locked hits are unlocked.
    fn find_items(&self, attributes: HashMap<String, String>) -> Result<Vec<ObjPath>, CoreError> {
        let (mut unlocked, locked): (Vec<ObjPath>, Vec<ObjPath>) = self
            .service()?
            .call("SearchItems", &(attributes,))
            .map_err(map_secret_error)?;
        if !locked.is_empty() {
            self.unlock(locked.clone())?;
            unlocked.extend(locked);
        }
        Ok(unlocked)
    }

    /// Every item stored for `key`, under the current marker first and then
    /// under the pre-Chewvy one.
    fn find_all(&self, key: Option<&TokenKey>) -> Result<Vec<ObjPath>, CoreError> {
        let mut items = self.find_items(Self::item_attributes(SECRET_ATTRIBUTES_VALUE, key))?;
        items.extend(self.find_items(Self::item_attributes(FORMER_ATTRIBUTES_VALUE, key))?);
        Ok(items)
    }

    fn find_item(&self, key: Option<&TokenKey>) -> Result<Option<ObjPath>, CoreError> {
        Ok(self.find_all(key)?.into_iter().next())
    }

    /// The pre-keying item: this client's `application` attribute, but written
    /// before the `account` attribute existed. Attribute search cannot express
    /// "absence", so the candidates are inspected one by one.
    fn find_legacy_item(&self) -> Result<Option<ObjPath>, CoreError> {
        for path in self.find_all(None)? {
            let item = self.proxy(path.as_str(), SECRET_IFACE_ITEM)?;
            let attributes: HashMap<String, String> = item
                .get_property("org.freedesktop.Secret.Item.Attributes")
                .map_err(map_secret_error)?;
            if !attributes.contains_key(SECRET_ACCOUNT_ATTRIBUTE) {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    fn read_secret(&self, path: &ObjPath) -> Result<Option<SecretString>, CoreError> {
        let session = self.open_session()?;
        let item = self.proxy(path.as_str(), SECRET_IFACE_ITEM)?;
        let (_, _, value, _): SecretWire = item
            .call("GetSecret", &(session,))
            .map_err(map_secret_error)?;
        if value.is_empty() {
            return Ok(None);
        }
        let text =
            String::from_utf8(value).map_err(|_| CoreError::Storage(StorageError::Payload))?;
        Ok(Some(SecretString::new(text)))
    }

    fn create_item(&self, key: Option<&TokenKey>, token: &SecretString) -> Result<(), CoreError> {
        let session = self.open_session()?;
        let collection_path = self.default_collection()?;
        self.unlock(vec![collection_path.clone()])?;
        let collection = self.proxy(collection_path.as_str(), SECRET_IFACE_COLLECTION)?;
        let secret: SecretWire = (
            session,
            Vec::new(),
            token.expose().as_bytes().to_vec(),
            String::from("text/plain"),
        );
        let mut properties: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        properties.insert(
            "org.freedesktop.Secret.Item.Label",
            zbus::zvariant::Value::from(SECRET_ITEM_LABEL),
        );
        properties.insert(
            "org.freedesktop.Secret.Item.Attributes",
            zbus::zvariant::Value::from(Self::item_attributes(SECRET_ATTRIBUTES_VALUE, key)),
        );
        // CreateItem(a{sv} properties, (oayays) secret, b replace) -> (o item, o prompt)
        let (_, prompt): (ObjPath, ObjPath) = collection
            .call("CreateItem", &(properties, secret, true))
            .map_err(map_secret_error)?;
        if prompt.as_str() != "/" {
            return Err(CoreError::Storage(StorageError::Locked));
        }
        Ok(())
    }

    fn delete_item(&self, path: &ObjPath) -> Result<(), CoreError> {
        let _prompt: ObjPath = self
            .proxy(path.as_str(), SECRET_IFACE_ITEM)?
            .call("Delete", &())
            .map_err(map_secret_error)?;
        Ok(())
    }
}

impl TokenStore for SecretServiceStore {
    fn backend(&self) -> Backend {
        Backend::SecretService
    }

    fn store(&self, key: &TokenKey, token: &SecretString) -> Result<(), CoreError> {
        if token.is_empty() {
            return Err(CoreError::InvalidInput);
        }
        self.create_item(Some(key), token)
    }

    fn load(&self, key: &TokenKey) -> Result<Option<SecretString>, CoreError> {
        match self.find_item(Some(key))? {
            Some(path) => self.read_secret(&path),
            None => Ok(None),
        }
    }

    fn delete(&self, key: &TokenKey) -> Result<(), CoreError> {
        // Both markers: a token left under the old one would sign back in.
        self.find_all(Some(key))?
            .iter()
            .try_for_each(|path| self.delete_item(path))
    }

    fn load_legacy(&self) -> Result<Option<SecretString>, CoreError> {
        match self.find_legacy_item()? {
            Some(path) => self.read_secret(&path),
            None => Ok(None),
        }
    }

    fn store_legacy(&self, token: &SecretString) -> Result<(), CoreError> {
        if token.is_empty() {
            return Err(CoreError::InvalidInput);
        }
        self.create_item(None, token)
    }

    fn delete_legacy(&self) -> Result<(), CoreError> {
        match self.find_legacy_item()? {
            Some(path) => self.delete_item(&path),
            None => Ok(()),
        }
    }
}

impl std::fmt::Debug for SecretServiceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretServiceStore").finish_non_exhaustive()
    }
}

/// Map a zbus/secret-service failure onto a token-free core error.
///
/// A "locked collection"/prompt rejection is `Storage(Locked)`; everything else
/// is `SecretServiceUnavailable` (which `open_store` uses to reach the consent
/// gate). zbus error text can name bus objects, never our token.
fn map_secret_error(error: zbus::Error) -> CoreError {
    let text = error.to_string();
    if text.contains("IsLocked") || text.contains("Denied") || text.contains("Cancelled") {
        return CoreError::Storage(StorageError::Locked);
    }
    CoreError::SecretServiceUnavailable
}
