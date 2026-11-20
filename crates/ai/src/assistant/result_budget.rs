//! Fits domain tool results into the active model profile's input budget.
//!
//! Executors return bounded pages, but a page that is harmless for one model
//! profile can still overflow a smaller input window, and several pages in one
//! turn accumulate. Instead of failing the turn, the assistant shrinks the
//! model-visible message: it removes trailing rows from the largest top-level
//! arrays, keeps page metadata truthful, narrows read citations to the rows
//! that remain and adds an explicit marker telling the model to refine its
//! query. Write drafts are never altered; only their model-visible preview is
//! shortened, so a human still reviews the complete draft.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::{AssistantError, Citation};

const TRUNCATION_REASON: &str = "model_input_budget";
const COMPACTION_REASON: &str = "compacted_for_input_budget";
const READ_TRUNCATION_NOTICE: &str = "Partial result: MuriArc removed records so this tool result \
     fits the model input budget; refine the query before relying on it. The records shown are \
     not the complete set, so do not infer totals, counts, or absence from them. Use \
     animal_statistics (when available) for counts and distributions, add narrower filters, or \
     continue from next_offset with a smaller limit.";
const DRAFT_TRUNCATION_NOTICE: &str = "The model view of this reviewable draft was shortened to \
     fit the model input budget. The complete draft, including every omitted change, is preserved \
     for human review; do not restate or reconstruct the omitted changes.";
const COMPACTION_NOTICE: &str = "This earlier tool result was removed from the model context to \
     make room for newer results within the input token budget. Its data is no longer visible; \
     refine the query and call the tool again only if it is still needed.";

/// Limits for one model-visible tool message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ResultBudget {
    /// Maximum serialized UTF-8 bytes of the tool message content.
    pub max_bytes: usize,
    /// Maximum citations that are not already recorded for this turn.
    pub max_new_citations: usize,
}

pub(super) struct FittedRead {
    pub message: String,
    pub citations: Vec<Citation>,
    pub truncated: bool,
}

