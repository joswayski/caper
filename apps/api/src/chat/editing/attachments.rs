//! Editing a sent message's files: which file each slot keeps, replaces or
//! restores, and the edit description stored with a replacement. These checks
//! are pure and tested without a database; `persist_edit` applies the plan in
//! its revision transaction (docs/media.md, "Editing attachments after sending").
use crate::{ApiError, assets::MAX_PER_MESSAGE};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Limits shared with every editor through `shared/messages/edit-cases.json`.
pub(crate) const MAX_MARKS: usize = 500;
pub(crate) const MAX_POINT_VALUES: usize = 100_000;
pub(crate) const MAX_TEXT_CHARACTERS: usize = 200;
/// Ten descriptions this size still fit under axum's 2 MiB JSON body limit.
pub(crate) const MAX_EDIT_BYTES: usize = 128 * 1024;
/// Editors round fractions to a millionth, so a crop may end slightly past an
/// edge; this leaves room for that and for floating-point sums.
const CROP_TOLERANCE: f64 = 1e-5;

/// One entry of an edit's `attachments`, in display order.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AttachmentChange {
    /// The file shown in this slot after the edit.
    pub id: String,
    /// A missing spoiler is false, as when sending.
    #[serde(default)]
    pub spoiler: bool,
    /// The slot's current file, when `id` takes its place.
    pub replaces: Option<String>,
    /// How a new upload was rendered from the slot's first-sent file.
    pub edit: Option<EditDescription>,
}

/// The pre-send editor's model (`apps/web/src/chat/edit.ts`), in the
/// coordinates of the slot's first-sent file. Stored with the render and shown
/// only to its author, never in the message or its versions.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditDescription {
    rotation: u16,
    crop: Crop,
    marks: Vec<Mark>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Crop {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Mark {
    Stroke {
        color: String,
        width: f64,
        points: Vec<f64>,
    },
    Text {
        color: String,
        size: f64,
        x: f64,
        y: f64,
        text: String,
    },
}

/// Why an edit's file list is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    TooMany,
    Mismatch,
    EditWithoutReplacement,
    MissingEdit,
    RestoreWithEdit,
    NotEditable,
    Unavailable,
    WrongKind,
    InvalidEdit,
    EditTooLarge,
}

impl From<Refusal> for ApiError {
    fn from(refusal: Refusal) -> Self {
        let bad = |message| ApiError::new(StatusCode::BAD_REQUEST, message);
        match refusal {
            Refusal::TooMany => bad("edit up to 10 different files"),
            Refusal::Mismatch => bad("list the message's files in their current order"),
            Refusal::EditWithoutReplacement => bad("only a replacement file has an edit"),
            Refusal::MissingEdit => bad("a replacement file needs its edit"),
            Refusal::RestoreWithEdit => bad("a restored file has no edit"),
            Refusal::NotEditable => bad("only photos and videos can be replaced"),
            Refusal::Unavailable => {
                ApiError::new(StatusCode::CONFLICT, "this file can no longer be edited")
            }
            Refusal::WrongKind => bad("a replacement must be the same kind of file"),
            Refusal::InvalidEdit => bad("invalid attachment edit"),
            Refusal::EditTooLarge => {
                bad("attachment edit is too large").with_code("edit_too_large")
            }
        }
    }
}

fn fraction(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= 1.0
}

