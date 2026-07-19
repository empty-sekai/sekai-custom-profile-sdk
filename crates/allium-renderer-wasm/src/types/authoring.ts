export type AuthoringElementId = number;

export const AUTHORING_CHECKPOINT_SCHEMA = "allium.renderer-authoring-checkpoint/v1" as const;

export type AuthoringCategory =
  | "bondsHonors"
  | "cardMembers"
  | "collections"
  | "generalBackgrounds"
  | "generals"
  | "honors"
  | "others"
  | "shapes"
  | "stamps"
  | "standMembers"
  | "storyBackgrounds"
  | "texts";

export type AuthoringCommand =
  | { kind: "create"; page: number; category: AuthoringCategory; element: Record<string, unknown> }
  | { kind: "duplicate"; id: AuthoringElementId }
  | { kind: "delete"; id: AuthoringElementId }
  | {
      kind: "set_transform";
      id: AuthoringElementId;
      position?: [number, number, number] | null;
      scale?: [number, number, number] | null;
      rotation?: [number, number, number, number] | null;
    }
  | { kind: "set_lock"; id: AuthoringElementId; lock: boolean }
  | { kind: "set_visible"; id: AuthoringElementId; visible: boolean }
  | { kind: "set_parameters"; id: AuthoringElementId; values: Record<string, unknown> }
  | { kind: "change_layer"; id: AuthoringElementId; layer: number };

export type AuthoringChangeKind = "inserted" | "updated" | "removed";

export type AuthoringElementChange = {
  id: AuthoringElementId;
  page: number;
  category: AuthoringCategory;
  kind: AuthoringChangeKind;
  element: Record<string, unknown> | null;
};

export type AuthoringSelection = {
  id: AuthoringElementId;
  page: number;
  category: AuthoringCategory;
  index: number;
  element: Record<string, unknown>;
};

export type AuthoringDelta = {
  revision: number;
  changes: AuthoringElementChange[];
  canUndo: boolean;
  canRedo: boolean;
  selectedId: AuthoringElementId | null;
  selected: AuthoringSelection | null;
  pageChanges: AuthoringPageChange[];
};

export type AuthoringPageChangeKind = "inserted" | "removed" | "moved";

export type AuthoringPageChange = {
  kind: AuthoringPageChangeKind;
  page: number;
  fromPage: number | null;
};

export type GameProfileDocument = {
  userCustomProfileCards: Array<Record<string, unknown>>;
};

/** Opaque renderer-owned history payload. Callers may persist it but must not edit its internals. */
export type AuthoringCheckpoint = {
  schema: typeof AUTHORING_CHECKPOINT_SCHEMA;
  document: GameProfileDocument;
  ids: unknown[];
  nextId: number;
  revision: number;
  undo: unknown[];
  redo: unknown[];
  selectedId: AuthoringElementId | null;
};
