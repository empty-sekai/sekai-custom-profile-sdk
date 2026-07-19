use std::cell::RefCell;
use std::collections::BTreeMap;

use allium_renderer_core::authoring_document::GameProfileDocument;
use allium_renderer_core::authoring_session::{
    AuthoringCommand, AuthoringElementId, AuthoringSession,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

thread_local! {
    static SESSIONS: RefCell<SessionTable> = RefCell::new(SessionTable::default());
}

#[derive(Default)]
struct SessionTable {
    next_handle: u32,
    sessions: BTreeMap<u32, AuthoringSession>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateResponse {
    handle: u32,
    revision: u64,
    document: Value,
}

pub fn create_blank() -> Result<String, String> {
    create(GameProfileDocument::blank())
}

pub fn import_profile(input: &str) -> Result<String, String> {
    let value = serde_json::from_str(input)
        .map_err(|error| format!("parse game profile failed: {error}"))?;
    let document =
        GameProfileDocument::from_profile_value(value).map_err(|error| error.to_string())?;
    create(document)
}

pub fn restore_checkpoint(input: &str) -> Result<String, String> {
    let value = serde_json::from_str(input)
        .map_err(|error| format!("parse authoring checkpoint failed: {error}"))?;
    let session =
        AuthoringSession::from_checkpoint_value(value).map_err(|error| error.to_string())?;
    create_session(session)
}

fn create(document: GameProfileDocument) -> Result<String, String> {
    create_session(AuthoringSession::new(document))
}

fn create_session(session: AuthoringSession) -> Result<String, String> {
    SESSIONS.with(|table| {
        let mut table = table.borrow_mut();
        table.next_handle = table.next_handle.wrapping_add(1).max(1);
        let handle = table.next_handle;
        let response = CreateResponse {
            handle,
            revision: session.revision(),
            document: session.export_value(),
        };
        table.sessions.insert(handle, session);
        serde_json::to_string(&response).map_err(|error| error.to_string())
    })
}

pub fn apply(handle: u32, input: &str) -> Result<String, String> {
    let command: AuthoringCommand = serde_json::from_str(input)
        .map_err(|error| format!("parse authoring command failed: {error}"))?;
    with_session(handle, |session| {
        let delta = session.apply(command).map_err(|error| error.to_string())?;
        serde_json::to_string(&delta).map_err(|error| error.to_string())
    })
}

#[derive(Deserialize)]
struct ElementRequest {
    id: AuthoringElementId,
}

#[derive(Deserialize)]
struct SelectionRequest {
    id: Option<AuthoringElementId>,
}

#[derive(Deserialize)]
struct PageRequest {
    page: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MovePageRequest {
    from_page: usize,
    page: usize,
}

pub fn select(handle: u32, input: &str) -> Result<String, String> {
    let request: SelectionRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse authoring selection failed: {error}"))?;
    with_session(handle, |session| {
        serde_json::to_string(
            &session
                .select(request.id)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    })
}

pub fn elements(handle: u32) -> Result<String, String> {
    with_session(handle, |session| {
        serde_json::to_string(&session.list_elements()).map_err(|error| error.to_string())
    })
}

pub fn begin_gesture(handle: u32, input: &str) -> Result<String, String> {
    let request: ElementRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse authoring gesture failed: {error}"))?;
    with_session(handle, |session| {
        serde_json::to_string(
            &session
                .begin_gesture(request.id)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    })
}

pub fn preview_gesture(handle: u32, input: &str) -> Result<String, String> {
    let command: AuthoringCommand = serde_json::from_str(input)
        .map_err(|error| format!("parse authoring gesture command failed: {error}"))?;
    with_session(handle, |session| {
        serde_json::to_string(
            &session
                .preview_gesture(command)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    })
}

pub fn commit_gesture(handle: u32) -> Result<String, String> {
    with_session(handle, |session| {
        serde_json::to_string(
            &session
                .commit_gesture()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    })
}

pub fn cancel_gesture(handle: u32) -> Result<String, String> {
    with_session(handle, |session| {
        serde_json::to_string(
            &session
                .cancel_gesture()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    })
}

pub fn append_page(handle: u32) -> Result<String, String> {
    with_session(handle, |session| {
        serde_json::to_string(
            &session
                .append_blank_page()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    })
}

pub fn duplicate_page(handle: u32, input: &str) -> Result<String, String> {
    let request: PageRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse authoring page failed: {error}"))?;
    with_session(handle, |session| {
        serde_json::to_string(
            &session
                .duplicate_page(request.page)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    })
}

pub fn delete_page(handle: u32, input: &str) -> Result<String, String> {
    let request: PageRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse authoring page failed: {error}"))?;
    with_session(handle, |session| {
        serde_json::to_string(
            &session
                .delete_page(request.page)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    })
}

pub fn move_page(handle: u32, input: &str) -> Result<String, String> {
    let request: MovePageRequest = serde_json::from_str(input)
        .map_err(|error| format!("parse authoring page move failed: {error}"))?;
    with_session(handle, |session| {
        serde_json::to_string(
            &session
                .move_page(request.from_page, request.page)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    })
}

pub fn undo(handle: u32) -> Result<String, String> {
    with_session(handle, |session| {
        serde_json::to_string(&session.undo().map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())
    })
}

pub fn redo(handle: u32) -> Result<String, String> {
    with_session(handle, |session| {
        serde_json::to_string(&session.redo().map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())
    })
}

pub fn export(handle: u32) -> Result<String, String> {
    with_session(handle, |session| {
        serde_json::to_string(&session.export_value()).map_err(|error| error.to_string())
    })
}

pub fn checkpoint(handle: u32) -> Result<String, String> {
    with_session(handle, |session| {
        serde_json::to_string(
            &session
                .export_checkpoint_value()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    })
}

pub fn destroy(handle: u32) -> bool {
    SESSIONS.with(|table| table.borrow_mut().sessions.remove(&handle).is_some())
}

fn with_session<T>(
    handle: u32,
    call: impl FnOnce(&mut AuthoringSession) -> Result<T, String>,
) -> Result<T, String> {
    SESSIONS.with(|table| {
        let mut table = table.borrow_mut();
        let session = table
            .sessions
            .get_mut(&handle)
            .ok_or_else(|| format!("unknown authoring session handle {handle}"))?;
        call(session)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_handle_applies_and_exports_without_browser_metadata() {
        let created: Value = serde_json::from_str(&create_blank().unwrap()).unwrap();
        let handle = created["handle"].as_u64().unwrap() as u32;
        let command = json!({
            "kind": "create",
            "page": 0,
            "category": "texts",
            "element": {
                "objectData": {
                    "position": {"x": 0.0, "y": 0.0, "z": 0.0},
                    "scale": {"x": 1.0, "y": 1.0, "z": 1.0},
                    "rotation": {"x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0},
                    "layer": 0, "lock": false, "visible": true
                },
                "text": "请多关照!", "fontId": 1, "type": 513, "colorId": 1,
                "size": 24.0, "outlineColorId": 1, "outlineSize": 0.0, "lineSpacing": 0.0
            }
        });
        let delta: Value =
            serde_json::from_str(&apply(handle, &command.to_string()).unwrap()).unwrap();
        assert_eq!(delta["revision"], 1);
        let exported: Value = serde_json::from_str(&export(handle).unwrap()).unwrap();
        assert_eq!(
            exported["userCustomProfileCards"][0]["customProfileCard"]["texts"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(exported.get("handle").is_none());
        assert!(destroy(handle));
    }
}