fn color(value: &str) -> bool {
    value
        .strip_prefix('#')
        .is_some_and(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

impl EditDescription {
    pub(crate) fn check(&self) -> Result<(), Refusal> {
        if self.marks.len() > MAX_MARKS {
            return Err(Refusal::EditTooLarge);
        }
        let points: usize = self
            .marks
            .iter()
            .map(|mark| match mark {
                Mark::Stroke { points, .. } => points.len(),
                Mark::Text { .. } => 0,
            })
            .sum();
        if points > MAX_POINT_VALUES {
            return Err(Refusal::EditTooLarge);
        }
        let Crop {
            x,
            y,
            width,
            height,
        } = self.crop;
        let crop = [x, y, width, height].iter().all(|v| v.is_finite())
            && x >= 0.0
            && y >= 0.0
            && width > 0.0
            && height > 0.0
            && x + width <= 1.0 + CROP_TOLERANCE
            && y + height <= 1.0 + CROP_TOLERANCE;
        let marks = self.marks.iter().all(|mark| match mark {
            Mark::Stroke {
                color: c,
                width,
                points,
            } => {
                color(c)
                    && fraction(*width)
                    && points.len() >= 2
                    && points.len() % 2 == 0
                    && points.iter().all(|v| v.is_finite())
            }
            Mark::Text {
                color: c,
                size,
                x,
                y,
                text,
            } => {
                color(c)
                    && fraction(*size)
                    && x.is_finite()
                    && y.is_finite()
                    && (1..=MAX_TEXT_CHARACTERS).contains(&text.chars().count())
                    && !text.trim().is_empty()
                    && !text.chars().any(char::is_control)
            }
        });
        if !matches!(self.rotation, 0 | 90 | 180 | 270) || !crop || !marks {
            return Err(Refusal::InvalidEdit);
        }
        // What is stored, so every edit is measured the same way.
        if serde_json::to_vec(self).map_or(true, |json| json.len() > MAX_EDIT_BYTES) {
            return Err(Refusal::EditTooLarge);
        }
        Ok(())
    }
}

/// Checks that do not depend on the message's current files, made before any
/// database work.
pub(crate) fn check_request(changes: &[AttachmentChange]) -> Result<(), Refusal> {
    if changes.len() > MAX_PER_MESSAGE
        || changes.iter().enumerate().any(|(i, change)| {
            changes[..i].iter().any(|earlier| {
                earlier.id == change.id
                    || (earlier.replaces.is_some() && earlier.replaces == change.replaces)
            })
        })
    {
        return Err(Refusal::TooMany);
    }
    for change in changes {
        if change.replaces.as_ref() == Some(&change.id) {
            return Err(Refusal::Mismatch);
        }
        if let Some(edit) = &change.edit {
            if change.replaces.is_none() {
                return Err(Refusal::EditWithoutReplacement);
            }
            edit.check()?;
        }
    }
    Ok(())
}

/// The files a message's content shows, as `(id, spoiler)` in display order.
pub(crate) fn shown(attachments: Option<&Value>) -> Vec<(&str, bool)> {
    attachments
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|file| {
            (
                file["id"].as_str().unwrap_or_default(),
                file["spoiler"] == true,
            )
        })
        .collect()
}

/// The files an edit asks the message to show, as `(id, spoiler)`.
pub(crate) fn requested(changes: &[AttachmentChange]) -> Vec<(&str, bool)> {
    changes
        .iter()
        .map(|change| (change.id.as_str(), change.spoiler))
        .collect()
}

/// A file the message shows now, with what the database knows about it.
#[derive(Clone, Debug)]
pub(crate) struct CurrentFile {
    pub id: String,
    pub kind: String,
    pub spoiler: bool,
    /// The first-sent file this one was rendered from; `None` when it is the
    /// first-sent file itself.
    pub source: Option<String>,
    /// Not deleted.
    pub available: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Change {
    Keep,
    /// The slot's first-sent file returns ("undo all edits"); no upload.
    Restore {
        retire: String,
    },
    /// A new upload rendered from the slot's first-sent file.
    Upload {
        retire: String,
        source: String,
        kind: String,
        edit: EditDescription,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlannedFile {
    pub id: String,
    pub spoiler: bool,
    pub change: Change,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Plan {
    pub files: Vec<PlannedFile>,
}

impl Plan {
    pub(crate) fn uploads(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|file| matches!(file.change, Change::Upload { .. }))
            .map(|file| file.id.clone())
            .collect()
    }

    pub(crate) fn restores(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|file| matches!(file.change, Change::Restore { .. }))
            .map(|file| file.id.clone())
            .collect()
    }

    /// Replaced files: they stay stored and linked, visible to the author only.
    pub(crate) fn retired(&self) -> Vec<String> {
        self.files
            .iter()
            .filter_map(|file| match &file.change {
                Change::Keep => None,
                Change::Restore { retire } | Change::Upload { retire, .. } => Some(retire.clone()),
            })
            .collect()
    }

    /// Whether the message's `(id, spoiler)` list changes. A spoiler toggle
    /// alone is an edit too.
    pub(crate) fn changes(&self, current: &[CurrentFile]) -> bool {
        self.files.len() != current.len()
            || self
                .files
                .iter()
                .zip(current)
                .any(|(file, now)| file.id != now.id || file.spoiler != now.spoiler)
    }
}

/// Matches each entry to the file in the same slot. The list keeps the
/// message's length and order: a file is kept (same id), replaced by a new
/// upload, or (when it was itself a replacement) replaced by the slot's
/// first-sent file again.
pub(crate) fn plan(current: &[CurrentFile], changes: &[AttachmentChange]) -> Result<Plan, Refusal> {
    // Removing files would allow leaving a current file out here, retiring it
    // like a replaced one, while still requiring text or at least one file.
    if changes.len() != current.len() {
        return Err(Refusal::Mismatch);
    }
    let mut files = Vec::with_capacity(changes.len());
    for (now, change) in current.iter().zip(changes) {
        let planned = match &change.replaces {
            None if change.id == now.id => Change::Keep,
            None => return Err(Refusal::Mismatch),
            Some(replaced) if *replaced != now.id => return Err(Refusal::Mismatch),
            Some(_) => {
                if !matches!(now.kind.as_str(), "image" | "video") {
                    return Err(Refusal::NotEditable);
                }
                if !now.available {
                    return Err(Refusal::Unavailable);
                }
                let first = now.source.clone().unwrap_or_else(|| now.id.clone());
                if change.id == first {
                    if change.edit.is_some() {
                        return Err(Refusal::RestoreWithEdit);
                    }
                    Change::Restore {
                        retire: now.id.clone(),
                    }
                } else if current.iter().any(|file| file.id == change.id) {
                    return Err(Refusal::Mismatch);
                } else {
                    Change::Upload {
                        retire: now.id.clone(),
                        source: first,
                        kind: now.kind.clone(),
                        edit: change.edit.clone().ok_or(Refusal::MissingEdit)?,
                    }
                }
            }
        };
        files.push(PlannedFile {
            id: change.id.clone(),
            spoiler: change.spoiler,
            change: planned,
        });
    }
    Ok(Plan { files })
}

#[cfg(test)]
mod tests;
