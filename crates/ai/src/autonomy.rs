use muriarc_core::AiAutonomyMode;

use crate::ToolName;

/// Server-authoritative policy for actions that may happen without an
/// additional prompt inside the current conversation. It does not grant any
/// domain permission and deliberately has no path for signatures, transfers,
/// death, deletion, imports, account changes, breeding facts, or audit cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiActionPolicy {
    mode: AiAutonomyMode,
}

impl AiActionPolicy {
    pub const fn new(mode: AiAutonomyMode) -> Self {
        Self { mode }
    }

    pub const fn allows_tool(self, tool: ToolName) -> bool {
        match tool {
            // Export creation materializes a new artifact. Ask mode may discuss
            // and preview it, but cannot create it without changing the grant.
            ToolName::ExportCreate => !matches!(self.mode, AiAutonomyMode::Ask),
            // All mutation tools below only create reviewable drafts. Existing
            // reinforced confirmation and researcher-signature gates remain in
            // force independently of autonomy mode.
            ToolName::ImportCommitDraft
            | ToolName::MutationDraft
            | ToolName::ExperimentTemplateDraft
            | ToolName::ImportPreview
            | ToolName::AnimalSearch
            | ToolName::AnimalTimeline
            | ToolName::CageList
            | ToolName::ProjectList
            | ToolName::ExperimentStatus
            | ToolName::MeasurementQuery
            | ToolName::SampleInventory => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_blocks_artifact_creation_but_never_blocks_reads_or_drafts() {
        let policy = AiActionPolicy::new(AiAutonomyMode::Ask);
        assert!(!policy.allows_tool(ToolName::ExportCreate));
        assert!(policy.allows_tool(ToolName::AnimalSearch));
        assert!(policy.allows_tool(ToolName::MutationDraft));
    }
}
