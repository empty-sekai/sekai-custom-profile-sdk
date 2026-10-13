use super::*;
use serde_json::json;

fn text() -> Value {
    json!({
        "objectData": {
            "position": {"x": 0.0, "y": 0.0, "z": 0.0},
            "scale": {"x": 1.0, "y": 1.0, "z": 1.0},
            "rotation": {"x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0},
            "layer": 0, "lock": false, "visible": true
        },
        "text": "请多关照!", "fontId": 1, "type": 513, "colorId": 1,
        "size": 24.0, "outlineColorId": 1, "outlineSize": 0.0, "lineSpacing": 0.0
    })
}

#[test]
fn commands_keep_stable_ids_and_export_only_game_fields() {
    let mut session = AuthoringSession::new(GameProfileDocument::blank());
    let created = session
        .apply(AuthoringCommand::Create {
            page: 0,
            category: AuthoringCategory::Texts,
            element: text(),
        })
        .unwrap();
    let id = created.changes[0].id;
    let selected = created
        .selected
        .as_ref()
        .expect("created element is selected");
    assert_eq!(selected.id, id);
    assert_eq!(selected.page, 0);
    assert_eq!(selected.category, AuthoringCategory::Texts);
    assert_eq!(selected.index, 0);
    assert_eq!(selected.element["text"], "请多关照!");
    assert_eq!(session.list_elements(), vec![selected.clone()]);
    session
        .apply(AuthoringCommand::SetTransform {
            id,
            position: Some([12.0, 34.0, 0.0]),
            scale: None,
            rotation: None,
        })
        .unwrap();
    session
        .apply(AuthoringCommand::SetLock { id, lock: true })
        .unwrap();

    let exported = session.export_value();
    let element = &exported["userCustomProfileCards"][0]["customProfileCard"]["texts"][0];
    assert_eq!(element["objectData"]["position"]["x"], 12.0);
    assert_eq!(element["objectData"]["lock"], true);
    assert!(element.get("authoringElementId").is_none());
    assert!(element.get("stableId").is_none());

    let cleared = session.select(None).unwrap();
    assert_eq!(cleared.selected_id, None);
    assert_eq!(cleared.selected, None);
}

#[test]
fn duplicate_offsets_and_undo_redo_restore_json_and_ids() {
    let mut session = AuthoringSession::new(GameProfileDocument::blank());
    let id = session
        .apply(AuthoringCommand::Create {
            page: 0,
            category: AuthoringCategory::Texts,
            element: text(),
        })
        .unwrap()
        .changes[0]
        .id;
    let duplicate = session.apply(AuthoringCommand::Duplicate { id }).unwrap();
    let duplicate_id = duplicate.changes[0].id;
    let values = &session.export_value()["userCustomProfileCards"][0]["customProfileCard"]["texts"];
    assert_eq!(values[1]["objectData"]["position"]["x"], 30.0);
    assert_eq!(values[1]["objectData"]["position"]["y"], 30.0);

    session.undo().unwrap().unwrap();
    assert_eq!(session.element_ids(0).unwrap(), vec![id]);
    session.redo().unwrap().unwrap();
    assert_eq!(session.element_ids(0).unwrap(), vec![id, duplicate_id]);
}

#[test]
fn history_is_bounded_to_150_commands() {
    let mut session = AuthoringSession::new(GameProfileDocument::blank());
    let id = session
        .apply(AuthoringCommand::Create {
            page: 0,
            category: AuthoringCategory::Texts,
            element: text(),
        })
        .unwrap()
        .changes[0]
        .id;
    for index in 0..200 {
        session
            .apply(AuthoringCommand::SetVisible {
                id,
                visible: index % 2 == 0,
            })
            .unwrap();
    }
    let mut count = 0;
    while session.undo().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, AUTHORING_HISTORY_LIMIT);
}

