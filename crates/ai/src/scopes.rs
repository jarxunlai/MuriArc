use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use muriarc_core::AiScope as ToolScope;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScopeSet(BTreeSet<ToolScope>);

impl ScopeSet {
    pub fn new(scopes: impl IntoIterator<Item = ToolScope>) -> Self {
        Self(scopes.into_iter().collect())
    }

    pub fn contains(&self, scope: ToolScope) -> bool {
        self.0.contains(&scope)
    }

    pub fn intersection(&self, other: &Self) -> Self {
        Self(self.0.intersection(&other.0).copied().collect())
    }

    pub fn iter(&self) -> impl Iterator<Item = ToolScope> + '_ {
        self.0.iter().copied()
    }

    pub fn require(&self, required: &[ToolScope]) -> Result<(), ToolAuthorizationError> {
        let missing = required
            .iter()
            .copied()
            .filter(|scope| !self.contains(*scope))
            .collect::<Vec<_>>();

        if missing.is_empty() {
            Ok(())
        } else {
            Err(ToolAuthorizationError::MissingScopes { missing })
        }
    }
}

/// A fixed, auditable set of tools exposed to a model.
///
/// Arbitrary SQL and arbitrary HTTP requests are intentionally absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
    ResourceSearch,
    GenotypingQuery,
    AnimalContext,
    ProjectContext,
    ActivityQuery,
    AuditQuery,
    ProvenanceQuery,
    /// Legacy read names remain decodable for persisted traces and compatible
    /// callers, but production executors do not advertise them to models.
    AnimalSearch,
    AnimalTimeline,
    CageList,
    ProjectList,
    ExperimentStatus,
    MeasurementQuery,
    SampleInventory,
    SourceImportPreview,
    ImportPreview,
    ImportCommitDraft,
    ExportCreate,
    ExperimentTemplateDraft,
    MutationDraft,
    ExperimentGroupingDraft,
}

impl ToolName {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResourceSearch => "resource_search",
            Self::GenotypingQuery => "genotyping_query",
            Self::AnimalContext => "animal_context",
            Self::ProjectContext => "project_context",
            Self::ActivityQuery => "activity_query",
            Self::AuditQuery => "audit_query",
            Self::ProvenanceQuery => "provenance_query",
            Self::AnimalSearch => "animal_search",
            Self::AnimalTimeline => "animal_timeline",
            Self::CageList => "cage_list",
            Self::ProjectList => "project_list",
            Self::ExperimentStatus => "experiment_status",
            Self::MeasurementQuery => "measurement_query",
            Self::SampleInventory => "sample_inventory",
            Self::SourceImportPreview => "source_import_preview",
            Self::ImportPreview => "import_preview",
            Self::ImportCommitDraft => "import_commit_draft",
            Self::ExportCreate => "export_create",
            Self::ExperimentTemplateDraft => "experiment_template_draft",
            Self::MutationDraft => "mutation_draft",
            Self::ExperimentGroupingDraft => "experiment_grouping_draft",
        }
    }

    pub fn from_wire_name(value: &str) -> Option<Self> {
        match value {
            "resource_search" => Some(Self::ResourceSearch),
            "genotyping_query" => Some(Self::GenotypingQuery),
            "animal_context" => Some(Self::AnimalContext),
            "project_context" => Some(Self::ProjectContext),
            "activity_query" => Some(Self::ActivityQuery),
            "audit_query" => Some(Self::AuditQuery),
            "provenance_query" => Some(Self::ProvenanceQuery),
            "animal_search" => Some(Self::AnimalSearch),
            "animal_timeline" => Some(Self::AnimalTimeline),
            "cage_list" => Some(Self::CageList),
            "project_list" => Some(Self::ProjectList),
            "experiment_status" => Some(Self::ExperimentStatus),
            "measurement_query" => Some(Self::MeasurementQuery),
            "sample_inventory" => Some(Self::SampleInventory),
            "source_import_preview" => Some(Self::SourceImportPreview),
            "import_preview" => Some(Self::ImportPreview),
            "import_commit_draft" => Some(Self::ImportCommitDraft),
            "export_create" => Some(Self::ExportCreate),
            "experiment_template_draft" => Some(Self::ExperimentTemplateDraft),
            "mutation_draft" => Some(Self::MutationDraft),
            "experiment_grouping_draft" => Some(Self::ExperimentGroupingDraft),
            _ => None,
        }
    }

    /// Tools in this group may only return a reviewable `WriteDraft`; the
    /// assistant service never exposes an apply/commit operation to a model.
    pub const fn is_draft_only(self) -> bool {
        matches!(
            self,
            Self::ImportCommitDraft
                | Self::ExperimentTemplateDraft
                | Self::MutationDraft
                | Self::ExperimentGroupingDraft
        )
    }

    pub const fn required_scopes(self) -> &'static [ToolScope] {
        use ToolScope::{Export, Import, Read, TemplateDraft, WriteDraft};

        match self {
            Self::ResourceSearch
            | Self::GenotypingQuery
            | Self::AnimalContext
            | Self::ProjectContext
            | Self::ActivityQuery
            | Self::AuditQuery
            | Self::ProvenanceQuery
            | Self::AnimalSearch
            | Self::AnimalTimeline
            | Self::CageList
            | Self::ProjectList
            | Self::ExperimentStatus
            | Self::MeasurementQuery
            | Self::SampleInventory => &[Read],
            Self::SourceImportPreview | Self::ImportPreview => &[Read, Import],
            Self::ImportCommitDraft => &[Import, WriteDraft],
            Self::ExportCreate => &[Read, Export],
            Self::ExperimentTemplateDraft => &[TemplateDraft],
            Self::MutationDraft | Self::ExperimentGroupingDraft => &[Read, WriteDraft],
        }
    }
}

