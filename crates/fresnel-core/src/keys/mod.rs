//! The keys of a Secret or a ConfigMap: the data of their views (plan section 12.2).
//!
//! No store keeps a value of a Secret (plan section 9.4). A key view reads its one object
//! from the server while it is open, as a tier C object: a [`KeyObject`] has each value,
//! decoded as text when it is text, and the version of the read. An edit is a [`Draft`] on
//! that read. Its [`Save`] is a merge patch of the changed keys with that version, so each
//! other key stays equal byte for byte: the app never encodes a value that did not change.
//!
//! - `value`: the decode of one value. `name`: the rule for the name of a key.
//! - `object`: the object of a key view. `draft`: add, remove and change a key, the save,
//!   and the conflicts with a new read.
//! - `request`: the requests. `remote`: the calls through the gateway, and [`WriteState`].
//! - `secret_type`: the types of a Secret and a [`NewSecret`]. `special`, `tls` and `pull`:
//!   the certificates of a TLS Secret and the registries of an image pull Secret.

mod draft;
mod name;
mod object;
mod pull;
mod remote;
mod request;
mod secret_type;
mod special;
mod tls;
mod value;

pub use draft::{Change, Conflict, Draft, DraftKey, KeyState, KeyWrite, Refusal, Save};
pub use object::{Entry, KeyKind, KeyObject, Place, Target};
pub use pull::Registry;
pub use remote::{WriteState, create, read, save};
/// The operations send the same name, so `managedFields` shows one manager for the app.
pub(crate) use request::FIELD_MANAGER;
pub use secret_type::{NewSecret, SERVICE_ACCOUNT_ANNOTATION, SecretType};
pub use special::Special;
pub use tls::Certificate;
pub use value::{BinaryType, Form, Value};
