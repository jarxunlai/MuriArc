use std::collections::HashMap;

use axum::{Json, Router, extract::State, routing::get};
use chrono::{DateTime, Utc};
use muriarc_core::{
    Actor, AuditAction, AuditEntry, AuditFilter, EntityType, Permission, WriteSource,
    protect_public_audit_entries,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiQuery, CollectionResponse, collection, scope, store,
    validation::{collection_limit, truncate},
};

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route("/operations", get(list))
        .route("/operations/catalog", get(catalog))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    project_id: Option<Uuid>,
    entity_id: Option<Uuid>,
    entity_type: Option<String>,
    actor_id: Option<Uuid>,
    source: Option<WriteSource>,
    operation_code: Option<String>,
    scope: Option<OperationScope>,
    limit: Option<usize>,
    include_technical: Option<bool>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OperationScope {
    Lab,
    Project,
    Experiment,
    Animal,
    User,
    Ai,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationView {
    id: Uuid,
    operation_code: String,
    operation_version: i32,
    title: String,
    summary: String,
    entity_type: EntityType,
    entity_id: Uuid,
    entity_name_snapshot: Option<String>,
    entity_revision: Option<i64>,
    project_id: Option<Uuid>,
    actor: Actor,
    source: WriteSource,
    request_id: Option<String>,
    reason: Option<String>,
    operation_params: Value,
    before: Option<Value>,
    after: Option<Value>,
    occurred_at: DateTime<Utc>,
    batch_count: usize,
}

impl From<AuditEntry> for OperationView {
    fn from(entry: AuditEntry) -> Self {
        let title = operation_title(&entry);
        let summary = operation_summary(&entry, &title);
        Self {
            id: entry.id,
            operation_code: entry.operation_code,
            operation_version: entry.operation_version,
            title,
            summary,
            entity_type: entry.entity_type,
            entity_id: entry.entity_id,
            entity_name_snapshot: entry.entity_name_snapshot,
            entity_revision: entry.entity_revision,
            project_id: entry.project_id,
            actor: entry.actor,
            source: entry.source,
            request_id: entry.request_id,
            reason: entry.reason,
            operation_params: entry.operation_params,
            before: entry.before,
            after: entry.after,
            occurred_at: entry.occurred_at,
            batch_count: 1,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogItem {
    action: AuditAction,
    title_template: String,
    summary_template: &'static str,
}

async fn list(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<Json<CollectionResponse<OperationView>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadActivity,
    )
    .await?;

    let mut entries = store(
        state.store.list_audit_entries(&AuditFilter {
            lab_id: principal.lab_id,
            project_id: query.project_id,
            entity_id: query.entity_id,
        }),
        &metadata,
    )
    .await?;
    protect_public_audit_entries(&mut entries);
    entries.reverse();
    entries.retain(|entry| {
        query
            .entity_type
            .as_deref()
            .is_none_or(|value| entry.entity_type.as_str() == value)
            && query
                .actor_id
                .is_none_or(|value| entry.actor.user_id == Some(value))
            && query.source.is_none_or(|value| entry.source == value)
            && query
                .operation_code
                .as_deref()
                .is_none_or(|value| entry.operation_code == value)
            && query
                .scope
                .is_none_or(|value| in_scope(entry.entity_type, value))
    });
    let include_audit_details = query.include_technical.unwrap_or(false);
    if include_audit_details {
        scope::optional_project_permission(
            &state,
            &principal,
            &metadata,
            query.project_id,
            Permission::ReadAudit,
        )
        .await?;
    } else {
        entries.retain(is_key_activity);
    }
    truncate(&mut entries, collection_limit(query.limit, &metadata)?);
    let mut views = Vec::<OperationView>::new();
    let mut grouped = HashMap::<(String, String), usize>::new();
    for entry in entries {
        let group_key = entry.request_id.as_ref().map(|request_id| {
            (
                request_id.clone(),
                activity_group(entry.entity_type, entry.action).to_owned(),
            )
        });
        if let Some(index) = group_key.as_ref().and_then(|key| grouped.get(key)).copied() {
            let view = &mut views[index];
            view.batch_count += 1;
            view.summary = format!(
                "{} 批量完成 {} 项{}",
                view.actor.display_name, view.batch_count, view.title
            );
            continue;
        }
        let mut view = OperationView::from(entry);
        if !include_audit_details {
            view.operation_params = Value::Object(Default::default());
            view.before = None;
            view.after = None;
        }
        if let Some(key) = group_key {
            grouped.insert(key, views.len());
        }
        views.push(view);
    }
    Ok(collection(views, &metadata))
}

async fn catalog(
    principal: AuthPrincipal,
    metadata: RequestMetadata,
) -> Result<Json<CollectionResponse<CatalogItem>>, ApiError> {
    if !principal.can(Permission::ReadActivity, None)
        && !principal
            .project_ids()
            .any(|project_id| principal.can(Permission::ReadActivity, Some(project_id)))
    {
        return Err(ApiError::forbidden().with_request_id(metadata.request_id));
    }
    let actions = [
        AuditAction::Create,
        AuditAction::Update,
        AuditAction::SoftDelete,
        AuditAction::Revoke,
        AuditAction::Publish,
        AuditAction::Sign,
        AuditAction::Import,
        AuditAction::Link,
        AuditAction::Archive,
        AuditAction::Process,
        AuditAction::Approve,
        AuditAction::Export,
        AuditAction::Cleanup,
        AuditAction::EnterAdminView,
    ];
    Ok(collection(
        actions
            .into_iter()
            .map(|action| CatalogItem {
                action,
                title_template: action_label(action).to_owned(),
                summary_template: "{actor} 对 {entityType} {entityNameOrId} 执行了确定性操作",
            })
            .collect(),
        &metadata,
    ))
}

fn in_scope(entity_type: EntityType, scope: OperationScope) -> bool {
    match scope {
        OperationScope::Lab => true,
        OperationScope::Project => matches!(
            entity_type,
            EntityType::Project
                | EntityType::Membership
                | EntityType::ProjectAnimalAssignment
                | EntityType::Attachment
                | EntityType::AttachmentLink
                | EntityType::AttachmentDerivative
                | EntityType::Job
        ),
        OperationScope::Experiment => matches!(
            entity_type,
            EntityType::Experiment
                | EntityType::ExperimentEvent
                | EntityType::ExperimentTemplateVersion
                | EntityType::Cohort
                | EntityType::Participation
                | EntityType::Procedure
                | EntityType::Measurement
                | EntityType::Sample
                | EntityType::ObservationDefinition
                | EntityType::Observation
                | EntityType::ObservationValue
        ),
        OperationScope::Animal => matches!(
            entity_type,
            EntityType::Animal
                | EntityType::AnimalEvent
                | EntityType::Cage
                | EntityType::Genotype
                | EntityType::GenotypingRecord
                | EntityType::Pedigree
        ),
        OperationScope::User => matches!(
            entity_type,
            EntityType::User
                | EntityType::UserCredential
                | EntityType::AuthSession
                | EntityType::ExternalToken
                | EntityType::Membership
        ),
        OperationScope::Ai => matches!(
            entity_type,
            EntityType::AiConversation
                | EntityType::AiConversationMessage
                | EntityType::AiConversationSource
                | EntityType::AiProviderSettings
                | EntityType::AiPrivateImage
                | EntityType::AiExtractionDraft
                | EntityType::ToolRun
                | EntityType::Approval
        ),
    }
}

fn operation_title(entry: &AuditEntry) -> String {
    match (entry.entity_type, entry.action) {
        (EntityType::ProjectAnimalAssignment, AuditAction::Create) => "分配动物到项目".to_owned(),
        (EntityType::ProjectAnimalAssignment, AuditAction::SoftDelete) => {
            "从项目移除动物".to_owned()
        }
        (EntityType::Participation, AuditAction::Create) => "动物加入实验".to_owned(),
        (EntityType::AnimalEvent, AuditAction::Create)
            if entry
                .after
                .as_ref()
                .and_then(|value| value.get("kind"))
                .and_then(|kind| kind.get("transferred"))
                .is_some() =>
        {
            "动物转笼".to_owned()
        }
        _ => format!(
            "{}{}",
            action_label(entry.action),
            entity_label(entry.entity_type)
        ),
    }
}

fn activity_group(entity_type: EntityType, action: AuditAction) -> &'static str {
    match (entity_type, action) {
        (EntityType::ProjectAnimalAssignment, AuditAction::Create) => "project-animal-assign",
        (EntityType::ProjectAnimalAssignment, AuditAction::SoftDelete) => "project-animal-remove",
        (EntityType::AnimalEvent, AuditAction::Create) => "animal-event",
        (EntityType::Participation, AuditAction::Create) => "experiment-enroll",
        _ => entity_type.as_str(),
    }
}

fn is_key_activity(entry: &AuditEntry) -> bool {
    // Retention-policy changes and cleanup remain permanently auditable, but
    // automatic cleanup must not recreate the noise that the activity view is
    // designed to remove.
    if entry.entity_type == EntityType::TechnicalLogPolicy {
        return false;
    }
    matches!(
        entry.entity_type,
        EntityType::AnimalEvent
            | EntityType::ProjectAnimalAssignment
            | EntityType::BreedingPair
            | EntityType::MatingEvent
            | EntityType::Litter
            | EntityType::AnimalDraft
            | EntityType::Experiment
            | EntityType::Participation
            | EntityType::Procedure
            | EntityType::Measurement
            | EntityType::Sample
            | EntityType::Attachment
            | EntityType::Job
            | EntityType::Approval
    ) || matches!(
        entry.action,
        AuditAction::SoftDelete
            | AuditAction::Sign
            | AuditAction::Import
            | AuditAction::Export
            | AuditAction::Cleanup
    )
}

fn operation_summary(entry: &AuditEntry, title: &str) -> String {
    let target = entry
        .entity_name_snapshot
        .as_deref()
        .map(str::to_owned)
        .unwrap_or_else(|| entity_label(entry.entity_type).to_owned());
    format!(
        "{}：{}「{}」；来源 {}",
        entry.actor.display_name,
        title,
        target,
        source_label(entry.source)
    )
}

const fn action_label(action: AuditAction) -> &'static str {
    match action {
        AuditAction::Create => "新建",
        AuditAction::Update => "修改",
        AuditAction::SoftDelete => "删除",
        AuditAction::Revoke => "撤销",
        AuditAction::Publish => "发布",
        AuditAction::Sign => "签署",
        AuditAction::Import => "导入",
        AuditAction::Link => "关联",
        AuditAction::Archive => "归档",
        AuditAction::Process => "处理",
        AuditAction::Approve => "批准",
        AuditAction::Export => "导出",
        AuditAction::Cleanup => "批量清理",
        AuditAction::EnterAdminView => "进入管理员视图",
    }
}

const fn entity_label(entity_type: EntityType) -> &'static str {
    match entity_type {
        EntityType::Lab => "实验室",
        EntityType::User => "用户",
        EntityType::UserCredential => "用户凭据",
        EntityType::AuthSession => "登录会话",
        EntityType::ExternalToken => "外部令牌",
        EntityType::Project => "项目",
        EntityType::Membership => "成员权限",
        EntityType::ProjectAnimalAssignment => "项目动物分配",
        EntityType::Cage => "笼位",
        EntityType::Animal => "动物",
        EntityType::AnimalEvent => "动物事件",
        EntityType::GeneLocus => "基因位点",
        EntityType::Allele => "等位基因",
        EntityType::Genotype => "基因型",
        EntityType::GenotypeDefinition => "基因型定义",
        EntityType::GenotypingRecord => "基因检测记录",
        EntityType::BreedingLine => "繁育品系",
        EntityType::Colony => "繁育群体",
        EntityType::BreedingPair => "繁育配对",
        EntityType::BreedingPairMember => "繁育成员",
        EntityType::MatingEvent => "交配事件",
        EntityType::Litter => "窝次",
        EntityType::AnimalDraft => "待登记动物",
        EntityType::Pedigree => "谱系",
        EntityType::ExperimentEvent => "实验事件",
        EntityType::ObservationDefinition => "观察定义",
        EntityType::Observation => "观察记录",
        EntityType::ObservationValue => "观察值",
        EntityType::ExperimentTemplateVersion => "实验模板",
        EntityType::Experiment => "实验",
        EntityType::Cohort => "实验组",
        EntityType::Participation => "实验参与",
        EntityType::Procedure => "实验步骤",
        EntityType::Measurement => "测量",
        EntityType::Sample => "样本",
        EntityType::Attachment => "附件",
        EntityType::AttachmentLink => "附件关联",
        EntityType::AttachmentDerivative => "附件派生物",
        EntityType::AiPrivateImage => "私人 AI 图片",
        EntityType::AiExtractionDraft => "AI 提取草稿",
        EntityType::AiConversation => "AI 会话",
        EntityType::AiConversationMessage => "AI 消息",
        EntityType::AiConversationSource => "AI 会话来源",
        EntityType::AiAutonomyGrant => "AI 会话授权",
        EntityType::AiProviderSettings => "AI Provider 设置",
        EntityType::AiProviderEndpoint => "AI Provider 端点",
        EntityType::AiLabSettings => "实验室 AI 设置",
        EntityType::TechnicalLogPolicy => "技术日志策略",
        EntityType::ToolRun => "AI 工具执行",
        EntityType::Approval => "审批",
        EntityType::Job => "数据任务",
        EntityType::Provenance => "来源记录",
    }
}