#[test]
fn checkpoint_restores_stable_ids_selection_and_both_history_directions() {
    let mut session = AuthoringSession::new(GameProfileDocument::blank());
    let id = session
        .apply(AuthoringCommand::Create {
            page: 0,
            category: AuthoringCategory::Texts,
            element: text(),
        })
        .unwrap()
        .changes[0]
        .id;
    session
        .apply(AuthoringCommand::SetVisible { id, visible: false })
        .unwrap();
    session.undo().unwrap().unwrap();

    let checkpoint = session.export_checkpoint_value().unwrap();
    assert_eq!(checkpoint["schema"], AUTHORING_CHECKPOINT_SCHEMA);
    assert_eq!(checkpoint["undo"].as_array().unwrap().len(), 1);
    assert_eq!(checkpoint["redo"].as_array().unwrap().len(), 1);

    let mut restored = AuthoringSession::from_checkpoint_value(checkpoint).unwrap();
    assert_eq!(restored.revision(), session.revision());
    assert_eq!(restored.selected_id(), Some(id));
    assert_eq!(restored.element_ids(0).unwrap(), vec![id]);
    assert_eq!(
        restored.export_value()["userCustomProfileCards"][0]["customProfileCard"]["texts"][0]
            ["objectData"]["visible"],
        true
    );

    let redone = restored.redo().unwrap().unwrap();
    assert_eq!(redone.selected_id, Some(id));
    assert_eq!(
        restored.export_value()["userCustomProfileCards"][0]["customProfileCard"]["texts"][0]
            ["objectData"]["visible"],
        false
    );
    restored.undo().unwrap().unwrap();
    assert_eq!(
        restored.export_value()["userCustomProfileCards"][0]["customProfileCard"]["texts"][0]
            ["objectData"]["visible"],
        true
    );
    restored.undo().unwrap().unwrap();
    assert!(restored.element_ids(0).unwrap().is_empty());
    assert_eq!(restored.redo().unwrap().unwrap().changes[0].id, id);
}

#[test]
fn checkpoint_is_bounded_and_rejects_corrupt_identity_state() {
    let mut session = AuthoringSession::new(GameProfileDocument::blank());
    let first = session
        .apply(AuthoringCommand::Create {
            page: 0,
            category: AuthoringCategory::Texts,
            element: text(),
        })
        .unwrap()
        .changes[0]
        .id;
    session
        .apply(AuthoringCommand::Duplicate { id: first })
        .unwrap();
    for index in 0..200 {
        session
            .apply(AuthoringCommand::SetVisible {
                id: first,
                visible: index % 2 == 0,
            })
            .unwrap();
    }
    let checkpoint = session.export_checkpoint_value().unwrap();
    assert_eq!(
        checkpoint["undo"].as_array().unwrap().len(),
        AUTHORING_HISTORY_LIMIT
    );

    let mut wrong_schema = checkpoint.clone();
    wrong_schema["schema"] = Value::from("allium.renderer-authoring-checkpoint/v2");
    assert!(matches!(
        AuthoringSession::from_checkpoint_value(wrong_schema),
        Err(AuthoringError::InvalidCheckpoint(_))
    ));

    let mut duplicate_id = checkpoint.clone();
    let ids = duplicate_id["ids"][0]["texts"].as_array_mut().unwrap();
    ids[1] = ids[0].clone();
    assert!(matches!(
        AuthoringSession::from_checkpoint_value(duplicate_id),
        Err(AuthoringError::InvalidCheckpoint(_))
    ));

    let mut invalid_next = checkpoint;
    invalid_next["nextId"] = Value::from(first.0);
    assert!(matches!(
        AuthoringSession::from_checkpoint_value(invalid_next),
        Err(AuthoringError::InvalidCheckpoint(_))
    ));
}

#[test]
fn checkpoint_rejects_an_active_gesture() {
    let mut session = AuthoringSession::new(GameProfileDocument::blank());
    let id = session
        .apply(AuthoringCommand::Create {
            page: 0,
            category: AuthoringCategory::Texts,
            element: text(),
        })
        .unwrap()
        .changes[0]
        .id;
    session.begin_gesture(id).unwrap();
    assert!(matches!(
        session.export_checkpoint_value(),
        Err(AuthoringError::CheckpointGestureInProgress)
    ));
}

