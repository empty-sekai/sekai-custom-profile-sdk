//! Lossless authoring boundary for the public custom-profile document envelope.

use serde_json::{Map, Value};
use thiserror::Error;

const CUSTOM_PROFILE_CARDS_KEY: &str = "userCustomProfileCards";
const CARD_ARRAYS: [&str; 12] = [
    "bondsHonors",
    "cardMembers",
    "collections",
    "generalBackgrounds",
    "generals",
    "honors",
    "others",
    "shapes",
    "stamps",
    "standMembers",
    "storyBackgrounds",
    "texts",
];

#[derive(Debug, Clone, PartialEq)]
pub struct GameProfileDocument {
    export: Value,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GameProfileDocumentError {
    #[error("profile must be a JSON object")]
    ProfileMustBeObject,
    #[error("profile is missing userCustomProfileCards")]
    MissingCustomProfileCards,
    #[error("userCustomProfileCards must be an array")]
    CustomProfileCardsMustBeArray,
    #[error("userCustomProfileCards[{page}] must be an object")]
    PageMustBeObject { page: usize },
    #[error("userCustomProfileCards[{page}].{field} must be an integer")]
    PageIdentityMustBeInteger { page: usize, field: &'static str },
    #[error("userCustomProfileCards[{page}].customProfileCard must be an object")]
    CardMustBeObject { page: usize },
    #[error("userCustomProfileCards[{page}].customProfileCard.{category} is missing")]
    MissingCardCategory { page: usize, category: &'static str },
    #[error("userCustomProfileCards[{page}].customProfileCard.{category} must be an array")]
    CardCategoryMustBeArray { page: usize, category: &'static str },
    #[error("userCustomProfileCards[{page}] contains {count} elements; maximum is {max}")]
    ElementLimitExceeded {
        page: usize,
        count: usize,
        max: usize,
    },
}

impl GameProfileDocument {
    pub const MAX_ELEMENTS_PER_PAGE: usize = 150;

    pub fn blank() -> Self {
        let card = CARD_ARRAYS
            .into_iter()
            .map(|key| (key.to_owned(), Value::Array(Vec::new())))
            .collect::<Map<_, _>>();
        let page = Map::from_iter([
            ("customProfileCard".to_owned(), Value::Object(card)),
            ("customProfileCardId".to_owned(), Value::from(0)),
            ("customProfileId".to_owned(), Value::from(0)),
            ("seq".to_owned(), Value::from(1)),
        ]);
        let export = Map::from_iter([(
            CUSTOM_PROFILE_CARDS_KEY.to_owned(),
            Value::Array(vec![Value::Object(page)]),
        )]);
        Self {
            export: Value::Object(export),
        }
    }

    pub fn from_profile_value(profile: Value) -> Result<Self, GameProfileDocumentError> {
        let profile = profile
            .as_object()
            .ok_or(GameProfileDocumentError::ProfileMustBeObject)?;
        let cards = profile
            .get(CUSTOM_PROFILE_CARDS_KEY)
            .ok_or(GameProfileDocumentError::MissingCustomProfileCards)?
            .clone();
        let document = Self {
            export: Value::Object(Map::from_iter([(
                CUSTOM_PROFILE_CARDS_KEY.to_owned(),
                cards,
            )])),
        };
        document.validate()?;
        Ok(document)
    }

    pub fn from_export_value(export: Value) -> Result<Self, GameProfileDocumentError> {
        Self::from_profile_value(export)
    }

    pub fn export_value(&self) -> Value {
        self.export.clone()
    }

    pub fn page_count(&self) -> usize {
        self.export[CUSTOM_PROFILE_CARDS_KEY]
            .as_array()
            .map_or(0, Vec::len)
    }

    pub fn append_blank_page(&mut self) {
        let blank_page = Self::blank().export[CUSTOM_PROFILE_CARDS_KEY][0].clone();
        self.export[CUSTOM_PROFILE_CARDS_KEY]
            .as_array_mut()
            .expect("validated document")
            .push(blank_page);
        self.normalize_page_sequence();
    }

    pub fn normalize_page_sequence(&mut self) {
        for (index, page) in self.export[CUSTOM_PROFILE_CARDS_KEY]
            .as_array_mut()
            .expect("validated document")
            .iter_mut()
            .enumerate()
        {
            page["seq"] = Value::from(index + 1);
        }
    }

    pub(crate) fn pages(&self) -> &[Value] {
        self.export[CUSTOM_PROFILE_CARDS_KEY]
            .as_array()
            .expect("validated document")
    }

    pub(crate) fn pages_mut(&mut self) -> &mut Vec<Value> {
        self.export[CUSTOM_PROFILE_CARDS_KEY]
            .as_array_mut()
            .expect("validated document")
    }

    pub(crate) fn validate(&self) -> Result<(), GameProfileDocumentError> {
        let pages = self.export[CUSTOM_PROFILE_CARDS_KEY]
            .as_array()
            .ok_or(GameProfileDocumentError::CustomProfileCardsMustBeArray)?;
        for (page_index, page) in pages.iter().enumerate() {
            let page = page
                .as_object()
                .ok_or(GameProfileDocumentError::PageMustBeObject { page: page_index })?;
            for field in ["customProfileCardId", "customProfileId", "seq"] {
                if page.get(field).and_then(Value::as_i64).is_none() {
                    return Err(GameProfileDocumentError::PageIdentityMustBeInteger {
                        page: page_index,
                        field,
                    });
                }
            }
            let card = page
                .get("customProfileCard")
                .and_then(Value::as_object)
                .ok_or(GameProfileDocumentError::CardMustBeObject { page: page_index })?;
            let mut count = 0;
            for category in CARD_ARRAYS {
                let values = card
                    .get(category)
                    .ok_or(GameProfileDocumentError::MissingCardCategory {
                        page: page_index,
                        category,
                    })?
                    .as_array()
                    .ok_or(GameProfileDocumentError::CardCategoryMustBeArray {
                        page: page_index,
                        category,
                    })?;
                count += values.len();
            }
            if count > Self::MAX_ELEMENTS_PER_PAGE {
                return Err(GameProfileDocumentError::ElementLimitExceeded {
                    page: page_index,
                    count,
                    max: Self::MAX_ELEMENTS_PER_PAGE,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::GameProfileDocument;
    use serde_json::{json, Value};

    use super::CARD_ARRAYS;

    #[test]
    fn blank_document_uses_the_public_game_profile_envelope() {
        let document = GameProfileDocument::blank();
        let exported = document.export_value();
        let pages = exported["userCustomProfileCards"].as_array().unwrap();

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0]["customProfileCardId"], 0);
        assert_eq!(pages[0]["customProfileId"], 0);
        assert_eq!(pages[0]["seq"], 1);

        let card = pages[0]["customProfileCard"].as_object().unwrap();
        assert_eq!(card.len(), CARD_ARRAYS.len());
        for key in CARD_ARRAYS {
            assert_eq!(card[key], json!([]), "array {key}");
        }
    }

    #[test]
    fn full_profile_import_keeps_only_custom_profile_cards() {
        let profile = json!({
            "user": { "userId": 42, "name": "not exported" },
            "userDeck": { "deckId": 7 },
            "userCustomProfileCards": [{
                "customProfileCard": {
                    "bondsHonors": [], "cardMembers": [], "collections": [],
                    "generalBackgrounds": [], "generals": [], "honors": [],
                    "others": [], "shapes": [], "stamps": [],
                    "standMembers": [], "storyBackgrounds": [], "texts": []
                },
                "customProfileCardId": 9,
                "customProfileId": 3,
                "seq": 1
            }]
        });

        let document = GameProfileDocument::from_profile_value(profile).unwrap();
        let exported = document.export_value();

        assert_eq!(exported.as_object().unwrap().len(), 1);
        assert!(exported.get("user").is_none());
        assert!(exported.get("userDeck").is_none());
        assert_eq!(
            exported["userCustomProfileCards"][0]["customProfileCardId"],
            9
        );
    }

    #[test]
    fn import_export_preserves_unknown_legal_custom_profile_fields() {
        let profile = json!({
            "userCustomProfileCards": [{
                "customProfileCard": {
                    "bondsHonors": [], "cardMembers": [], "collections": [],
                    "generalBackgrounds": [],
                    "generals": [{
                        "objectData": {
                            "position": {"x": 1.0, "y": 2.0, "z": 0.0},
                            "scale": {"x": 1.0, "y": 1.0, "z": 1.0},
                            "rotation": {"x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0},
                            "layer": 0, "lock": false, "visible": true,
                            "futureTransformFlag": "keep"
                        },
                        "type": 13,
                        "futureGeneralField": {"enabled": true}
                    }],
                    "honors": [], "others": [], "shapes": [], "stamps": [],
                    "standMembers": [], "storyBackgrounds": [], "texts": [],
                    "version": 3,
                    "futureCardField": [1, 2, 3]
                },
                "customProfileCardId": 9,
                "customProfileId": 3,
                "seq": 1,
                "futurePageField": "keep"
            }]
        });
        let expected = profile["userCustomProfileCards"].clone();

        let document = GameProfileDocument::from_profile_value(profile).unwrap();

        assert_eq!(document.export_value()["userCustomProfileCards"], expected);
    }

    #[test]
    fn rejects_missing_or_non_array_authored_categories() {
        let mut missing = GameProfileDocument::blank().export_value();
        missing["userCustomProfileCards"][0]["customProfileCard"]
            .as_object_mut()
            .unwrap()
            .remove("texts");
        assert!(GameProfileDocument::from_export_value(missing).is_err());

        let mut wrong_type = GameProfileDocument::blank().export_value();
        wrong_type["userCustomProfileCards"][0]["customProfileCard"]["texts"] = json!({});
        assert!(GameProfileDocument::from_export_value(wrong_type).is_err());
    }

    #[test]
    fn rejects_invalid_page_shape_and_non_integer_identity_fields() {
        let invalid_values = [
            json!({"userCustomProfileCards": [null]}),
            json!({"userCustomProfileCards": [{
                "customProfileCard": null,
                "customProfileCardId": 0,
                "customProfileId": 0,
                "seq": 1
            }]}),
        ];
        for value in invalid_values {
            assert!(GameProfileDocument::from_export_value(value).is_err());
        }

        for field in ["customProfileCardId", "customProfileId", "seq"] {
            let mut value = GameProfileDocument::blank().export_value();
            value["userCustomProfileCards"][0][field] = json!(1.5);
            assert!(
                GameProfileDocument::from_export_value(value).is_err(),
                "field {field}"
            );
        }
    }

    #[test]
    fn rejects_more_than_150_elements_on_one_page() {
        let mut value = GameProfileDocument::blank().export_value();
        value["userCustomProfileCards"][0]["customProfileCard"]["texts"] =
            Value::Array((0..151).map(|_| json!({})).collect());

        assert!(GameProfileDocument::from_export_value(value).is_err());
    }

    #[test]
    fn append_blank_page_uses_zero_ids_and_normalizes_sequence() {
        let mut value = GameProfileDocument::blank().export_value();
        value["userCustomProfileCards"][0]["seq"] = json!(42);
        let mut document = GameProfileDocument::from_export_value(value).unwrap();

        document.append_blank_page();

        let exported = document.export_value();
        let pages = exported["userCustomProfileCards"].as_array().unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0]["seq"], 1);
        assert_eq!(pages[1]["seq"], 2);
        assert_eq!(pages[1]["customProfileCardId"], 0);
        assert_eq!(pages[1]["customProfileId"], 0);
        for key in CARD_ARRAYS {
            assert_eq!(pages[1]["customProfileCard"][key], json!([]));
        }
    }
}
