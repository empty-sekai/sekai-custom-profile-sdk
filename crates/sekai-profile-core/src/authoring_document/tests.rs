use super::{GameProfileDocument, GameProfileDocumentError};
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

    assert_eq!(
        GameProfileDocument::from_export_value(value),
        Err(GameProfileDocumentError::ElementLimitExceeded {
            page: 0,
            count: 151,
            max: 150
        })
    );
}

fn element() -> Value {
    json!({
        "objectData": {
            "position": {"x": 0.0, "y": 0.0, "z": 0.0},
            "scale": {"x": 1.0, "y": 1.0, "z": 1.0},
            "rotation": {"x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0},
            "layer": 0, "lock": false, "visible": true
        },
        "type": 1
    })
}

#[test]
fn rejects_elements_without_the_game_object_data_shape() {
    let mut missing_lock = element();
    missing_lock["objectData"]
        .as_object_mut()
        .unwrap()
        .remove("lock");
    let malformed = [
        json!(5),
        json!(null),
        json!("text"),
        json!({}),
        json!({"objectData": 5}),
        json!({"objectData": {}}),
        missing_lock,
    ];
    for malformed in malformed {
        let mut value = GameProfileDocument::blank().export_value();
        value["userCustomProfileCards"][0]["customProfileCard"]["shapes"] =
            json!([element(), malformed.clone()]);
        assert_eq!(
            GameProfileDocument::from_export_value(value),
            Err(GameProfileDocumentError::InvalidElement {
                page: 0,
                category: "shapes",
                index: 1
            }),
            "element {malformed}"
        );
    }

    let mut value = GameProfileDocument::blank().export_value();
    value["userCustomProfileCards"][0]["customProfileCard"]["shapes"] = json!([element()]);
    assert!(GameProfileDocument::from_export_value(value).is_ok());
}

#[test]
fn icon_arrays_may_be_absent_but_are_checked_when_present() {
    let with = |key: &str, values: Value| {
        let mut value = GameProfileDocument::blank().export_value();
        value["userCustomProfileCards"][0]["customProfileCard"][key] = values;
        GameProfileDocument::from_export_value(value)
    };
    assert!(with("characterIcons", json!([element()])).is_ok());
    assert_eq!(
        with("materials", json!({})),
        Err(GameProfileDocumentError::CardCategoryMustBeArray {
            page: 0,
            category: "materials"
        })
    );
    assert_eq!(
        with("userInterfaceIcons", json!([element(), {}])),
        Err(GameProfileDocumentError::InvalidElement {
            page: 0,
            category: "userInterfaceIcons",
            index: 1
        })
    );

    let mut value = GameProfileDocument::blank().export_value();
    let card = &mut value["userCustomProfileCards"][0]["customProfileCard"];
    card["texts"] = Value::Array((0..100).map(|_| element()).collect());
    card["materials"] = Value::Array((0..51).map(|_| element()).collect());
    assert_eq!(
        GameProfileDocument::from_export_value(value),
        Err(GameProfileDocumentError::ElementLimitExceeded {
            page: 0,
            count: 151,
            max: 150
        })
    );
}

#[test]
fn imported_pages_follow_ascending_seq_like_the_game() {
    let page = |seq: i64, card_id: i64| {
        let mut page =
            GameProfileDocument::blank().export_value()["userCustomProfileCards"][0].clone();
        page["seq"] = json!(seq);
        page["customProfileCardId"] = json!(card_id);
        page
    };
    let profile = json!({
        "userCustomProfileCards": [page(6, 60), page(2, 20), page(5, 50), page(2, 21)]
    });

    let exported = GameProfileDocument::from_profile_value(profile)
        .unwrap()
        .export_value();
    let order = exported["userCustomProfileCards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|page| {
            (
                page["seq"].as_i64().unwrap(),
                page["customProfileCardId"].as_i64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(order, [(2, 20), (2, 21), (5, 50), (6, 60)]);
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
