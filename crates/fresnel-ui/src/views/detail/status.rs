//! The field `status` of an object as JSON, with no other field.
//!
//! The panel reads the conditions of each kind from the JSON of its status, for each new
//! object and version. The JSON of the whole object takes time for each byte, and that time
//! is in a frame: for a ConfigMap of 70,000 keys, 9 ms on the Mac of the 16 ms limit and 16
//! to 26 ms on a CI runner. Each object type of `k8s-openapi` writes itself as a struct, so
//! this serializer writes its field `status` and skips each other field with no work.

use std::fmt::{self, Display};

use serde::Serialize;
use serde::ser::{self, Impossible, SerializeStruct, Serializer};
use serde_json::Value;

/// The JSON of the field `status` of an object. `None` for an object with no status, and
/// for a value that is not a struct.
pub(super) fn of<K: Serialize>(object: &K) -> Option<Value> {
    object.serialize(Object).ok().flatten()
}

/// The serializer of an object. It accepts a struct only.
struct Object;

/// The fields of the struct: it keeps the JSON of `status`.
struct Fields(Option<Value>);

/// A value that is not a struct, or a status that has no JSON form.
#[derive(Debug)]
struct NotAnObject;

impl Display for NotAnObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not an object with a JSON status")
    }
}

impl std::error::Error for NotAnObject {}

impl ser::Error for NotAnObject {
    fn custom<T: Display>(_message: T) -> Self {
        Self
    }
}

impl SerializeStruct for Fields {
    type Ok = Option<Value>;
    type Error = NotAnObject;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, key: &'static str, value: &T) -> Result<(), NotAnObject> {
        if key == "status" {
            self.0 = Some(serde_json::to_value(value).map_err(|_| NotAnObject)?);
        }
        Ok(())
    }

    fn end(self) -> Result<Option<Value>, NotAnObject> {
        Ok(self.0)
    }
}

/// The methods of each form of a value that is not a struct: each one refuses it.
macro_rules! refuse {
    ($($method:ident($($argument:ty),*) -> $result:ty;)*) => {
        $(fn $method(self, $(_: $argument),*) -> Result<$result, NotAnObject> {
            Err(NotAnObject)
        })*
    };
}

type Refused = Impossible<Option<Value>, NotAnObject>;

impl Serializer for Object {
    type Ok = Option<Value>;
    type Error = NotAnObject;
    type SerializeSeq = Refused;
    type SerializeTuple = Refused;
    type SerializeTupleStruct = Refused;
    type SerializeTupleVariant = Refused;
    type SerializeMap = Refused;
    type SerializeStruct = Fields;
    type SerializeStructVariant = Refused;

    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Fields, NotAnObject> {
        Ok(Fields(None))
    }

    refuse! {
        serialize_bool(bool) -> Self::Ok;
        serialize_i8(i8) -> Self::Ok;
        serialize_i16(i16) -> Self::Ok;
        serialize_i32(i32) -> Self::Ok;
        serialize_i64(i64) -> Self::Ok;
        serialize_u8(u8) -> Self::Ok;
        serialize_u16(u16) -> Self::Ok;
        serialize_u32(u32) -> Self::Ok;
        serialize_u64(u64) -> Self::Ok;
        serialize_f32(f32) -> Self::Ok;
        serialize_f64(f64) -> Self::Ok;
        serialize_char(char) -> Self::Ok;
        serialize_str(&str) -> Self::Ok;
        serialize_bytes(&[u8]) -> Self::Ok;
        serialize_none() -> Self::Ok;
        serialize_unit() -> Self::Ok;
        serialize_unit_struct(&'static str) -> Self::Ok;
        serialize_unit_variant(&'static str, u32, &'static str) -> Self::Ok;
        serialize_seq(Option<usize>) -> Refused;
        serialize_tuple(usize) -> Refused;
        serialize_tuple_struct(&'static str, usize) -> Refused;
        serialize_tuple_variant(&'static str, u32, &'static str, usize) -> Refused;
        serialize_map(Option<usize>) -> Refused;
        serialize_struct_variant(&'static str, u32, &'static str, usize) -> Refused;
    }

    fn serialize_some<T: ?Sized + Serialize>(self, _value: &T) -> Result<Self::Ok, NotAnObject> {
        Err(NotAnObject)
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, NotAnObject> {
        Err(NotAnObject)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<Self::Ok, NotAnObject> {
        Err(NotAnObject)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use k8s_openapi::api::core::v1::ConfigMap;
    use serde_json::json;

    use super::*;

    /// A field that counts how often it is written.
    struct Counted<'a>(&'a Cell<usize>);

    impl Serialize for Counted<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.0.set(self.0.get() + 1);
            serializer.serialize_unit()
        }
    }

    #[derive(Serialize)]
    struct Fake<'a> {
        data: Counted<'a>,
        status: Value,
    }

    #[test]
    fn the_status_is_written_and_no_other_field() {
        let writes = Cell::new(0);
        let status = json!({ "conditions": [{ "type": "Ready", "status": "True" }] });
        assert_eq!(of(&Fake { data: Counted(&writes), status: status.clone() }), Some(status));
        assert_eq!(writes.get(), 0, "the data of the object costs no time");
        assert_eq!(of(&ConfigMap::default()), None, "an object with no status");
        assert_eq!(of(&"text"), None, "a value that is not an object");
    }
}