#[test]
fn gesture_previews_do_not_consume_history_and_commit_once() {
    let mut session = AuthoringSession::new(GameProfileDocument::blank());
    let created = session
        .apply(AuthoringCommand::Create {
            page: 0,
            category: AuthoringCategory::Texts,
            element: text(),
        })
        .unwrap();
    let id = created.changes[0].id;
    assert_eq!(created.selected_id, Some(id));
    assert!(created.changes[0].element.is_some());

    session.begin_gesture(id).unwrap();
    for x in [10.0, 20.0, 30.0] {
        let preview = session
            .preview_gesture(AuthoringCommand::SetTransform {
                id,
                position: Some([x, 5.0, 0.0]),
                scale: None,
                rotation: None,
            })
            .unwrap();
        assert_eq!(preview.revision, 1);
        assert_eq!(
            preview.changes[0].element.as_ref().unwrap()["objectData"]["position"]["x"],
            x
        );
    }
    let committed = session.commit_gesture().unwrap();
    assert_eq!(committed.revision, 2);

    session.undo().unwrap().unwrap();
    assert_eq!(
        session.export_value()["userCustomProfileCards"][0]["customProfileCard"]["texts"][0]
            ["objectData"]["position"]["x"],
        0.0
    );
}

#[test]
fn gesture_can_preview_text_parameters_and_commit_once() {
    let mut session = AuthoringSession::new(GameProfileDocument::blank());
    let id = session
        .apply(AuthoringCommand::Create {
            page: 0,
            category: AuthoringCategory::Texts,
            element: text(),
        })
        .unwrap()
        .changes[0]
        .id;

    session.begin_gesture(id).unwrap();
    let preview = session
        .preview_gesture(AuthoringCommand::SetParameters {
            id,
            values: BTreeMap::from([("size".into(), Value::from(64.0))]),
        })
        .unwrap();
    assert_eq!(preview.revision, 1);
    assert_eq!(preview.selected.unwrap().element["size"], 64.0);
    assert_eq!(session.commit_gesture().unwrap().revision, 2);

    session.undo().unwrap().unwrap();
    assert_eq!(
        session.export_value()["userCustomProfileCards"][0]["customProfileCard"]["texts"][0]
            ["size"],
        24.0
    );
}

#[test]
fn page_operations_preserve_game_sequence_and_round_trip_through_history() {
    let mut session = AuthoringSession::new(GameProfileDocument::blank());
    session
        .apply(AuthoringCommand::Create {
            page: 0,
            category: AuthoringCategory::Texts,
            element: text(),
        })
        .unwrap();
    let duplicated = session.duplicate_page(0).unwrap();
    assert_eq!(
        duplicated.page_changes[0].kind,
        AuthoringPageChangeKind::Inserted
    );
    let pages = session.export_value()["userCustomProfileCards"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0]["seq"], 1);
    assert_eq!(pages[1]["seq"], 2);
    assert_eq!(pages[1]["customProfileCardId"], 0);
    assert_ne!(
        session.element_ids(0).unwrap(),
        session.element_ids(1).unwrap()
    );

    session.move_page(1, 0).unwrap();
    session.delete_page(1).unwrap();
    assert_eq!(
        session.export_value()["userCustomProfileCards"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    session.undo().unwrap().unwrap();
    assert_eq!(
        session.export_value()["userCustomProfileCards"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

fn session_with_texts(count: usize) -> (AuthoringSession, Vec<AuthoringElementId>) {
    let mut session = AuthoringSession::new(GameProfileDocument::blank());
    let ids = (0..count)
        .map(|_| {
            session
                .apply(AuthoringCommand::Create {
                    page: 0,
                    category: AuthoringCategory::Texts,
                    element: text(),
                })
                .unwrap()
                .changes[0]
                .id
        })
        .collect();
    (session, ids)
}

fn layer_of(change: &AuthoringElementChange) -> u64 {
    change.element.as_ref().unwrap()["objectData"]["layer"]
        .as_u64()
        .unwrap()
}

#[test]
fn delete_reports_every_element_whose_layer_it_renumbers() {
    let (mut session, ids) = session_with_texts(3);

    let deleted = session
        .apply(AuthoringCommand::Delete { id: ids[0] })
        .unwrap();
    let kinds = deleted
        .changes
        .iter()
        .map(|change| (change.id, change.kind))
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            (ids[0], AuthoringChangeKind::Removed),
            (ids[1], AuthoringChangeKind::Updated),
            (ids[2], AuthoringChangeKind::Updated),
        ]
    );
    assert_eq!(layer_of(&deleted.changes[1]), 0);
    assert_eq!(layer_of(&deleted.changes[2]), 1);

    let undone = session.undo().unwrap().unwrap();
    let kinds = undone
        .changes
        .iter()
        .map(|change| (change.id, change.kind))
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            (ids[0], AuthoringChangeKind::Inserted),
            (ids[1], AuthoringChangeKind::Updated),
            (ids[2], AuthoringChangeKind::Updated),
        ]
    );
    assert_eq!(layer_of(&undone.changes[0]), 0);
    assert_eq!(layer_of(&undone.changes[1]), 1);
    assert_eq!(layer_of(&undone.changes[2]), 2);

    // Deleting the top element renumbers nothing else.
    let top = session
        .apply(AuthoringCommand::Delete { id: ids[2] })
        .unwrap();
    assert_eq!(top.changes.len(), 1);
}

