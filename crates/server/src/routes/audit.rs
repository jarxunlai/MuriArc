use axum::{Json, Router, extract::State, routing::get};
use muriarc_core::{AuditEntry, AuditFilter, Permission};
use serde::Deserialize;
use uuid::Uuid;

use crate::{ApiError, AppState, AuthPrincipal, RequestMetadata};

use super::{
    ApiQuery, CollectionResponse, collection, scope, store,
    validation::{collection_limit, truncate},
};

pub(super) fn router() -> Router<AppState> {
    Router::new().route("/audit", get(list))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    project_id: Option<Uuid>,
    entity_id: Option<Uuid>,
    limit: Option<usize>,
}

async fn list(
    State(state): State<AppState>,
    principal: AuthPrincipal,
    metadata: RequestMetadata,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<Json<CollectionResponse<AuditEntry>>, ApiError> {
    scope::optional_project_permission(
        &state,
        &principal,
        &metadata,
        query.project_id,
        Permission::ReadAudit,
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
    entries.reverse();
    truncate(&mut entries, collection_limit(query.limit, &metadata)?);
    Ok(collection(entries, &metadata))
}