pub(super) struct FittedDraftView {
    pub message: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OmittedArray {
    pointer: String,
    kept: usize,
    omitted: usize,
}

pub(super) fn fit_read(
    data: Value,
    citations: Vec<Citation>,
    recorded: &[Citation],
    budget: ResultBudget,
) -> Result<FittedRead, AssistantError> {
    let full = json!({
        "kind": "read_result",
        "data": data,
        "citations": citations,
    });
    let message = to_message(&full)?;
    if message.len() <= budget.max_bytes
        && new_citation_count(&citations, recorded) <= budget.max_new_citations
    {
        return Ok(FittedRead {
            message,
            citations,
            truncated: false,
        });
    }

    let Value::Object(mut full) = full else {
        unreachable!("the read payload is constructed as an object")
    };
    let mut data = full.remove("data").unwrap_or(Value::Null);
    let mut omitted = Vec::new();
    let render = |data: &Value, omitted: &[OmittedArray]| {
        let kept = referenced_citations(&citations, data);
        let message = to_message(&truncated_read(data, &kept, omitted))?;
        let fits = message.len() <= budget.max_bytes
            && new_citation_count(&kept, recorded) <= budget.max_new_citations;
        Ok::<_, AssistantError>((message, kept, fits))
    };
    loop {
        let (message, kept, fits) = render(&data, &omitted)?;
        if fits {
            return Ok(FittedRead {
                message,
                citations: kept,
                truncated: true,
            });
        }
        let shrunk =
            shrink_largest_array(&mut data, "/data", None, &mut omitted, |data, omitted| {
                render(data, omitted).is_ok_and(|(_, _, fits)| fits)
            });
        if shrunk == Shrink::NothingLeft {
            break;
        }
    }
    Ok(FittedRead {
        message: to_message(&truncated_read(&Value::Null, &[], &omitted))?,
        citations: Vec::new(),
        truncated: true,
    })
}

/// Bounds the model-visible view of a write draft. The draft itself, its
/// persisted changes and its citations are unaffected.
pub(super) fn fit_draft_view(
    payload: Value,
    max_bytes: usize,
) -> Result<FittedDraftView, AssistantError> {
    let message = to_message(&payload)?;
    if message.len() <= max_bytes {
        return Ok(FittedDraftView {
            message,
            truncated: false,
        });
    }
    let mut payload = payload;
    let mut omitted = Vec::new();
    let fits = |payload: &Value, omitted: &[OmittedArray]| {
        to_message(&truncated_draft(payload, omitted))
            .is_ok_and(|message| message.len() <= max_bytes)
    };
    while !fits(&payload, &omitted)
        && shrink_largest_array(
            &mut payload,
            "",
            Some(&["changes", "citations"]),
            &mut omitted,
            fits,
        ) != Shrink::NothingLeft
    {}
    Ok(FittedDraftView {
        message: to_message(&truncated_draft(&payload, &omitted))?,
        truncated: true,
    })
}

/// Replaces an earlier tool message with a short marker. Returns `None` when
/// the message is already compacted.
pub(super) fn compacted_tool_message(content: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(content).unwrap_or(Value::Null);
    if parsed.pointer("/truncation/reason") == Some(&Value::from(COMPACTION_REASON)) {
        return None;
    }
    let kind = parsed
        .get("kind")
        .and_then(Value::as_str)
        .filter(|kind| *kind == "write_draft")
        .unwrap_or("read_result");
    let mut compacted = json!({
        "kind": kind,
        "truncated": true,
        "truncation": {
            "reason": COMPACTION_REASON,
            "notice": COMPACTION_NOTICE,
        },
    });
    if kind == "write_draft" {
        if let Some(draft_id) = parsed.get("draft_id") {
            compacted["draft_id"] = draft_id.clone();
        }
    } else {
        compacted["data"] = Value::Null;
    }
    Some(compacted.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shrink {
    /// The chosen array now keeps the most rows that still fit.
    Fits,
    /// Even an empty chosen array does not fit; try the next array.
    Emptied,
    /// No non-empty array remains to shrink.
    NothingLeft,
}

fn truncated_read(data: &Value, citations: &[Citation], omitted: &[OmittedArray]) -> Value {
    json!({
        "kind": "read_result",
        "truncated": true,
        "truncation": truncation_marker(READ_TRUNCATION_NOTICE, omitted),
        "data": data,
        "citations": citations,
    })
}

fn truncated_draft(payload: &Value, omitted: &[OmittedArray]) -> Value {
    let mut payload = payload.clone();
    if let Value::Object(object) = &mut payload {
        object.insert("truncated".to_owned(), Value::Bool(true));
        object.insert(
            "truncation".to_owned(),
            truncation_marker(DRAFT_TRUNCATION_NOTICE, omitted),
        );
    }
    payload
}

fn truncation_marker(notice: &str, omitted: &[OmittedArray]) -> Value {
    json!({
        "reason": TRUNCATION_REASON,
        "omitted": omitted
            .iter()
            .map(|item| json!({
                "path": item.pointer,
                "kept": item.kept,
                "omitted": item.omitted,
            }))
            .collect::<Vec<_>>(),
        "notice": notice,
    })
}

fn to_message(value: &Value) -> Result<String, AssistantError> {
    serde_json::to_string(value).map_err(|_| AssistantError::UnsafeToolOutput)
}

/// Mirrors `append_citations`: only distinct citations not yet recorded for
/// this turn consume the turn's citation capacity.
fn new_citation_count(citations: &[Citation], recorded: &[Citation]) -> usize {
    let mut seen: Vec<&Citation> = Vec::new();
    for citation in citations {
        if !recorded.contains(citation) && !seen.contains(&citation) {
            seen.push(citation);
        }
    }
    seen.len()
}

/// Keeps citations whose entity identifier is still present in the shrunken
/// data, so a partial result never claims sources the model did not receive.
fn referenced_citations(citations: &[Citation], data: &Value) -> Vec<Citation> {
    let mut identifiers = BTreeSet::new();
    collect_uuid_strings(data, &mut identifiers);
    citations
        .iter()
        .filter(|citation| identifiers.contains(&citation.entity_id))
        .cloned()
        .collect()
}

fn collect_uuid_strings(value: &Value, identifiers: &mut BTreeSet<Uuid>) {
    match value {
        Value::String(text) if text.len() == 36 => {
            if let Ok(identifier) = Uuid::parse_str(text) {
                identifiers.insert(identifier);
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect_uuid_strings(item, identifiers)),
        Value::Object(object) => object
            .values()
            .for_each(|item| collect_uuid_strings(item, identifiers)),
        _ => {}
    }
}

/// Shrinks the largest non-empty array that is not nested inside another
/// array, keeping the longest prefix of rows for which `fits` holds. Removing
/// rows only ever shrinks a message, so a binary search finds that prefix.
fn shrink_largest_array(
    root: &mut Value,
    pointer_prefix: &str,
    allowed_top_level_keys: Option<&[&str]>,
    omitted: &mut Vec<OmittedArray>,
    fits: impl Fn(&Value, &[OmittedArray]) -> bool,
) -> Shrink {
    let mut candidates = Vec::new();
    match allowed_top_level_keys {
        Some(keys) => {
            for key in keys {
                if let Some(value @ Value::Array(items)) = root.get(*key)
                    && !items.is_empty()
                {
                    candidates.push((vec![(*key).to_owned()], serialized_len(value)));
                }
            }
        }
        None => collect_array_paths(root, &mut Vec::new(), &mut candidates),
    }
    let Some((path, _)) = candidates.into_iter().max_by_key(|(_, size)| *size) else {
        return Shrink::NothingLeft;
    };
    let Some(original) = value_at(root, &path)
        .and_then(Value::as_array)
        .map(Vec::len)
    else {
        return Shrink::NothingLeft;
    };
    let pointer = format!(
        "{pointer_prefix}{}",
        path.iter()
            .map(|key| format!("/{}", key.replace('~', "~0").replace('/', "~1")))
            .collect::<String>()
    );
    let keep = |kept: usize| {
        let mut candidate = root.clone();
        truncate_array(&mut candidate, &path, kept);
        let mut candidate_omitted = omitted.clone();
        record_omitted(&mut candidate_omitted, &pointer, original, kept);
        (candidate, candidate_omitted)
    };

    let (empty, empty_omitted) = keep(0);
    if !fits(&empty, &empty_omitted) {
        *root = empty;
        *omitted = empty_omitted;
        return Shrink::Emptied;
    }
    let (mut low, mut high) = (0, original - 1);
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        let (candidate, candidate_omitted) = keep(middle);
        if fits(&candidate, &candidate_omitted) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let (shrunk, shrunk_omitted) = keep(low);
    *root = shrunk;
    *omitted = shrunk_omitted;
    Shrink::Fits
}

fn truncate_array(root: &mut Value, path: &[String], kept: usize) {
    if let Some(Value::Array(items)) = value_at_mut(root, path) {
        items.truncate(kept);
    }
    if path.last().is_some_and(|key| key == "items")
        && let Some(Value::Object(parent)) = value_at_mut(root, &path[..path.len() - 1])
        && let Some(Value::Object(page)) = parent.get_mut("page")
    {
        mark_partial_page(page, kept);
    }
}

fn record_omitted(omitted: &mut Vec<OmittedArray>, pointer: &str, original: usize, kept: usize) {
    let removed = original - kept;
    match omitted.iter_mut().find(|item| item.pointer == pointer) {
        Some(item) => {
            item.kept = kept;
            item.omitted += removed;
        }
        None => omitted.push(OmittedArray {
            pointer: pointer.to_owned(),
            kept,
            omitted: removed,
        }),
    }
}

fn collect_array_paths(
    value: &Value,
    path: &mut Vec<String>,
    candidates: &mut Vec<(Vec<String>, usize)>,
) {
    match value {
        Value::Array(items) if !items.is_empty() => {
            candidates.push((path.clone(), serialized_len(value)));
        }
        Value::Object(object) => {
            for (key, child) in object {
                path.push(key.clone());
                collect_array_paths(child, path, candidates);
                path.pop();
            }
        }
        _ => {}
    }
}

fn value_at<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter()
        .try_fold(root, |value, key| value.as_object()?.get(key))
}

fn value_at_mut<'a>(root: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    path.iter()
        .try_fold(root, |value, key| value.as_object_mut()?.get_mut(key))
}

/// A truncated page always has more rows: describe exactly what the model
/// received and where to continue.
fn mark_partial_page(page: &mut Map<String, Value>, kept: usize) {
    if page.contains_key("returned") {
        page.insert("returned".to_owned(), Value::from(kept));
    }
    if page.contains_key("complete") {
        page.insert("complete".to_owned(), Value::Bool(false));
    }
    if page.contains_key("has_more") {
        page.insert("has_more".to_owned(), Value::Bool(true));
    }
    if let Some(offset) = page.get("offset").and_then(Value::as_u64) {
        page.insert(
            "next_offset".to_owned(),
            Value::from(offset.saturating_add(kept as u64)),
        );
    }
}

fn serialized_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(0, |bytes| bytes.len())
}