#[test]
fn commands_that_change_nothing_leave_history_and_revision_alone() {
    let (mut session, ids) = session_with_texts(2);
    session.undo().unwrap().unwrap();
    let revision = session.revision();
    let id = ids[0];

    let no_ops = [
        AuthoringCommand::SetVisible { id, visible: true },
        AuthoringCommand::SetLock { id, lock: false },
        AuthoringCommand::SetTransform {
            id,
            position: None,
            scale: None,
            rotation: None,
        },
        AuthoringCommand::SetTransform {
            id,
            position: Some([0.0, 0.0, 0.0]),
            scale: Some([1.0, 1.0, 1.0]),
            rotation: Some([0.0, 0.0, 0.0, 1.0]),
        },
        AuthoringCommand::SetParameters {
            id,
            values: BTreeMap::from([("size".into(), Value::from(24.0))]),
        },
        AuthoringCommand::SetParameters {
            id,
            values: BTreeMap::new(),
        },
        AuthoringCommand::ChangeLayer { id, layer: 0 },
    ];
    for command in no_ops {
        let delta = session.apply(command.clone()).unwrap();
        assert_eq!(delta.revision, revision, "{command:?}");
        assert!(delta.changes.is_empty(), "{command:?}");
        assert!(delta.can_redo, "{command:?}");
    }
    assert_eq!(session.revision(), revision);
    // Only the creation of the remaining element is left to undo.
    session.undo().unwrap().unwrap();
    assert!(session.element_ids(0).unwrap().is_empty());
    assert!(session.undo().unwrap().is_none());
}

#[test]
fn page_operations_keep_the_game_order_of_imported_pages() {
    let page = |seq: i64, card_id: i64| {
        let mut page =
            GameProfileDocument::blank().export_value()["userCustomProfileCards"][0].clone();
        page["seq"] = Value::from(seq);
        page["customProfileCardId"] = Value::from(card_id);
        page
    };
    let document = GameProfileDocument::from_profile_value(serde_json::json!({
        "userCustomProfileCards": [page(2, 20), page(1, 10)]
    }))
    .unwrap();
    let mut session = AuthoringSession::new(document);

    session.append_blank_page().unwrap();

    let exported = session.export_value();
    let mut pages = exported["userCustomProfileCards"]
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
    // The game shows pages by ascending seq.
    pages.sort_by_key(|(seq, _)| *seq);
    let shown = pages.iter().map(|(_, card)| *card).collect::<Vec<_>>();
    assert_eq!(shown, [10, 20, 0]);
}

#[test]
fn checkpoints_keep_the_page_order_their_ids_were_captured_with() {
    let mut first =
        GameProfileDocument::blank().export_value()["userCustomProfileCards"][0].clone();
    first["seq"] = Value::from(2);
    first["customProfileCard"]["texts"] = serde_json::json!([text()]);
    let mut second = first.clone();
    second["seq"] = Value::from(1);
    second["customProfileCard"]["texts"] = serde_json::json!([]);
    let document = GameProfileDocument::from_snapshot_value(serde_json::json!({
        "userCustomProfileCards": [first, second]
    }))
    .unwrap();
    let session = AuthoringSession::new(document);
    let text_id = session.element_ids(0).unwrap()[0];

    let restored =
        AuthoringSession::from_checkpoint_value(session.export_checkpoint_value().unwrap())
            .unwrap();

    assert_eq!(restored.element_ids(0).unwrap(), vec![text_id]);
    assert!(restored.element_ids(1).unwrap().is_empty());
    assert_eq!(restored.export_value(), session.export_value());
}