/// Effective permissions are always the intersection of the signed-in user's
/// scopes and the external token's scopes. A token can only narrow access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessGrant {
    user_scopes: ScopeSet,
    integration_scopes: Option<ScopeSet>,
}

impl AccessGrant {
    pub fn local_user(user_scopes: ScopeSet) -> Self {
        Self {
            user_scopes,
            integration_scopes: None,
        }
    }

    pub fn external(user_scopes: ScopeSet, integration_scopes: ScopeSet) -> Self {
        Self {
            user_scopes,
            integration_scopes: Some(integration_scopes),
        }
    }

    pub fn effective_scopes(&self) -> ScopeSet {
        match &self.integration_scopes {
            Some(scopes) => self.user_scopes.intersection(scopes),
            None => self.user_scopes.clone(),
        }
    }

    pub fn authorize(&self, tool: ToolName) -> Result<(), ToolAuthorizationError> {
        self.effective_scopes().require(tool.required_scopes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ToolAuthorizationError {
    #[error("missing AI tool scopes: {missing:?}")]
    MissingScopes { missing: Vec<ToolScope> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_token_can_only_narrow_user_access() {
        let grant = AccessGrant::external(
            ScopeSet::new([ToolScope::Read, ToolScope::WriteDraft]),
            ScopeSet::new([ToolScope::Read, ToolScope::Import]),
        );

        assert_eq!(grant.effective_scopes(), ScopeSet::new([ToolScope::Read]));
        assert!(grant.authorize(ToolName::AnimalSearch).is_ok());
        assert!(matches!(
            grant.authorize(ToolName::ImportCommitDraft),
            Err(ToolAuthorizationError::MissingScopes { .. })
        ));
    }

    #[test]
    fn import_commit_requires_import_and_draft_scopes() {
        let grant = AccessGrant::local_user(ScopeSet::new([ToolScope::Import]));
        let error = grant
            .authorize(ToolName::ImportCommitDraft)
            .expect_err("write-draft is also required");

        assert_eq!(
            error,
            ToolAuthorizationError::MissingScopes {
                missing: vec![ToolScope::WriteDraft]
            }
        );
    }

    #[test]
    fn scope_serialization_is_stable() {
        let set = ScopeSet::new([ToolScope::WriteDraft, ToolScope::Read]);
        assert_eq!(
            serde_json::to_value(set).unwrap(),
            serde_json::json!(["read", "write-draft"])
        );
    }
}
