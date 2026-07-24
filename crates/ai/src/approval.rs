use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::ToolName;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftKind {
    OrdinaryWrite,
    MeasurementResult,
    ResearchPlan,
    BulkImport,
    SoftDelete,
    PermissionChange,
    Migration,
}

impl DraftKind {
    pub const fn approval_requirement(self) -> ApprovalRequirement {
        match self {
            Self::OrdinaryWrite => ApprovalRequirement::PreviewConfirmation,
            Self::MeasurementResult | Self::ResearchPlan => {
                ApprovalRequirement::ResearcherSignature
            }
            Self::BulkImport | Self::SoftDelete | Self::PermissionChange | Self::Migration => {
                ApprovalRequirement::ReinforcedConfirmation
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequirement {
    PreviewConfirmation,
    ResearcherSignature,
    ReinforcedConfirmation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftStatus {
    PendingApproval,
    Approved,
    Rejected,
    Applied,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalActor {
    Human {
        user_id: Uuid,
    },
    Ai {
        /// The user whose authority the model is operating under.
        user_id: Uuid,
        tool_run_id: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanApprover {
    pub user_id: Uuid,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldChange {
    /// JSON Pointer identifying the changed field.
    pub path: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub decision: ApprovalDecision,
    pub approver: HumanApprover,
    pub statement: Option<String>,
    pub step_up_verified: bool,
    pub decided_at: DateTime<Utc>,
    pub draft_revision: u64,
}

/// A serializable mutation proposal that cannot apply itself.
///
/// Application code must persist this draft, obtain a human decision, then call
/// mark_applied in the same transaction as the actual domain write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WriteDraft {
    id: Uuid,
    kind: DraftKind,
    tool: ToolName,
    proposed_by: ProposalActor,
    project_id: Option<Uuid>,
    changes: Vec<FieldChange>,
    payload: Value,
    requirement: ApprovalRequirement,
    status: DraftStatus,
    revision: u64,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    decisions: Vec<ApprovalRecord>,
}

impl WriteDraft {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: DraftKind,
        tool: ToolName,
        proposed_by: ProposalActor,
        project_id: Option<Uuid>,
        changes: Vec<FieldChange>,
        payload: Value,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Result<Self, ApprovalError> {
        if expires_at <= created_at {
            return Err(ApprovalError::InvalidExpiration);
        }
        if changes.is_empty() && payload.is_null() {
            return Err(ApprovalError::EmptyDraft);
        }
        if matches!(proposed_by, ProposalActor::Ai { .. })
            && matches!(kind, DraftKind::PermissionChange | DraftKind::Migration)
        {
            return Err(ApprovalError::AiCannotProposeKind { kind });
        }

        ensure_no_raw_sql(&payload, "/payload")?;
        for change in &changes {
            validate_change(change)?;
        }

        Ok(Self {
            id: Uuid::new_v4(),
            kind,
            tool,
            proposed_by,
            project_id,
            changes,
            payload,
            requirement: kind.approval_requirement(),
            status: DraftStatus::PendingApproval,
            revision: 1,
            created_at,
            expires_at,
            decisions: Vec::new(),
        })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn kind(&self) -> DraftKind {
        self.kind
    }

    pub fn tool(&self) -> ToolName {
        self.tool
    }

    pub fn proposed_by(&self) -> &ProposalActor {
        &self.proposed_by
    }

    pub fn project_id(&self) -> Option<Uuid> {
        self.project_id
    }

    pub fn changes(&self) -> &[FieldChange] {
        &self.changes
    }

    pub fn payload(&self) -> &Value {
        &self.payload
    }

    pub fn requirement(&self) -> ApprovalRequirement {
        self.requirement
    }

    pub fn status(&self) -> DraftStatus {
        self.status
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn decisions(&self) -> &[ApprovalRecord] {
        &self.decisions
    }

    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub const fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    #[allow(clippy::too_many_arguments)]
    pub fn decide(
        &mut self,
        expected_revision: u64,
        decision: ApprovalDecision,
        approver: HumanApprover,
        statement: Option<String>,
        step_up_verified: bool,
        decided_at: DateTime<Utc>,
    ) -> Result<(), ApprovalError> {
        self.ensure_revision(expected_revision)?;
        if self.status != DraftStatus::PendingApproval {
            return Err(ApprovalError::DraftIsFinal {
                status: self.status,
            });
        }
        if decided_at >= self.expires_at {
            self.status = DraftStatus::Expired;
            self.revision += 1;
            return Err(ApprovalError::Expired);
        }

        if decision == ApprovalDecision::Approve {
            let has_statement = statement
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());

            match self.requirement {
                ApprovalRequirement::PreviewConfirmation => {}
                ApprovalRequirement::ResearcherSignature if !has_statement => {
                    return Err(ApprovalError::SignatureRequired);
                }
                ApprovalRequirement::ReinforcedConfirmation if !step_up_verified => {
                    return Err(ApprovalError::StepUpRequired);
                }
                ApprovalRequirement::ReinforcedConfirmation if !has_statement => {
                    return Err(ApprovalError::ConfirmationStatementRequired);
                }
                ApprovalRequirement::ResearcherSignature
                | ApprovalRequirement::ReinforcedConfirmation => {}
            }
        }

        self.decisions.push(ApprovalRecord {
            decision,
            approver,
            statement,
            step_up_verified,
            decided_at,
            draft_revision: self.revision,
        });
        self.status = match decision {
            ApprovalDecision::Approve => DraftStatus::Approved,
            ApprovalDecision::Reject => DraftStatus::Rejected,
        };
        self.revision += 1;
        Ok(())
    }

    /// Marks an approved draft as applied after its domain mutation commits.
    pub fn mark_applied(&mut self, expected_revision: u64) -> Result<(), ApprovalError> {
        self.ensure_revision(expected_revision)?;
        if self.status != DraftStatus::Approved {
            return Err(ApprovalError::NotApproved {
                status: self.status,
            });
        }
        self.status = DraftStatus::Applied;
        self.revision += 1;
        Ok(())
    }

    pub fn cancel(&mut self, expected_revision: u64) -> Result<(), ApprovalError> {
        self.ensure_revision(expected_revision)?;
        if self.status != DraftStatus::PendingApproval {
            return Err(ApprovalError::DraftIsFinal {
                status: self.status,
            });
        }
        self.status = DraftStatus::Cancelled;
        self.revision += 1;
        Ok(())
    }

    pub fn expire_if_due(&mut self, now: DateTime<Utc>) -> bool {
        if self.status == DraftStatus::PendingApproval && now >= self.expires_at {
            self.status = DraftStatus::Expired;
            self.revision += 1;
            true
        } else {
            false
        }
    }

    /// Revalidates a deserialized draft before it is trusted by application code.
    pub fn validate_integrity(&self) -> Result<(), ApprovalError> {
        if self.expires_at <= self.created_at {
            return Err(ApprovalError::InvalidExpiration);
        }
        if self.changes.is_empty() && self.payload.is_null() {
            return Err(ApprovalError::EmptyDraft);
        }
        if self.requirement != self.kind.approval_requirement() {
            return Err(ApprovalError::RequirementMismatch);
        }
        ensure_no_raw_sql(&self.payload, "/payload")?;
        for change in &self.changes {
            validate_change(change)?;
        }
        Ok(())
    }

    fn ensure_revision(&self, expected_revision: u64) -> Result<(), ApprovalError> {
        if self.revision == expected_revision {
            Ok(())
        } else {
            Err(ApprovalError::RevisionConflict {
                expected: expected_revision,
                actual: self.revision,
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ApprovalError {
    #[error("a write draft must contain a payload or at least one field change")]
    EmptyDraft,
    #[error("draft expiration must be after creation")]
    InvalidExpiration,
    #[error("AI is not permitted to propose this draft kind: {kind:?}")]
    AiCannotProposeKind { kind: DraftKind },
    #[error("invalid field change at {path}: {reason}")]
    InvalidChange { path: String, reason: &'static str },
    #[error("raw SQL is forbidden at {path}")]
    RawSqlForbidden { path: String },
    #[error("stored approval requirement does not match the draft kind")]
    RequirementMismatch,
    #[error("draft revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("draft is no longer pending approval: {status:?}")]
    DraftIsFinal { status: DraftStatus },
    #[error("draft has expired")]
    Expired,
    #[error("a researcher signature statement is required")]
    SignatureRequired,
    #[error("step-up verification is required")]
    StepUpRequired,
    #[error("a confirmation statement is required")]
    ConfirmationStatementRequired,
    #[error("draft has not been approved: {status:?}")]
    NotApproved { status: DraftStatus },
}

fn validate_change(change: &FieldChange) -> Result<(), ApprovalError> {
    if !change.path.starts_with('/') || change.path.len() > 256 {
        return Err(ApprovalError::InvalidChange {
            path: change.path.clone(),
            reason: "path must be a JSON Pointer no longer than 256 bytes",
        });
    }
    if change.before.is_none() && change.after.is_none() {
        return Err(ApprovalError::InvalidChange {
            path: change.path.clone(),
            reason: "before and after cannot both be absent",
        });
    }
    if let Some(value) = &change.before {
        ensure_no_raw_sql(value, &format!("{}/before", change.path))?;
    }
    if let Some(value) = &change.after {
        ensure_no_raw_sql(value, &format!("{}/after", change.path))?;
    }
    Ok(())
}

fn ensure_no_raw_sql(value: &Value, path: &str) -> Result<(), ApprovalError> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let normalized = key
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>();
                let child_path = format!("{path}/{key}");
                if normalized == "sql" || normalized == "rawsql" {
                    return Err(ApprovalError::RawSqlForbidden { path: child_path });
                }
                ensure_no_raw_sql(child, &child_path)?;
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                ensure_no_raw_sql(child, &format!("{path}/{index}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use serde_json::json;

    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-18T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn approver() -> HumanApprover {
        HumanApprover {
            user_id: Uuid::new_v4(),
            display_name: "Researcher".into(),
        }
    }

    fn draft(kind: DraftKind) -> WriteDraft {
        WriteDraft::new(
            kind,
            ToolName::MutationDraft,
            ProposalActor::Ai {
                user_id: Uuid::new_v4(),
                tool_run_id: Uuid::new_v4(),
            },
            Some(Uuid::new_v4()),
            vec![FieldChange {
                path: "/status".into(),
                before: Some(json!("active")),
                after: Some(json!("completed")),
            }],
            json!({"status": "completed"}),
            now(),
            now() + Duration::hours(1),
        )
        .unwrap()
    }

    #[test]
    fn measurement_requires_researcher_signature() {
        let mut draft = draft(DraftKind::MeasurementResult);
        let error = draft
            .decide(
                draft.revision(),
                ApprovalDecision::Approve,
                approver(),
                None,
                false,
                now() + Duration::minutes(1),
            )
            .unwrap_err();

        assert_eq!(error, ApprovalError::SignatureRequired);
        assert_eq!(draft.status(), DraftStatus::PendingApproval);

        draft
            .decide(
                draft.revision(),
                ApprovalDecision::Approve,
                approver(),
                Some("I verified the source measurement.".into()),
                false,
                now() + Duration::minutes(2),
            )
            .unwrap();
        assert_eq!(draft.status(), DraftStatus::Approved);
    }

    #[test]
    fn research_plan_cannot_be_approved_without_researcher_signature() {
        let mut draft = draft(DraftKind::ResearchPlan);
        let error = draft
            .decide(
                draft.revision(),
                ApprovalDecision::Approve,
                approver(),
                Some("   ".to_owned()),
                false,
                now() + Duration::minutes(1),
            )
            .unwrap_err();

        assert_eq!(error, ApprovalError::SignatureRequired);
        assert_eq!(draft.status(), DraftStatus::PendingApproval);
    }

    #[test]
    fn sensitive_draft_requires_step_up_confirmation() {
        let mut draft = draft(DraftKind::SoftDelete);
        let error = draft
            .decide(
                draft.revision(),
                ApprovalDecision::Approve,
                approver(),
                Some("I confirm this deletion.".into()),
                false,
                now() + Duration::minutes(1),
            )
            .unwrap_err();

        assert_eq!(error, ApprovalError::StepUpRequired);
    }

    #[test]
    fn stale_revision_cannot_approve() {
        let mut draft = draft(DraftKind::OrdinaryWrite);
        let error = draft
            .decide(
                0,
                ApprovalDecision::Approve,
                approver(),
                None,
                false,
                now() + Duration::minutes(1),
            )
            .unwrap_err();

        assert!(matches!(error, ApprovalError::RevisionConflict { .. }));
    }

    #[test]
    fn raw_sql_payload_is_rejected_recursively() {
        let error = WriteDraft::new(
            DraftKind::OrdinaryWrite,
            ToolName::MutationDraft,
            ProposalActor::Human {
                user_id: Uuid::new_v4(),
            },
            None,
            vec![],
            json!({"command": {"raw_sql": "delete from animals"}}),
            now(),
            now() + Duration::hours(1),
        )
        .unwrap_err();

        assert!(matches!(error, ApprovalError::RawSqlForbidden { .. }));
    }

    #[test]
    fn ai_cannot_propose_permission_or_migration() {
        let error = WriteDraft::new(
            DraftKind::PermissionChange,
            ToolName::MutationDraft,
            ProposalActor::Ai {
                user_id: Uuid::new_v4(),
                tool_run_id: Uuid::new_v4(),
            },
            None,
            vec![],
            json!({"role": "admin"}),
            now(),
            now() + Duration::hours(1),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            ApprovalError::AiCannotProposeKind {
                kind: DraftKind::PermissionChange
            }
        ));
    }

    #[test]
    fn applying_requires_approval_and_matching_revision() {
        let mut draft = draft(DraftKind::OrdinaryWrite);
        assert!(matches!(
            draft.mark_applied(draft.revision()),
            Err(ApprovalError::NotApproved { .. })
        ));

        draft
            .decide(
                draft.revision(),
                ApprovalDecision::Approve,
                approver(),
                None,
                false,
                now() + Duration::minutes(1),
            )
            .unwrap();
        let approved_revision = draft.revision();
        draft.mark_applied(approved_revision).unwrap();
        assert_eq!(draft.status(), DraftStatus::Applied);
    }
}
