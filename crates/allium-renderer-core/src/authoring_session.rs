//! Stateful editing commands for game-compatible custom-profile documents.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::authoring_document::{GameProfileDocument, GameProfileDocumentError};

pub const AUTHORING_HISTORY_LIMIT: usize = 150;
pub const AUTHORING_CHECKPOINT_SCHEMA: &str = "allium.renderer-authoring-checkpoint/v1";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct AuthoringElementId(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthoringCategory {
    BondsHonors,
    CardMembers,
    Collections,
    GeneralBackgrounds,
    Generals,
    Honors,
    Others,
    Shapes,
    Stamps,
    StandMembers,
    StoryBackgrounds,
    Texts,
}

impl AuthoringCategory {
    pub const ALL: [Self; 12] = [
        Self::BondsHonors,
        Self::CardMembers,
        Self::Collections,
        Self::GeneralBackgrounds,
        Self::Generals,
        Self::Honors,
        Self::Others,
        Self::Shapes,
        Self::Stamps,
        Self::StandMembers,
        Self::StoryBackgrounds,
        Self::Texts,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::BondsHonors => "bondsHonors",
            Self::CardMembers => "cardMembers",
            Self::Collections => "collections",
            Self::GeneralBackgrounds => "generalBackgrounds",
            Self::Generals => "generals",
            Self::Honors => "honors",
            Self::Others => "others",
            Self::Shapes => "shapes",
            Self::Stamps => "stamps",
            Self::StandMembers => "standMembers",
            Self::StoryBackgrounds => "storyBackgrounds",
            Self::Texts => "texts",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthoringCommand {
    Create {
        page: usize,
        category: AuthoringCategory,
        element: Value,
    },
    Duplicate {
        id: AuthoringElementId,
    },
    Delete {
        id: AuthoringElementId,
    },
    SetTransform {
        id: AuthoringElementId,
        position: Option<[f32; 3]>,
        scale: Option<[f32; 3]>,
        rotation: Option<[f32; 4]>,
    },
    SetLock {
        id: AuthoringElementId,
        lock: bool,
    },
    SetVisible {
        id: AuthoringElementId,
        visible: bool,
    },
    SetParameters {
        id: AuthoringElementId,
        values: BTreeMap<String, Value>,
    },
    ChangeLayer {
        id: AuthoringElementId,
        layer: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringChangeKind {
    Inserted,
    Updated,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthoringElementChange {
    pub id: AuthoringElementId,
    pub page: usize,
    pub category: AuthoringCategory,
    pub kind: AuthoringChangeKind,
    /// Canonical game element after the change. Removed elements carry `None`.
    pub element: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthoringSelection {
    pub id: AuthoringElementId,
    pub page: usize,
    pub category: AuthoringCategory,
    pub index: usize,
    pub element: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthoringDelta {
    pub revision: u64,
    pub changes: Vec<AuthoringElementChange>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub selected_id: Option<AuthoringElementId>,
    pub selected: Option<AuthoringSelection>,
    pub page_changes: Vec<AuthoringPageChange>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringPageChangeKind {
    Inserted,
    Removed,
    Moved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthoringPageChange {
    pub kind: AuthoringPageChangeKind,
    pub page: usize,
    pub from_page: Option<usize>,
}

#[derive(Debug, Error)]
pub enum AuthoringError {
    #[error(transparent)]
    Document(#[from] GameProfileDocumentError),
    #[error("page {0} does not exist")]
    PageNotFound(usize),
    #[error("element {0:?} does not exist")]
    ElementNotFound(AuthoringElementId),
    #[error("element must be a JSON object")]
    ElementMustBeObject,
    #[error("element objectData is missing or invalid")]
    InvalidObjectData,
    #[error("transform contains a non-finite number")]
    NonFiniteTransform,
    #[error("objectData cannot be changed through SetParameters")]
    ObjectDataParameterForbidden,
    #[error("page {page} already contains the maximum of {max} elements")]
    ElementLimitReached { page: usize, max: usize },
    #[error("layer {layer} is outside page {page} element range 0..{count}")]
    InvalidLayer {
        page: usize,
        layer: usize,
        count: usize,
    },
    #[error("an authoring gesture is already active")]
    GestureAlreadyActive,
    #[error("no authoring gesture is active")]
    GestureNotActive,
    #[error("gesture command targets a different element")]
    GestureElementMismatch,
    #[error("document commands are unavailable while an authoring gesture is active")]
    GestureInProgress,
    #[error("the final custom-profile page cannot be deleted")]
    FinalPageCannotBeDeleted,
    #[error("page destination {0} does not exist")]
    PageDestinationNotFound(usize),
    #[error("an active authoring gesture cannot be checkpointed")]
    CheckpointGestureInProgress,
    #[error("invalid authoring checkpoint: {0}")]
    InvalidCheckpoint(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageSnapshot {
    page: usize,
    value: Value,
    ids: BTreeMap<AuthoringCategory, Vec<AuthoringElementId>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocumentSnapshot {
    value: Value,
    ids: Vec<BTreeMap<AuthoringCategory, Vec<AuthoringElementId>>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "snapshot", rename_all = "snake_case")]
enum HistorySnapshot {
    Page(PageSnapshot),
    Document(DocumentSnapshot),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    before: HistorySnapshot,
    after: HistorySnapshot,
    forward: Vec<AuthoringElementChange>,
    reverse: Vec<AuthoringElementChange>,
    selection_before: Option<AuthoringElementId>,
    selection_after: Option<AuthoringElementId>,
    page_forward: Vec<AuthoringPageChange>,
    page_reverse: Vec<AuthoringPageChange>,
}

#[derive(Clone)]
struct ActiveGesture {
    id: AuthoringElementId,
    before: PageSnapshot,
    selection_before: Option<AuthoringElementId>,
}

#[derive(Clone, Copy)]
struct ElementLocation {
    page: usize,
    category: AuthoringCategory,
    index: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthoringCheckpoint {
    schema: String,
    document: Value,
    ids: Vec<BTreeMap<AuthoringCategory, Vec<AuthoringElementId>>>,
    next_id: u32,
    revision: u64,
    undo: VecDeque<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    selected_id: Option<AuthoringElementId>,
}

#[derive(Clone)]
pub struct AuthoringSession {
    document: GameProfileDocument,
    ids: Vec<BTreeMap<AuthoringCategory, Vec<AuthoringElementId>>>,
    next_id: u32,
    revision: u64,
    undo: VecDeque<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    selected_id: Option<AuthoringElementId>,
    gesture: Option<ActiveGesture>,
}

impl AuthoringSession {
    pub fn new(document: GameProfileDocument) -> Self {
        let mut next_id = 1;
        let ids = document
            .pages()
            .iter()
            .map(|page| {
                AuthoringCategory::ALL
                    .into_iter()
                    .map(|category| {
                        let count = page["customProfileCard"][category.key()]
                            .as_array()
                            .expect("validated document")
                            .len();
                        let values = (0..count)
                            .map(|_| {
                                let id = AuthoringElementId(next_id);
                                next_id += 1;
                                id
                            })
                            .collect();
                        (category, values)
                    })
                    .collect()
            })
            .collect();
        Self {
            document,
            ids,
            next_id,
            revision: 0,
            undo: VecDeque::new(),
            redo: Vec::new(),
            selected_id: None,
            gesture: None,
        }
    }

    pub fn from_checkpoint_value(value: Value) -> Result<Self, AuthoringError> {
        let checkpoint: AuthoringCheckpoint = serde_json::from_value(value)
            .map_err(|error| AuthoringError::InvalidCheckpoint(error.to_string()))?;
        if checkpoint.schema != AUTHORING_CHECKPOINT_SCHEMA {
            return Err(AuthoringError::InvalidCheckpoint(format!(
                "unsupported schema {}",
                checkpoint.schema
            )));
        }
        if checkpoint.undo.len() + checkpoint.redo.len() > AUTHORING_HISTORY_LIMIT {
            return Err(AuthoringError::InvalidCheckpoint(format!(
                "history contains {} entries, maximum is {}",
                checkpoint.undo.len() + checkpoint.redo.len(),
                AUTHORING_HISTORY_LIMIT
            )));
        }
        let document = GameProfileDocument::from_export_value(checkpoint.document)?;
        let session = Self {
            document,
            ids: checkpoint.ids,
            next_id: checkpoint.next_id,
            revision: checkpoint.revision,
            undo: checkpoint.undo,
            redo: checkpoint.redo,
            selected_id: checkpoint.selected_id,
            gesture: None,
        };
        let mut maximum_id = session.validate_current_state()?;
        for entry in session.undo.iter().chain(session.redo.iter()) {
            maximum_id = maximum_id.max(validate_history_snapshot(&entry.before)?);
            maximum_id = maximum_id.max(validate_history_snapshot(&entry.after)?);
            for id in entry
                .forward
                .iter()
                .chain(entry.reverse.iter())
                .map(|change| change.id)
                .chain(
                    [entry.selection_before, entry.selection_after]
                        .into_iter()
                        .flatten(),
                )
            {
                if id.0 == 0 {
                    return Err(AuthoringError::InvalidCheckpoint(
                        "element IDs must be positive".into(),
                    ));
                }
                maximum_id = maximum_id.max(id.0);
            }
        }
        if session.next_id == 0 || session.next_id <= maximum_id {
            return Err(AuthoringError::InvalidCheckpoint(format!(
                "nextId {} must be greater than every element ID ({maximum_id})",
                session.next_id
            )));
        }

        // Exercise both directions before accepting persisted history. Snapshot validation alone
        // cannot prove that page-local entries remain applicable around page insert/delete entries.
        let mut probe = session.clone();
        while probe.undo()?.is_some() {}
        while probe.redo()?.is_some() {}
        while probe.undo()?.is_some() {}
        Ok(session)
    }

    pub fn export_checkpoint_value(&self) -> Result<Value, AuthoringError> {
        if self.gesture.is_some() {
            return Err(AuthoringError::CheckpointGestureInProgress);
        }
        serde_json::to_value(AuthoringCheckpoint {
            schema: AUTHORING_CHECKPOINT_SCHEMA.into(),
            document: self.document.export_value(),
            ids: self.ids.clone(),
            next_id: self.next_id,
            revision: self.revision,
            undo: self.undo.clone(),
            redo: self.redo.clone(),
            selected_id: self.selected_id,
        })
        .map_err(|error| AuthoringError::InvalidCheckpoint(error.to_string()))
    }

    pub fn export_value(&self) -> Value {
        self.document.export_value()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn selected_id(&self) -> Option<AuthoringElementId> {
        self.selected_id
    }

    /// Returns current authoring IDs with their page/category/index and element.
    pub fn list_elements(&self) -> Vec<AuthoringSelection> {
        let mut result = Vec::new();
        for (page, page_ids) in self.ids.iter().enumerate() {
            for category in AuthoringCategory::ALL {
                let Some(ids) = page_ids.get(&category) else {
                    continue;
                };
                let Ok(elements) = self.elements(page, category) else {
                    continue;
                };
                for (index, id) in ids.iter().copied().enumerate() {
                    let Some(element) = elements.get(index).cloned() else {
                        continue;
                    };
                    result.push(AuthoringSelection {
                        id,
                        page,
                        category,
                        index,
                        element,
                    });
                }
            }
        }
        result
    }

    pub fn select(
        &mut self,
        id: Option<AuthoringElementId>,
    ) -> Result<AuthoringDelta, AuthoringError> {
        if self.gesture.is_some() {
            return Err(AuthoringError::GestureInProgress);
        }
        if let Some(id) = id {
            self.locate(id)?;
        }
        self.selected_id = id;
        Ok(self.delta(Vec::new(), false))
    }

    pub fn append_blank_page(&mut self) -> Result<AuthoringDelta, AuthoringError> {
        self.ensure_no_gesture()?;
        let before = self.snapshot_document();
        let selection_before = self.selected_id;
        self.document.append_blank_page();
        self.ids.push(empty_page_ids());
        let page = self.document.page_count() - 1;
        self.record_document_change(
            before,
            selection_before,
            vec![AuthoringPageChange {
                kind: AuthoringPageChangeKind::Inserted,
                page,
                from_page: None,
            }],
            vec![AuthoringPageChange {
                kind: AuthoringPageChangeKind::Removed,
                page,
                from_page: None,
            }],
        )
    }

    pub fn duplicate_page(&mut self, page: usize) -> Result<AuthoringDelta, AuthoringError> {
        self.ensure_no_gesture()?;
        let source = self
            .document
            .pages()
            .get(page)
            .cloned()
            .ok_or(AuthoringError::PageNotFound(page))?;
        let before = self.snapshot_document();
        let selection_before = self.selected_id;
        let destination = page + 1;
        let mut copy = source;
        copy["customProfileCardId"] = Value::from(0);
        copy["customProfileId"] = Value::from(0);
        self.document.pages_mut().insert(destination, copy);
        let ids = self.allocate_page_ids(destination)?;
        self.ids.insert(destination, ids);
        self.document.normalize_page_sequence();
        self.selected_id = None;
        self.record_document_change(
            before,
            selection_before,
            vec![AuthoringPageChange {
                kind: AuthoringPageChangeKind::Inserted,
                page: destination,
                from_page: Some(page),
            }],
            vec![AuthoringPageChange {
                kind: AuthoringPageChangeKind::Removed,
                page: destination,
                from_page: None,
            }],
        )
    }

    pub fn delete_page(&mut self, page: usize) -> Result<AuthoringDelta, AuthoringError> {
        self.ensure_no_gesture()?;
        if self.document.page_count() == 1 {
            return Err(AuthoringError::FinalPageCannotBeDeleted);
        }
        if page >= self.document.page_count() {
            return Err(AuthoringError::PageNotFound(page));
        }
        let before = self.snapshot_document();
        let selection_before = self.selected_id;
        let removed_ids = self.ids.remove(page);
        self.document.pages_mut().remove(page);
        self.document.normalize_page_sequence();
        if self
            .selected_id
            .is_some_and(|id| removed_ids.values().any(|ids| ids.contains(&id)))
        {
            self.selected_id = None;
        }
        self.record_document_change(
            before,
            selection_before,
            vec![AuthoringPageChange {
                kind: AuthoringPageChangeKind::Removed,
                page,
                from_page: None,
            }],
            vec![AuthoringPageChange {
                kind: AuthoringPageChangeKind::Inserted,
                page,
                from_page: None,
            }],
        )
    }

    pub fn move_page(
        &mut self,
        from_page: usize,
        page: usize,
    ) -> Result<AuthoringDelta, AuthoringError> {
        self.ensure_no_gesture()?;
        let count = self.document.page_count();
        if from_page >= count {
            return Err(AuthoringError::PageNotFound(from_page));
        }
        if page >= count {
            return Err(AuthoringError::PageDestinationNotFound(page));
        }
        if from_page == page {
            return Ok(self.delta(Vec::new(), false));
        }
        let before = self.snapshot_document();
        let selection_before = self.selected_id;
        let value = self.document.pages_mut().remove(from_page);
        self.document.pages_mut().insert(page, value);
        let ids = self.ids.remove(from_page);
        self.ids.insert(page, ids);
        self.document.normalize_page_sequence();
        self.record_document_change(
            before,
            selection_before,
            vec![AuthoringPageChange {
                kind: AuthoringPageChangeKind::Moved,
                page,
                from_page: Some(from_page),
            }],
            vec![AuthoringPageChange {
                kind: AuthoringPageChangeKind::Moved,
                page: from_page,
                from_page: Some(page),
            }],
        )
    }

    pub fn element_ids(&self, page: usize) -> Result<Vec<AuthoringElementId>, AuthoringError> {
        let page = self
            .ids
            .get(page)
            .ok_or(AuthoringError::PageNotFound(page))?;
        Ok(AuthoringCategory::ALL
            .into_iter()
            .flat_map(|category| page[&category].iter().copied())
            .collect())
    }

    pub fn apply(&mut self, command: AuthoringCommand) -> Result<AuthoringDelta, AuthoringError> {
        if self.gesture.is_some() {
            return Err(AuthoringError::GestureInProgress);
        }
        let page = self.command_page(&command)?;
        let before = self.snapshot(page)?;
        let selection_before = self.selected_id;
        let forward = self.apply_inner(command)?;
        self.document.validate()?;
        let after = self.snapshot(page)?;
        let reverse = reverse_changes(&forward);
        self.undo.push_back(HistoryEntry {
            before: HistorySnapshot::Page(before),
            after: HistorySnapshot::Page(after),
            forward: forward.clone(),
            reverse,
            selection_before,
            selection_after: self.selected_id,
            page_forward: Vec::new(),
            page_reverse: Vec::new(),
        });
        while self.undo.len() > AUTHORING_HISTORY_LIMIT {
            self.undo.pop_front();
        }
        self.redo.clear();
        Ok(self.finish(forward, Vec::new()))
    }

    pub fn undo(&mut self) -> Result<Option<AuthoringDelta>, AuthoringError> {
        let Some(entry) = self.undo.pop_back() else {
            return Ok(None);
        };
        self.restore_history(&entry.before)?;
        self.selected_id = entry.selection_before;
        self.validate_selected_id()?;
        let changes = entry.reverse.clone();
        let page_changes = entry.page_reverse.clone();
        self.redo.push(entry);
        Ok(Some(self.finish(changes, page_changes)))
    }

    pub fn redo(&mut self) -> Result<Option<AuthoringDelta>, AuthoringError> {
        let Some(entry) = self.redo.pop() else {
            return Ok(None);
        };
        self.restore_history(&entry.after)?;
        self.selected_id = entry.selection_after;
        self.validate_selected_id()?;
        let changes = entry.forward.clone();
        let page_changes = entry.page_forward.clone();
        self.undo.push_back(entry);
        Ok(Some(self.finish(changes, page_changes)))
    }

    pub fn begin_gesture(
        &mut self,
        id: AuthoringElementId,
    ) -> Result<AuthoringDelta, AuthoringError> {
        if self.gesture.is_some() {
            return Err(AuthoringError::GestureAlreadyActive);
        }
        let location = self.locate(id)?;
        let gesture = ActiveGesture {
            id,
            before: self.snapshot(location.page)?,
            selection_before: self.selected_id,
        };
        self.selected_id = Some(id);
        self.gesture = Some(gesture);
        Ok(self.delta(Vec::new(), false))
    }

    pub fn preview_gesture(
        &mut self,
        command: AuthoringCommand,
    ) -> Result<AuthoringDelta, AuthoringError> {
        let id = match &command {
            AuthoringCommand::SetTransform { id, .. }
            | AuthoringCommand::SetParameters { id, .. } => *id,
            _ => return Err(AuthoringError::GestureElementMismatch),
        };
        if self.gesture.as_ref().map(|gesture| gesture.id) != Some(id) {
            return Err(if self.gesture.is_some() {
                AuthoringError::GestureElementMismatch
            } else {
                AuthoringError::GestureNotActive
            });
        }
        let changes = self.apply_inner(command)?;
        self.document.validate()?;
        Ok(self.delta(changes, false))
    }

    pub fn commit_gesture(&mut self) -> Result<AuthoringDelta, AuthoringError> {
        let gesture = self
            .gesture
            .take()
            .ok_or(AuthoringError::GestureNotActive)?;
        let location = self.locate(gesture.id)?;
        let after = self.snapshot(location.page)?;
        if after.value == gesture.before.value {
            return Ok(self.delta(Vec::new(), false));
        }
        let forward = vec![change(
            gesture.id,
            location.page,
            location.category,
            AuthoringChangeKind::Updated,
        )];
        self.undo.push_back(HistoryEntry {
            before: HistorySnapshot::Page(gesture.before),
            after: HistorySnapshot::Page(after),
            reverse: forward.clone(),
            forward: forward.clone(),
            selection_before: gesture.selection_before,
            selection_after: self.selected_id,
            page_forward: Vec::new(),
            page_reverse: Vec::new(),
        });
        while self.undo.len() > AUTHORING_HISTORY_LIMIT {
            self.undo.pop_front();
        }
        self.redo.clear();
        Ok(self.finish(forward, Vec::new()))
    }

    pub fn cancel_gesture(&mut self) -> Result<AuthoringDelta, AuthoringError> {
        let gesture = self
            .gesture
            .take()
            .ok_or(AuthoringError::GestureNotActive)?;
        let location = self.locate(gesture.id)?;
        self.restore_page(&gesture.before)?;
        self.selected_id = gesture.selection_before;
        Ok(self.delta(
            vec![change(
                gesture.id,
                location.page,
                location.category,
                AuthoringChangeKind::Updated,
            )],
            false,
        ))
    }

    fn command_page(&self, command: &AuthoringCommand) -> Result<usize, AuthoringError> {
        match command {
            AuthoringCommand::Create { page, .. } => self
                .ids
                .get(*page)
                .map(|_| *page)
                .ok_or(AuthoringError::PageNotFound(*page)),
            AuthoringCommand::Duplicate { id }
            | AuthoringCommand::Delete { id }
            | AuthoringCommand::SetTransform { id, .. }
            | AuthoringCommand::SetLock { id, .. }
            | AuthoringCommand::SetVisible { id, .. }
            | AuthoringCommand::SetParameters { id, .. }
            | AuthoringCommand::ChangeLayer { id, .. } => Ok(self.locate(*id)?.page),
        }
    }

    fn apply_inner(
        &mut self,
        command: AuthoringCommand,
    ) -> Result<Vec<AuthoringElementChange>, AuthoringError> {
        match command {
            AuthoringCommand::Create {
                page,
                category,
                mut element,
            } => {
                self.ensure_capacity(page)?;
                validate_element(&element)?;
                let layer = self.page_element_count(page);
                element["objectData"]["layer"] = Value::from(layer);
                let id = self.allocate_id();
                self.elements_mut(page, category)?.push(element);
                self.ids[page].get_mut(&category).unwrap().push(id);
                self.selected_id = Some(id);
                Ok(vec![change(
                    id,
                    page,
                    category,
                    AuthoringChangeKind::Inserted,
                )])
            }
            AuthoringCommand::Duplicate { id } => {
                let location = self.locate(id)?;
                self.ensure_capacity(location.page)?;
                let mut element =
                    self.elements(location.page, location.category)?[location.index].clone();
                offset_position(&mut element, 30.0, 30.0)?;
                let layer = self.page_element_count(location.page);
                element["objectData"]["layer"] = Value::from(layer);
                let new_id = self.allocate_id();
                self.elements_mut(location.page, location.category)?
                    .push(element);
                self.ids[location.page]
                    .get_mut(&location.category)
                    .unwrap()
                    .push(new_id);
                self.selected_id = Some(new_id);
                Ok(vec![change(
                    new_id,
                    location.page,
                    location.category,
                    AuthoringChangeKind::Inserted,
                )])
            }
            AuthoringCommand::Delete { id } => {
                let location = self.locate(id)?;
                self.elements_mut(location.page, location.category)?
                    .remove(location.index);
                self.ids[location.page]
                    .get_mut(&location.category)
                    .unwrap()
                    .remove(location.index);
                self.normalize_layers(location.page)?;
                if self.selected_id == Some(id) {
                    self.selected_id = None;
                }
                Ok(vec![change(
                    id,
                    location.page,
                    location.category,
                    AuthoringChangeKind::Removed,
                )])
            }
            AuthoringCommand::SetTransform {
                id,
                position,
                scale,
                rotation,
            } => {
                for values in [
                    position.as_ref().map(|v| &v[..]),
                    scale.as_ref().map(|v| &v[..]),
                    rotation.as_ref().map(|v| &v[..]),
                ]
                .into_iter()
                .flatten()
                {
                    if values.iter().any(|value| !value.is_finite()) {
                        return Err(AuthoringError::NonFiniteTransform);
                    }
                }
                let location = self.locate(id)?;
                let element =
                    &mut self.elements_mut(location.page, location.category)?[location.index];
                if let Some(value) = position {
                    element["objectData"]["position"] = vec3(value);
                }
                if let Some(value) = scale {
                    element["objectData"]["scale"] = vec3(value);
                }
                if let Some(value) = rotation {
                    element["objectData"]["rotation"] = quaternion(value);
                }
                Ok(vec![change(
                    id,
                    location.page,
                    location.category,
                    AuthoringChangeKind::Updated,
                )])
            }
            AuthoringCommand::SetLock { id, lock } => self.set_object_flag(id, "lock", lock),
            AuthoringCommand::SetVisible { id, visible } => {
                self.set_object_flag(id, "visible", visible)
            }
            AuthoringCommand::SetParameters { id, values } => {
                if values.contains_key("objectData") {
                    return Err(AuthoringError::ObjectDataParameterForbidden);
                }
                let location = self.locate(id)?;
                let element = self.elements_mut(location.page, location.category)?[location.index]
                    .as_object_mut()
                    .ok_or(AuthoringError::ElementMustBeObject)?;
                element.extend(values);
                Ok(vec![change(
                    id,
                    location.page,
                    location.category,
                    AuthoringChangeKind::Updated,
                )])
            }
            AuthoringCommand::ChangeLayer { id, layer } => {
                let location = self.locate(id)?;
                let count = self.page_element_count(location.page);
                if layer >= count {
                    return Err(AuthoringError::InvalidLayer {
                        page: location.page,
                        layer,
                        count,
                    });
                }
                let old_layer = self.elements(location.page, location.category)?[location.index]
                    ["objectData"]["layer"]
                    .as_u64()
                    .unwrap_or_default() as usize;
                for category in AuthoringCategory::ALL {
                    for element in self.elements_mut(location.page, category)? {
                        let current =
                            element["objectData"]["layer"].as_u64().unwrap_or_default() as usize;
                        let replacement =
                            if old_layer < layer && current > old_layer && current <= layer {
                                current - 1
                            } else if layer < old_layer && current >= layer && current < old_layer {
                                current + 1
                            } else {
                                current
                            };
                        element["objectData"]["layer"] = Value::from(replacement);
                    }
                }
                let location = self.locate(id)?;
                self.elements_mut(location.page, location.category)?[location.index]
                    ["objectData"]["layer"] = Value::from(layer);
                Ok(self
                    .element_ids(location.page)?
                    .into_iter()
                    .map(|changed_id| {
                        let changed = self.locate(changed_id).expect("indexed element");
                        change(
                            changed_id,
                            changed.page,
                            changed.category,
                            AuthoringChangeKind::Updated,
                        )
                    })
                    .collect())
            }
        }
    }

    fn set_object_flag(
        &mut self,
        id: AuthoringElementId,
        key: &str,
        value: bool,
    ) -> Result<Vec<AuthoringElementChange>, AuthoringError> {
        let location = self.locate(id)?;
        self.elements_mut(location.page, location.category)?[location.index]["objectData"][key] =
            Value::Bool(value);
        Ok(vec![change(
            id,
            location.page,
            location.category,
            AuthoringChangeKind::Updated,
        )])
    }

    fn ensure_no_gesture(&self) -> Result<(), AuthoringError> {
        if self.gesture.is_some() {
            Err(AuthoringError::GestureInProgress)
        } else {
            Ok(())
        }
    }

    fn allocate_page_ids(
        &mut self,
        page: usize,
    ) -> Result<BTreeMap<AuthoringCategory, Vec<AuthoringElementId>>, AuthoringError> {
        let counts = AuthoringCategory::ALL
            .into_iter()
            .map(|category| Ok((category, self.elements(page, category)?.len())))
            .collect::<Result<Vec<_>, AuthoringError>>()?;
        Ok(counts
            .into_iter()
            .map(|(category, count)| {
                let ids = (0..count).map(|_| self.allocate_id()).collect();
                (category, ids)
            })
            .collect())
    }

    fn record_document_change(
        &mut self,
        before: DocumentSnapshot,
        selection_before: Option<AuthoringElementId>,
        page_forward: Vec<AuthoringPageChange>,
        page_reverse: Vec<AuthoringPageChange>,
    ) -> Result<AuthoringDelta, AuthoringError> {
        self.document.validate()?;
        let after = self.snapshot_document();
        self.undo.push_back(HistoryEntry {
            before: HistorySnapshot::Document(before),
            after: HistorySnapshot::Document(after),
            forward: Vec::new(),
            reverse: Vec::new(),
            selection_before,
            selection_after: self.selected_id,
            page_forward: page_forward.clone(),
            page_reverse,
        });
        while self.undo.len() > AUTHORING_HISTORY_LIMIT {
            self.undo.pop_front();
        }
        self.redo.clear();
        Ok(self.finish(Vec::new(), page_forward))
    }

    fn ensure_capacity(&self, page: usize) -> Result<(), AuthoringError> {
        let count = self.page_element_count(page);
        if count >= GameProfileDocument::MAX_ELEMENTS_PER_PAGE {
            return Err(AuthoringError::ElementLimitReached {
                page,
                max: GameProfileDocument::MAX_ELEMENTS_PER_PAGE,
            });
        }
        Ok(())
    }

    fn page_element_count(&self, page: usize) -> usize {
        self.ids[page].values().map(Vec::len).sum()
    }

    fn elements(
        &self,
        page: usize,
        category: AuthoringCategory,
    ) -> Result<&Vec<Value>, AuthoringError> {
        self.document
            .pages()
            .get(page)
            .ok_or(AuthoringError::PageNotFound(page))?["customProfileCard"][category.key()]
        .as_array()
        .ok_or(AuthoringError::ElementMustBeObject)
    }

    fn elements_mut(
        &mut self,
        page: usize,
        category: AuthoringCategory,
    ) -> Result<&mut Vec<Value>, AuthoringError> {
        self.document
            .pages_mut()
            .get_mut(page)
            .ok_or(AuthoringError::PageNotFound(page))?["customProfileCard"][category.key()]
        .as_array_mut()
        .ok_or(AuthoringError::ElementMustBeObject)
    }

    fn locate(&self, id: AuthoringElementId) -> Result<ElementLocation, AuthoringError> {
        for (page_index, page) in self.ids.iter().enumerate() {
            for category in AuthoringCategory::ALL {
                if let Some(index) = page[&category]
                    .iter()
                    .position(|candidate| *candidate == id)
                {
                    return Ok(ElementLocation {
                        page: page_index,
                        category,
                        index,
                    });
                }
            }
        }
        Err(AuthoringError::ElementNotFound(id))
    }

    fn allocate_id(&mut self) -> AuthoringElementId {
        let id = AuthoringElementId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("authoring element id space exhausted");
        id
    }

    fn snapshot(&self, page: usize) -> Result<PageSnapshot, AuthoringError> {
        Ok(PageSnapshot {
            page,
            value: self
                .document
                .pages()
                .get(page)
                .ok_or(AuthoringError::PageNotFound(page))?
                .clone(),
            ids: self
                .ids
                .get(page)
                .ok_or(AuthoringError::PageNotFound(page))?
                .clone(),
        })
    }

    fn restore_page(&mut self, snapshot: &PageSnapshot) -> Result<(), AuthoringError> {
        *self
            .document
            .pages_mut()
            .get_mut(snapshot.page)
            .ok_or(AuthoringError::PageNotFound(snapshot.page))? = snapshot.value.clone();
        self.ids[snapshot.page] = snapshot.ids.clone();
        self.document.validate()?;
        validate_ids_for_document(&self.document, &self.ids)?;
        Ok(())
    }

    fn snapshot_document(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            value: self.document.export_value(),
            ids: self.ids.clone(),
        }
    }

    fn restore_history(&mut self, snapshot: &HistorySnapshot) -> Result<(), AuthoringError> {
        match snapshot {
            HistorySnapshot::Page(page) => self.restore_page(page),
            HistorySnapshot::Document(document) => {
                self.document = GameProfileDocument::from_export_value(document.value.clone())?;
                self.ids = document.ids.clone();
                validate_ids_for_document(&self.document, &self.ids)?;
                Ok(())
            }
        }
    }

    fn validate_current_state(&self) -> Result<u32, AuthoringError> {
        self.document.validate()?;
        let maximum_id = validate_ids_for_document(&self.document, &self.ids)?;
        self.validate_selected_id()?;
        Ok(maximum_id)
    }

    fn validate_selected_id(&self) -> Result<(), AuthoringError> {
        if self.selected_id.is_some_and(|id| self.locate(id).is_err()) {
            return Err(AuthoringError::InvalidCheckpoint(
                "selectedId does not exist in the restored document".into(),
            ));
        }
        Ok(())
    }

    fn normalize_layers(&mut self, page: usize) -> Result<(), AuthoringError> {
        let mut locations = Vec::new();
        for category in AuthoringCategory::ALL {
            for index in 0..self.elements(page, category)?.len() {
                let layer = self.elements(page, category)?[index]["objectData"]["layer"]
                    .as_u64()
                    .unwrap_or_default();
                locations.push((layer, category, index));
            }
        }
        locations.sort_by_key(|(layer, _, _)| *layer);
        for (layer, (_, category, index)) in locations.into_iter().enumerate() {
            self.elements_mut(page, category)?[index]["objectData"]["layer"] = Value::from(layer);
        }
        Ok(())
    }

    fn finish(
        &mut self,
        changes: Vec<AuthoringElementChange>,
        page_changes: Vec<AuthoringPageChange>,
    ) -> AuthoringDelta {
        self.revision += 1;
        self.delta_with_pages(changes, page_changes)
    }

    fn delta(&self, changes: Vec<AuthoringElementChange>, _committed: bool) -> AuthoringDelta {
        self.delta_with_pages(changes, Vec::new())
    }

    fn delta_with_pages(
        &self,
        mut changes: Vec<AuthoringElementChange>,
        page_changes: Vec<AuthoringPageChange>,
    ) -> AuthoringDelta {
        for change in &mut changes {
            change.element = self
                .locate(change.id)
                .ok()
                .and_then(|location| {
                    self.elements(location.page, location.category)
                        .ok()
                        .and_then(|elements| elements.get(location.index))
                })
                .cloned();
        }
        let selected = self.selected_id.and_then(|id| {
            let location = self.locate(id).ok()?;
            let element = self
                .elements(location.page, location.category)
                .ok()?
                .get(location.index)?
                .clone();
            Some(AuthoringSelection {
                id,
                page: location.page,
                category: location.category,
                index: location.index,
                element,
            })
        });
        AuthoringDelta {
            revision: self.revision,
            changes,
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            selected_id: self.selected_id,
            selected,
            page_changes,
        }
    }
}

fn validate_history_snapshot(snapshot: &HistorySnapshot) -> Result<u32, AuthoringError> {
    match snapshot {
        HistorySnapshot::Page(page) => {
            let document = GameProfileDocument::from_export_value(serde_json::json!({
                "userCustomProfileCards": [page.value.clone()]
            }))?;
            validate_ids_for_document(&document, std::slice::from_ref(&page.ids))
        }
        HistorySnapshot::Document(document) => {
            let value = GameProfileDocument::from_export_value(document.value.clone())?;
            validate_ids_for_document(&value, &document.ids)
        }
    }
}

fn validate_ids_for_document(
    document: &GameProfileDocument,
    ids: &[BTreeMap<AuthoringCategory, Vec<AuthoringElementId>>],
) -> Result<u32, AuthoringError> {
    if ids.len() != document.page_count() {
        return Err(AuthoringError::InvalidCheckpoint(format!(
            "ID table has {} pages for a {} page document",
            ids.len(),
            document.page_count()
        )));
    }
    let mut seen = BTreeSet::new();
    let mut maximum_id = 0;
    for (page_index, page_ids) in ids.iter().enumerate() {
        if page_ids.len() != AuthoringCategory::ALL.len() {
            return Err(AuthoringError::InvalidCheckpoint(format!(
                "page {page_index} ID table does not contain every authoring category"
            )));
        }
        for category in AuthoringCategory::ALL {
            let category_ids = page_ids.get(&category).ok_or_else(|| {
                AuthoringError::InvalidCheckpoint(format!(
                    "page {page_index} is missing {} IDs",
                    category.key()
                ))
            })?;
            let element_count = document.pages()[page_index]["customProfileCard"][category.key()]
                .as_array()
                .expect("validated document")
                .len();
            if category_ids.len() != element_count {
                return Err(AuthoringError::InvalidCheckpoint(format!(
                    "page {page_index} {} has {} IDs for {element_count} elements",
                    category.key(),
                    category_ids.len()
                )));
            }
            for id in category_ids {
                if id.0 == 0 {
                    return Err(AuthoringError::InvalidCheckpoint(
                        "element IDs must be positive".into(),
                    ));
                }
                if !seen.insert(*id) {
                    return Err(AuthoringError::InvalidCheckpoint(format!(
                        "duplicate element ID {}",
                        id.0
                    )));
                }
                maximum_id = maximum_id.max(id.0);
            }
        }
    }
    Ok(maximum_id)
}

fn empty_page_ids() -> BTreeMap<AuthoringCategory, Vec<AuthoringElementId>> {
    AuthoringCategory::ALL
        .into_iter()
        .map(|category| (category, Vec::new()))
        .collect()
}

fn validate_element(element: &Value) -> Result<(), AuthoringError> {
    let object = element
        .as_object()
        .ok_or(AuthoringError::ElementMustBeObject)?;
    let data = object
        .get("objectData")
        .and_then(Value::as_object)
        .ok_or(AuthoringError::InvalidObjectData)?;
    for key in ["position", "scale", "rotation"] {
        if !data.get(key).is_some_and(Value::is_object) {
            return Err(AuthoringError::InvalidObjectData);
        }
    }
    if data.get("layer").and_then(Value::as_i64).is_none()
        || data.get("lock").and_then(Value::as_bool).is_none()
        || data.get("visible").and_then(Value::as_bool).is_none()
    {
        return Err(AuthoringError::InvalidObjectData);
    }
    Ok(())
}

fn offset_position(element: &mut Value, dx: f64, dy: f64) -> Result<(), AuthoringError> {
    let position = element["objectData"]["position"]
        .as_object_mut()
        .ok_or(AuthoringError::InvalidObjectData)?;
    let x = position
        .get("x")
        .and_then(Value::as_f64)
        .ok_or(AuthoringError::InvalidObjectData)?;
    let y = position
        .get("y")
        .and_then(Value::as_f64)
        .ok_or(AuthoringError::InvalidObjectData)?;
    position.insert("x".into(), Value::from(x + dx));
    position.insert("y".into(), Value::from(y + dy));
    Ok(())
}

fn vec3(value: [f32; 3]) -> Value {
    Value::Object(Map::from_iter([
        ("x".into(), Value::from(value[0])),
        ("y".into(), Value::from(value[1])),
        ("z".into(), Value::from(value[2])),
    ]))
}

fn quaternion(value: [f32; 4]) -> Value {
    Value::Object(Map::from_iter([
        ("x".into(), Value::from(value[0])),
        ("y".into(), Value::from(value[1])),
        ("z".into(), Value::from(value[2])),
        ("w".into(), Value::from(value[3])),
    ]))
}

fn change(
    id: AuthoringElementId,
    page: usize,
    category: AuthoringCategory,
    kind: AuthoringChangeKind,
) -> AuthoringElementChange {
    AuthoringElementChange {
        id,
        page,
        category,
        kind,
        element: None,
    }
}

fn reverse_changes(changes: &[AuthoringElementChange]) -> Vec<AuthoringElementChange> {
    changes
        .iter()
        .map(|change| AuthoringElementChange {
            kind: match change.kind {
                AuthoringChangeKind::Inserted => AuthoringChangeKind::Removed,
                AuthoringChangeKind::Updated => AuthoringChangeKind::Updated,
                AuthoringChangeKind::Removed => AuthoringChangeKind::Inserted,
            },
            element: None,
            ..change.clone()
        })
        .collect()
}

#[cfg(test)]
mod tests {
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
        let values =
            &session.export_value()["userCustomProfileCards"][0]["customProfileCard"]["texts"];
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
}
