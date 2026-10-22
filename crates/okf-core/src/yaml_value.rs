use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;

/// Deserialize YAML into JSON values without interpreting JSON's internal number
/// marker as a producer-defined mapping. Large YAML integers retain their value.
pub(crate) struct YamlValue(pub Value);

impl<'de> Deserialize<'de> for YamlValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct YamlVisitor;
        impl<'de> Visitor<'de> for YamlVisitor {
            type Value = YamlValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a YAML value")
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(YamlValue(Value::Null))
            }
            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(YamlValue(Value::Bool(value)))
            }
            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(YamlValue(Value::Number(value.into())))
            }
            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(YamlValue(Value::Number(value.into())))
            }
            fn visit_i128<E: de::Error>(self, value: i128) -> Result<Self::Value, E> {
                Number::from_i128(value)
                    .map(|number| YamlValue(Value::Number(number)))
                    .ok_or_else(|| E::custom("integer out of range"))
            }
            fn visit_u128<E: de::Error>(self, value: u128) -> Result<Self::Value, E> {
                Number::from_u128(value)
                    .map(|number| YamlValue(Value::Number(number)))
                    .ok_or_else(|| E::custom("integer out of range"))
            }
            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
                Ok(YamlValue(
                    Number::from_f64(value).map_or(Value::Null, Value::Number),
                ))
            }
            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(YamlValue(Value::String(value.into())))
            }
            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(YamlValue(Value::String(value)))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = vec![];
                while let Some(YamlValue(value)) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(YamlValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut mapping: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some((key, YamlValue(value))) =
                    mapping.next_entry::<String, YamlValue>()?
                {
                    values.insert(key, value);
                }
                Ok(YamlValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(YamlVisitor)
    }
}
