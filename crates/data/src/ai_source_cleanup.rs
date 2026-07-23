use chrono::{DateTime, Utc};
use muriarc_core::{ActorType, AuditContext, MuriArcStore, StoreError, StoreResult, WriteSource};
use uuid::Uuid;

use crate::AttachmentFiles;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AiConversationSourceCleanupReport {
    pub inspected: usize,
    pub discarded: usize,
    pub cleaned: usize,
    pub conflicts: usize,
    pub store_failures: usize,
    pub object_failures: usize,
}

/// Retires one deterministic, bounded batch of expired conversation sources.
///
/// Each source and attachment is soft-deleted atomically with a durable object
/// cleanup queue item before the immutable object is removed. Object deletion
/// is deliberately best-effort: a filesystem failure is reported and retained
/// for a later sweep without rolling back audit/provenance or resurrecting
/// source metadata.
pub async fn cleanup_expired_ai_conversation_sources(
    store: &dyn MuriArcStore,
    files: &AttachmentFiles,
    lab_id: Uuid,
    now: DateTime<Utc>,
    limit: i64,
    write_source: WriteSource,
) -> StoreResult<AiConversationSourceCleanupReport> {
    let pending = store
        .list_pending_ai_conversation_source_object_deletions(lab_id, limit)
        .await?;
    let mut report = AiConversationSourceCleanupReport::default();
    let mut audit = AuditContext::system(write_source);
    debug_assert_eq!(audit.actor.actor_type, ActorType::System);
    audit.request_id = Some(Uuid::new_v4().to_string());
    audit.reason = Some("ai_conversation_source_retention".to_owned());

    report.inspected += pending.len();
    for candidate in pending {
        match files.remove_verified_object(&candidate.attachment).await {
            Ok(()) => match store
                .complete_ai_conversation_source_object_deletion(
                    candidate.source.id,
                    candidate.attachment.id,
                    now,
                    &audit,
                )
                .await
            {
                Ok(()) => report.cleaned += 1,
                Err(StoreError::Conflict(_) | StoreError::NotFound { .. }) => {
                    report.conflicts += 1;
                }
                Err(_) => report.store_failures += 1,
            },
            Err(_) => report.object_failures += 1,
        }
    }

    let remaining = limit.saturating_sub(i64::try_from(report.inspected).unwrap_or(limit));
    if remaining == 0 {
        return Ok(report);
    }
    let expired = store
        .list_expired_ai_conversation_sources(lab_id, now, remaining)
        .await?;
    report.inspected += expired.len();
    for candidate in expired {
        match store
            .discard_ai_conversation_source(
                candidate.source.id,
                candidate.source.meta.revision,
                now,
                &audit,
            )
            .await
        {
            Ok(_) => {
                report.discarded += 1;
                match files.remove_verified_object(&candidate.attachment).await {
                    Ok(()) => match store
                        .complete_ai_conversation_source_object_deletion(
                            candidate.source.id,
                            candidate.attachment.id,
                            now,
                            &audit,
                        )
                        .await
                    {
                        Ok(()) => report.cleaned += 1,
                        Err(StoreError::Conflict(_) | StoreError::NotFound { .. }) => {
                            report.conflicts += 1;
                        }
                        Err(_) => report.store_failures += 1,
                    },
                    Err(_) => report.object_failures += 1,
                }
            }
            Err(StoreError::Conflict(_) | StoreError::NotFound { .. }) => {
                report.conflicts += 1;
            }
            Err(_) => {
                report.store_failures += 1;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use muriarc_core::{
        ActorType, AiConversation, AiConversationSource, AiConversationSourceKind,
        AiConversationSourceStatus, AiOperationStore, Attachment, AuditAction, AuditFilter, Lab,
        MuriArcStore, Project, ProvenanceFilter, RecordMeta, User, WorkspaceStore,
    };
    use muriarc_store_sqlite::SqliteStore;

    use super::*;

    struct Fixture {
        _root: tempfile::TempDir,
        store: SqliteStore,
        files: AttachmentFiles,
        lab: Lab,
        user: User,
        lab_conversation: AiConversation,
        project_conversation: AiConversation,
        project: Project,
        now: DateTime<Utc>,
    }

    impl Fixture {
        async fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let files = AttachmentFiles::new(root.path().join("attachments"));
            files.initialize().await.unwrap();
            let store = SqliteStore::in_memory().await.unwrap();
            store.migrate().await.unwrap();
            let now = Utc::now();
            let lab = Lab::new("AI cleanup lab", now).unwrap();
            let migration = AuditContext::system(WriteSource::Migration);
            store.create_lab(&lab, &migration).await.unwrap();
            let user = User::new(lab.id, "cleanup@example.test", "Cleanup owner", now).unwrap();
            store.create_user(&user, &migration).await.unwrap();
            let project = Project::new(lab.id, "Cleanup project", now).unwrap();
            store.create_project(&project, &migration).await.unwrap();
            let lab_conversation = AiConversation {
                id: Uuid::new_v4(),
                lab_id: lab.id,
                project_id: None,
                user_id: user.id,
                title: "Lab cleanup".to_owned(),
                pinned_at: None,
                archived_at: None,
                meta: RecordMeta::new(now),
            };
            let project_conversation = AiConversation {
                id: Uuid::new_v4(),
                project_id: Some(project.id),
                title: "Project cleanup".to_owned(),
                ..lab_conversation.clone()
            };
            store
                .create_ai_conversation(&lab_conversation, &migration)
                .await
                .unwrap();
            store
                .create_ai_conversation(&project_conversation, &migration)
                .await
                .unwrap();
            Self {
                _root: root,
                store,
                files,
                lab,
                user,
                lab_conversation,
                project_conversation,
                project,
                now,
            }
        }

        async fn source(
            &self,
            conversation: &AiConversation,
            status: AiConversationSourceStatus,
            last_activity_at: DateTime<Utc>,
            expires_at: DateTime<Utc>,
            bytes: &[u8],
        ) -> (AiConversationSource, Attachment, std::path::PathBuf) {
            let attachment_id = Uuid::new_v4();
            let object = self.files.write_bytes(attachment_id, bytes).await.unwrap();
            let source_id = Uuid::new_v4();
            let attachment = Attachment {
                id: attachment_id,
                lab_id: self.lab.id,
                project_id: None,
                entity_type: "ai_conversation_source".to_owned(),
                entity_id: source_id,
                file_name: format!("{source_id}.txt"),
                media_type: Some("text/plain".to_owned()),
                relative_path: object.relative_path,
                size_bytes: object.size_bytes,
                sha256: object.sha256,
                version: 1,
                meta: RecordMeta::new(last_activity_at),
            };
            let source = AiConversationSource {
                id: source_id,
                lab_id: self.lab.id,
                user_id: self.user.id,
                conversation_id: Some(conversation.id),
                project_id: conversation.project_id,
                attachment_id,
                kind: AiConversationSourceKind::Text,
                status,
                last_activity_at,
                expires_at,
                archived_at: None,
                error_code: (status == AiConversationSourceStatus::Failed)
                    .then(|| "parse_failed".to_owned()),
                meta: RecordMeta::new(last_activity_at),
            };
            self.store
                .create_ai_conversation_source(
                    &attachment,
                    &source,
                    &AuditContext::system(WriteSource::Migration),
                )
                .await
                .unwrap();
            (source, attachment, object.absolute_path)
        }
    }

    #[tokio::test]
    async fn cleanup_removes_only_expired_unarchived_objects_with_system_audit() {
        let fixture = Fixture::new().await;
        let (expired, _, expired_path) = fixture
            .source(
                &fixture.lab_conversation,
                AiConversationSourceStatus::Ready,
                fixture.now - Duration::hours(2),
                fixture.now - Duration::hours(1),
                b"expired",
            )
            .await;
        let (_, future_attachment, future_path) = fixture
            .source(
                &fixture.lab_conversation,
                AiConversationSourceStatus::Failed,
                fixture.now,
                fixture.now + Duration::days(10),
                b"future",
            )
            .await;
        let (archivable, archived_attachment, archived_path) = fixture
            .source(
                &fixture.project_conversation,
                AiConversationSourceStatus::Staged,
                fixture.now,
                fixture.now + Duration::days(1),
                b"archived",
            )
            .await;
        fixture
            .store
            .archive_ai_conversation_source(
                archivable.id,
                fixture.project.id,
                archivable.meta.revision,
                fixture.now + Duration::seconds(1),
                &AuditContext::system(WriteSource::Migration),
            )
            .await
            .unwrap();

        let report = cleanup_expired_ai_conversation_sources(
            &fixture.store,
            &fixture.files,
            fixture.lab.id,
            fixture.now + Duration::days(2),
            100,
            WriteSource::Desktop,
        )
        .await
        .unwrap();
        assert_eq!(
            report,
            AiConversationSourceCleanupReport {
                inspected: 1,
                discarded: 1,
                cleaned: 1,
                conflicts: 0,
                store_failures: 0,
                object_failures: 0,
            }
        );
        assert!(!expired_path.exists());
        assert!(future_path.exists());
        assert!(archived_path.exists());
        assert!(
            fixture
                .store
                .list_pending_ai_conversation_source_object_deletions(fixture.lab.id, 100)
                .await
                .unwrap()
                .is_empty()
        );
        fixture
            .files
            .open_verified(&future_attachment)
            .await
            .unwrap();
        fixture
            .files
            .open_verified(&archived_attachment)
            .await
            .unwrap();

        let audit = fixture
            .store
            .list_audit_entries(&AuditFilter {
                lab_id: fixture.lab.id,
                project_id: None,
                entity_id: Some(expired.id),
            })
            .await
            .unwrap();
        assert!(audit.iter().any(|entry| {
            entry.action == AuditAction::SoftDelete
                && entry.actor.actor_type == ActorType::System
                && entry.source == WriteSource::Desktop
        }));
        assert!(
            !fixture
                .store
                .list_provenance(&ProvenanceFilter {
                    lab_id: fixture.lab.id,
                    entity_id: Some(expired.id),
                    ..ProvenanceFilter::default()
                })
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn object_failure_is_reported_without_rolling_back_soft_discard() {
        let fixture = Fixture::new().await;
        let (expired, _, path) = fixture
            .source(
                &fixture.lab_conversation,
                AiConversationSourceStatus::Ready,
                fixture.now - Duration::hours(2),
                fixture.now - Duration::hours(1),
                b"expected",
            )
            .await;
        tokio::fs::write(&path, b"tampered").await.unwrap();

        let report = cleanup_expired_ai_conversation_sources(
            &fixture.store,
            &fixture.files,
            fixture.lab.id,
            fixture.now,
            100,
            WriteSource::Web,
        )
        .await
        .unwrap();
        assert_eq!(report.cleaned, 0);
        assert_eq!(report.discarded, 1);
        assert_eq!(report.object_failures, 1);
        assert!(path.exists());
        assert!(matches!(
            fixture.store.get_ai_conversation_source(expired.id).await,
            Err(StoreError::NotFound { .. })
        ));
        assert_eq!(
            fixture
                .store
                .list_pending_ai_conversation_source_object_deletions(fixture.lab.id, 100)
                .await
                .unwrap()
                .len(),
            1,
            "failed object removal must remain durably retryable"
        );

        tokio::fs::write(&path, b"expected").await.unwrap();
        let retry = cleanup_expired_ai_conversation_sources(
            &fixture.store,
            &fixture.files,
            fixture.lab.id,
            fixture.now + Duration::hours(1),
            100,
            WriteSource::Web,
        )
        .await
        .unwrap();
        assert_eq!(retry.discarded, 0);
        assert_eq!(retry.cleaned, 1);
        assert_eq!(retry.object_failures, 0);
        assert!(!path.exists());
        assert!(
            fixture
                .store
                .list_pending_ai_conversation_source_object_deletions(fixture.lab.id, 100)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