#[test]
fn a_custom_profile_holds_at_most_ten_pages() {
    let mut session = AuthoringSession::new(GameProfileDocument::blank());
    for _ in 1..10 {
        session.append_blank_page().unwrap();
    }
    assert_eq!(
        session.export_value()["userCustomProfileCards"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let revision = session.revision();
    assert!(session.append_blank_page().is_err());
    assert!(session.duplicate_page(0).is_err());
    assert_eq!(session.revision(), revision);
    assert_eq!(
        session.export_value()["userCustomProfileCards"]
            .as_array()
            .unwrap()
            .len(),
        10
    );

    session.delete_page(9).unwrap();
    session.duplicate_page(0).unwrap();
}

#[test]
fn exhausted_element_ids_fail_closed() {
    let (session, ids) = session_with_texts(1);
    let mut checkpoint = session.export_checkpoint_value().unwrap();
    checkpoint["nextId"] = Value::from(u32::MAX - 1);
    let mut restored = AuthoringSession::from_checkpoint_value(checkpoint).unwrap();
    let create = || AuthoringCommand::Create {
        page: 0,
        category: AuthoringCategory::Texts,
        element: text(),
    };

    let last = restored.apply(create()).unwrap().changes[0].id;
    assert_eq!(last, AuthoringElementId(u32::MAX - 1));
    let before = restored.export_value();
    let revision = restored.revision();
    assert!(restored.apply(create()).is_err());
    assert!(restored
        .apply(AuthoringCommand::Duplicate { id: ids[0] })
        .is_err());
    assert!(restored.duplicate_page(0).is_err());
    assert_eq!(restored.export_value(), before);
    assert_eq!(restored.revision(), revision);
    assert_eq!(restored.element_ids(0).unwrap(), vec![ids[0], last]);
    restored.append_blank_page().unwrap();
}

#[test]
fn checkpoint_rejects_a_revision_that_cannot_round_trip_through_javascript() {
    let (session, _) = session_with_texts(1);
    let checkpoint = session.export_checkpoint_value().unwrap();
    for revision in [u64::MAX, 1u64 << 53] {
        let mut corrupt = checkpoint.clone();
        corrupt["revision"] = Value::from(revision);
        assert!(
            matches!(
                AuthoringSession::from_checkpoint_value(corrupt),
                Err(AuthoringError::InvalidCheckpoint(_))
            ),
            "revision {revision}"
        );
    }
    let mut largest = checkpoint;
    largest["revision"] = Value::from((1u64 << 53) - 1);
    assert!(AuthoringSession::from_checkpoint_value(largest).is_ok());
}

#[test]
fn undo_and_redo_are_rejected_while_a_gesture_is_active() {
    // Restoring a snapshot mid-gesture would desync the active gesture's
    // baseline; both entries must wait for the gesture to end.
    let mut session = AuthoringSession::new(GameProfileDocument::blank());
    let id = session
        .apply(AuthoringCommand::Create {
            page: 0,
            category: AuthoringCategory::Texts,
            element: text(),
        })
        .unwrap()
        .changes[0]
        .id;
    session.begin_gesture(id).unwrap();
    assert!(matches!(
        session.undo(),
        Err(AuthoringError::GestureInProgress)
    ));
    assert!(matches!(
        session.redo(),
        Err(AuthoringError::GestureInProgress)
    ));
    // Once the gesture is gone the pre-gesture history is reachable again.
    session.cancel_gesture().unwrap();
    assert!(session.undo().unwrap().is_some());
}

fn at_layer(mut element: Value, layer: u64) -> Value {
    element["objectData"]["layer"] = Value::from(layer);
    element
}

fn user_interface_icon() -> Value {
    json!({ "objectData": text()["objectData"], "id": 1, "colorId": 1, "alpha": 1.0 })
}

/// A page holding texts at layers 0 and 2 and a character icon at layer 1.
fn session_with_a_character_icon() -> AuthoringSession {
    let mut profile = GameProfileDocument::blank().export_value();
    let card = &mut profile["userCustomProfileCards"][0]["customProfileCard"];
    card["texts"] = json!([at_layer(text(), 0), at_layer(text(), 2)]);
    card["characterIcons"] = json!([at_layer(
        json!({ "objectData": text()["objectData"], "id": 1 }),
        1
    )]);
    AuthoringSession::new(GameProfileDocument::from_profile_value(profile).unwrap())
}

fn text_ids(session: &AuthoringSession) -> Vec<AuthoringElementId> {
    session
        .list_elements()
        .into_iter()
        .filter(|element| element.category == AuthoringCategory::Texts)
        .map(|element| element.id)
        .collect()
}

#[test]
fn icon_categories_share_the_page_layers_and_stay_absent_until_used() {
    let mut session = session_with_a_character_icon();
    let listed = session.list_elements();
    assert_eq!(listed.len(), 3);
    let icon = listed
        .iter()
        .find(|element| element.category == AuthoringCategory::CharacterIcons)
        .expect("imported character icon")
        .id;

    let created = session
        .apply(AuthoringCommand::Create {
            page: 0,
            category: AuthoringCategory::UserInterfaceIcons,
            element: user_interface_icon(),
        })
        .unwrap();
    let created_element = created.changes[0].element.as_ref().unwrap();
    assert_eq!(created_element["objectData"]["layer"], 3);

    session
        .apply(AuthoringCommand::Delete { id: icon })
        .unwrap();
    let exported = session.export_value();
    let card = &exported["userCustomProfileCards"][0]["customProfileCard"];
    assert_eq!(card["characterIcons"], json!([]));
    assert_eq!(card["texts"][0]["objectData"]["layer"], 0);
    assert_eq!(card["texts"][1]["objectData"]["layer"], 1);
    assert_eq!(card["userInterfaceIcons"][0]["objectData"]["layer"], 2);
    assert!(card.get("materials").is_none());

    let blank = AuthoringSession::new(GameProfileDocument::blank()).export_value();
    for key in OPTIONAL_CARD_ARRAYS {
        assert!(blank["userCustomProfileCards"][0]["customProfileCard"]
            .get(key)
            .is_none());
    }
}

#[test]
fn layer_moves_include_icons_without_adding_absent_categories() {
    let mut session = session_with_a_character_icon();
    let top_text = text_ids(&session)[1];
    session
        .apply(AuthoringCommand::ChangeLayer {
            id: top_text,
            layer: 0,
        })
        .unwrap();
    let exported = session.export_value();
    let card = &exported["userCustomProfileCards"][0]["customProfileCard"];
    assert_eq!(card["characterIcons"][0]["objectData"]["layer"], 2);
    assert_eq!(card["texts"][0]["objectData"]["layer"], 1);
    assert_eq!(card["texts"][1]["objectData"]["layer"], 0);
    assert!(card.get("materials").is_none());
    assert!(card.get("userInterfaceIcons").is_none());
}

#[test]
fn checkpoints_without_icon_category_ids_still_restore() {
    let mut session = session_with_a_character_icon();
    let texts = text_ids(&session);
    session
        .apply(AuthoringCommand::SetVisible {
            id: texts[0],
            visible: false,
        })
        .unwrap();
    let mut checkpoint = session.export_checkpoint_value().unwrap();
    let strip = |ids: &mut Value| {
        let ids = ids.as_object_mut().unwrap();
        ids.remove("materials");
        ids.remove("userInterfaceIcons");
    };
    strip(&mut checkpoint["ids"][0]);
    for entry in checkpoint["undo"].as_array_mut().unwrap() {
        for side in ["before", "after"] {
            strip(&mut entry[side]["snapshot"]["ids"]);
        }
    }
    let restored = AuthoringSession::from_checkpoint_value(checkpoint).unwrap();
    assert_eq!(restored.list_elements().len(), 3);
}
