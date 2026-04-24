use serde::{Serialize, Serializer, ser::SerializeStruct};

pub struct SchemaField {
    pub name: &'static str,
    pub kind: FieldKind,
    pub required: bool,
}

pub enum FieldKind {
    Text,
    Date,
    Select(&'static [&'static str]),
    #[allow(dead_code)]
    MultiSelect(&'static [&'static str]),
    Tags,
    Relation,
}

impl FieldKind {
    fn kind_name(&self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Date => "date",
            FieldKind::Select(_) => "select",
            FieldKind::MultiSelect(_) => "multi_select",
            FieldKind::Tags => "tags",
            FieldKind::Relation => "relation",
        }
    }

    fn options(&self) -> Option<&'static [&'static str]> {
        match self {
            FieldKind::Select(opts) | FieldKind::MultiSelect(opts) => Some(opts),
            _ => None,
        }
    }
}

pub const THOUGHT_SCHEMA: &[SchemaField] = &[
    SchemaField {
        name: "title",
        kind: FieldKind::Text,
        required: true,
    },
    SchemaField {
        name: "type",
        kind: FieldKind::Select(&["plan", "research", "handoff", "note"]),
        required: true,
    },
    SchemaField {
        name: "date",
        kind: FieldKind::Date,
        required: true,
    },
    SchemaField {
        name: "status",
        kind: FieldKind::Select(&["draft", "active", "implemented", "superseded", "archived"]),
        required: true,
    },
    SchemaField {
        name: "ticket",
        kind: FieldKind::Text,
        required: false,
    },
    SchemaField {
        name: "project",
        kind: FieldKind::Text,
        required: true,
    },
    SchemaField {
        name: "scope",
        kind: FieldKind::Select(&["user", "shared", "global"]),
        required: true,
    },
    SchemaField {
        name: "tags",
        kind: FieldKind::Tags,
        required: false,
    },
    SchemaField {
        name: "author",
        kind: FieldKind::Text,
        required: true,
    },
    SchemaField {
        name: "related",
        kind: FieldKind::Relation,
        required: false,
    },
];

impl Serialize for SchemaField {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let options = self.kind.options();
        let field_count = if options.is_some() { 4 } else { 3 };
        let mut state = serializer.serialize_struct("SchemaField", field_count)?;
        state.serialize_field("name", self.name)?;
        state.serialize_field("kind", self.kind.kind_name())?;
        if let Some(opts) = options {
            state.serialize_field("options", opts)?;
        }
        state.serialize_field("required", &self.required)?;
        state.end()
    }
}

/// Serialize `THOUGHT_SCHEMA` as a JSON array suitable for `storage info --json`.
///
/// `storage info --json` is called by every write-oriented slash command's
/// dispatch preamble, so this runs on the hot path of every agent interaction.
/// The schema is `static` and the serialized form is identical on every call;
/// compute once and clone.
pub fn schema_as_json_value() -> serde_json::Value {
    static CACHED: std::sync::LazyLock<serde_json::Value> = std::sync::LazyLock::new(|| {
        serde_json::to_value(THOUGHT_SCHEMA).expect("schema serialization is infallible")
    });
    CACHED.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_ten_fields_in_documented_order() {
        let names: Vec<&str> = THOUGHT_SCHEMA.iter().map(|f| f.name).collect();
        assert_eq!(
            names,
            vec![
                "title", "type", "date", "status", "ticket", "project", "scope", "tags", "author",
                "related",
            ]
        );
        assert_eq!(THOUGHT_SCHEMA.len(), 10);
    }

    #[test]
    fn required_fields_are_correctly_marked() {
        let required: Vec<&str> = THOUGHT_SCHEMA
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(
            required,
            vec![
                "title", "type", "date", "status", "project", "scope", "author"
            ]
        );
    }

    #[test]
    fn select_fields_have_options() {
        for field in THOUGHT_SCHEMA {
            if matches!(field.kind, FieldKind::Select(_) | FieldKind::MultiSelect(_)) {
                assert!(field.kind.options().is_some(), "{}", field.name);
            }
        }
    }

    #[test]
    fn type_field_options_match_spec() {
        let type_field = THOUGHT_SCHEMA
            .iter()
            .find(|f| f.name == "type")
            .expect("type field exists");
        assert_eq!(
            type_field.kind.options(),
            Some(&["plan", "research", "handoff", "note"][..])
        );
    }

    #[test]
    fn status_field_options_match_spec() {
        let field = THOUGHT_SCHEMA
            .iter()
            .find(|f| f.name == "status")
            .expect("status field exists");
        assert_eq!(
            field.kind.options(),
            Some(&["draft", "active", "implemented", "superseded", "archived",][..])
        );
    }

    #[test]
    fn scope_field_options_match_spec() {
        let field = THOUGHT_SCHEMA
            .iter()
            .find(|f| f.name == "scope")
            .expect("scope field exists");
        assert_eq!(
            field.kind.options(),
            Some(&["user", "shared", "global"][..])
        );
    }

    #[test]
    fn serializes_text_field_without_options() {
        let field = &THOUGHT_SCHEMA[0]; // title
        let json = serde_json::to_value(field).unwrap();
        assert_eq!(json["name"], "title");
        assert_eq!(json["kind"], "text");
        assert_eq!(json["required"], true);
        assert!(json.get("options").is_none());
    }

    #[test]
    fn serializes_select_field_with_options() {
        let field = &THOUGHT_SCHEMA[1]; // type
        let json = serde_json::to_value(field).unwrap();
        assert_eq!(json["name"], "type");
        assert_eq!(json["kind"], "select");
        assert_eq!(
            json["options"],
            serde_json::json!(["plan", "research", "handoff", "note"])
        );
        assert_eq!(json["required"], true);
    }

    #[test]
    fn full_schema_serializes_to_array() {
        let value = schema_as_json_value();
        assert!(value.is_array());
        assert_eq!(value.as_array().unwrap().len(), 10);
    }
}