const fn source_label(source: WriteSource) -> &'static str {
    match source {
        WriteSource::Desktop => "桌面端",
        WriteSource::Web => "Web",
        WriteSource::Api => "API",
        WriteSource::Mcp => "MCP",
        WriteSource::Ai => "AI",
        WriteSource::Migration => "迁移",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_rendering_is_deterministic_and_human_readable() {
        assert_eq!(
            format!(
                "{}{}",
                action_label(AuditAction::Link),
                entity_label(EntityType::AttachmentLink)
            ),
            "关联附件关联"
        );
        assert_eq!(source_label(WriteSource::Web), "Web");
        assert!(in_scope(EntityType::AiExtractionDraft, OperationScope::Ai));
        assert!(!in_scope(EntityType::Animal, OperationScope::Ai));

        let mut entry = AuditEntry {
            id: Uuid::new_v4(),
            lab_id: Uuid::new_v4(),
            project_id: None,
            entity_type: EntityType::AiProviderEndpoint,
            entity_id: Uuid::new_v4(),
            action: AuditAction::Create,
            actor: Actor::system("MuriArc"),
            source: WriteSource::Web,
            request_id: None,
            reason: None,
            before: None,
            after: None,
            operation_code: String::new(),
            operation_version: 1,
            operation_params: Value::Object(Default::default()),
            entity_name_snapshot: None,
            entity_revision: None,
            occurred_at: Utc::now(),
        };
        assert_eq!(operation_title(&entry), "新建AI Provider 端点");

        entry.entity_type = EntityType::AnimalEvent;
        let summary = operation_summary(&entry, "记录动物事件");
        assert!(summary.contains("动物事件"));
        assert!(!summary.contains(&entry.entity_id.to_string()[..8]));

        entry.entity_type = EntityType::TechnicalLogPolicy;
        entry.action = AuditAction::Cleanup;
        assert!(!is_key_activity(&entry));
    }
}
