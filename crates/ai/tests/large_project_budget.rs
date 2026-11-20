//! Regression coverage for project-scoped conversations over large projects.
//!
//! A biotree conversation over 643 animals failed its first counting question
//! with `context_exceeded` because paged read results accumulated beyond the
//! model profile's input budget. These tests replay that model behaviour
//! against a real SQLite store with the same profile limits.

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::Utc;
use muriarc_ai::{
    AccessGrant, AiProvider, AssistantRequest, AssistantRuntimeConfig, AssistantService, ChatRole,
    CompletionRequest, CompletionResponse, DomainToolExecutor, ProviderCredentials, ProviderError,
    ProviderToolCall, ScopeSet, StoreDomainToolExecutor, StoreToolAccessContext, TokenUsage,
    ToolName, ToolScope, estimate_completion_input_tokens,
};
use muriarc_application::{
    CreateAlleleCommand, CreateGeneLocusCommand, CreateGenotypeComponentInput,
    CreateGenotypeDefinitionCommand, create_allele, create_gene_locus, create_genotype_definition,
};
use muriarc_core::{
    Animal, AuditContext, Cage, GenotypeComponentMode, GenotypingRecord, GenotypingState, Lab,
    MuriArcStore, Project, ProjectAnimalAssignment, Sex, WriteSource,
};
use muriarc_store_sqlite::SqliteStore;
use serde_json::{Value, json};
use uuid::Uuid;

const ANIMALS: usize = 643;
const COMBO: &str = "Prox1 Cre+/Cre-; Ai9 tdT+/tdT-";
/// (locus symbol, first allele, second allele)
type Component = (&'static str, &'static str, &'static str);

const DEFINITIONS: [(&str, &[Component]); 6] = [
    ("Prox1 Cre+/Cre+", &[("Prox1", "Cre+", "Cre+")]),
    ("Prox1 Cre+/Cre-", &[("Prox1", "Cre+", "Cre-")]),
    ("Prox1 Cre-/Cre-", &[("Prox1", "Cre-", "Cre-")]),
    ("Ai9 tdT+/tdT+", &[("Ai9", "tdT+", "tdT+")]),
    (COMBO, &[("Prox1", "Cre+", "Cre-"), ("Ai9", "tdT+", "tdT-")]),
    (
        "Prox1 Cre-/Cre-; Ai9 tdT+/tdT-",
        &[("Prox1", "Cre-", "Cre-"), ("Ai9", "tdT+", "tdT-")],
    ),
];

/// Deterministic genotype for synthetic animal `index`; `None` = not genotyped.
fn genotype_of(index: usize) -> Option<&'static str> {
    match index % 7 {
        0 | 1 => Some(COMBO),
        2 => Some("Prox1 Cre-/Cre-; Ai9 tdT+/tdT-"),
        3 => Some("Prox1 Cre+/Cre+"),
        4 => Some("Prox1 Cre+/Cre-"),
        5 => Some("Ai9 tdT+/tdT+"),
        _ => None,
    }
}

/// The biotree model profile that produced the original failure.
fn biotree_runtime() -> AssistantRuntimeConfig {
    AssistantRuntimeConfig {
        context_window_tokens: 131_072,
        max_input_tokens: 65_536,
        max_output_tokens: 4_096,
        history_token_budget: 32_768,
        history_turns: 20,
        temperature: 0.0,
        timeout_ms: 120_000,
    }
}

struct LargeProject {
    store: Arc<SqliteStore>,
    lab_id: Uuid,
    project_id: Uuid,
}

async fn large_project() -> LargeProject {
    let store = Arc::new(SqliteStore::in_memory().await.unwrap());
    store.migrate().await.unwrap();
    let now = Utc::now();
    let audit = AuditContext::system(WriteSource::Api);
    let lab = Lab::new("Lymphatic lab", now).unwrap();
    store.create_lab(&lab, &audit).await.unwrap();
    let project = Project::new(lab.id, "淋巴管基因鼠管理", now).unwrap();
    store.create_project(&project, &audit).await.unwrap();

    let mut loci = BTreeMap::new();
    let mut alleles = BTreeMap::new();
    for (symbol, allele_symbols) in [("Prox1", ["Cre+", "Cre-"]), ("Ai9", ["tdT+", "tdT-"])] {
        let locus = create_gene_locus(
            store.as_ref(),
            CreateGeneLocusCommand {
                lab_id: lab.id,
                symbol: symbol.to_owned(),
                description: None,
                now,
            },
            &audit,
        )
        .await
        .unwrap();
        loci.insert(symbol, locus.id);
        for (index, allele_symbol) in allele_symbols.into_iter().enumerate() {
            let allele = create_allele(
                store.as_ref(),
                CreateAlleleCommand {
                    locus_id: locus.id,
                    symbol: allele_symbol.to_owned(),
                    description: None,
                    is_wild_type: index == 1,
                    now,
                },
                &audit,
            )
            .await
            .unwrap();
            alleles.insert(allele_symbol, allele.id);
        }
    }
    let mut definitions = BTreeMap::new();
    for (name, components) in DEFINITIONS {
        let definition = create_genotype_definition(
            store.as_ref(),
            CreateGenotypeDefinitionCommand {
                lab_id: lab.id,
                name: name.to_owned(),
                description: None,
                components: components
                    .iter()
                    .enumerate()
                    .map(
                        |(order, (locus, first, second))| CreateGenotypeComponentInput {
                            locus_id: loci[locus],
                            allele_1_id: alleles[first],
                            allele_2_id: Some(alleles[second]),
                            mode: GenotypeComponentMode::Diploid,
                            display_order: order as i32,
                        },
                    )
                    .collect(),
                now,
            },
            &audit,
        )
        .await
        .unwrap();
        definitions.insert(name, definition.id);
    }

    let cages = (0..ANIMALS.div_ceil(5))
        .map(|index| Cage::new(lab.id, "淋巴管", format!("L-{index:03}"), now).unwrap())
        .collect::<Vec<_>>();
    for cage in &cages {
        store.create_cage(cage, &audit).await.unwrap();
    }
    let mut assignments = Vec::with_capacity(ANIMALS);
    for index in 0..ANIMALS {
        let sex = if index % 2 == 0 {
            Sex::Female
        } else {
            Sex::Male
        };
        let mut animal = Animal::new_mouse(lab.id, format!("0524-{index:03}"), sex, now).unwrap();
        animal.strain = Some("C57BL/6J".to_owned());
        animal.current_cage_id = Some(cages[index / 5].id);
        let records = genotype_of(index)
            .map(|name| {
                vec![
                    GenotypingRecord::new(
                        lab.id,
                        animal.id,
                        definitions[name],
                        GenotypingState::Expected,
                        None,
                        now,
                    )
                    .unwrap(),
                ]
            })
            .unwrap_or_default();
        store
            .create_animal_with_genotyping_records(&animal, &records, &audit)
            .await
            .unwrap();
        assignments.push(ProjectAnimalAssignment::new(
            lab.id, project.id, animal.id, None, None, now,
        ));
    }
    for batch in assignments.chunks(100) {
        store
            .assign_animals_to_project(batch, &audit)
            .await
            .unwrap();
    }
    LargeProject {
        store,
        lab_id: lab.id,
        project_id: project.id,
    }
}

/// A project-scoped conversation executor, matching the workflow's narrowing
/// of a project conversation (no lab registry access).
fn project_executor(project: &LargeProject) -> StoreDomainToolExecutor {
    StoreDomainToolExecutor::new(
        project.store.clone(),
        StoreToolAccessContext::new(project.lab_id, [project.project_id]),
    )
}

/// Replays a fixed sequence of model responses and records every request.
#[derive(Clone)]
struct ScriptedModel {
    responses: Arc<Mutex<VecDeque<CompletionResponse>>>,
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
}

impl ScriptedModel {
    fn new(turns: Vec<Turn>) -> Self {
        let responses = turns
            .into_iter()
            .map(|turn| CompletionResponse {
                id: None,
                model: Some("scripted".to_owned()),
                content: turn.content.map(str::to_owned),
                tool_calls: turn
                    .calls
                    .into_iter()
                    .map(|(id, tool, arguments)| ProviderToolCall {
                        id: id.to_owned(),
                        name: tool.as_str().to_owned(),
                        arguments,
                    })
                    .collect(),
                finish_reason: None,
                usage: Some(TokenUsage {
                    input_tokens: 1,
                    output_tokens: 1,
                    total_tokens: 2,
                }),
            })
            .collect();
        Self {
            responses: Arc::new(Mutex::new(responses)),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().unwrap().clone()
    }
}

struct Turn {
    content: Option<&'static str>,
    calls: Vec<(&'static str, ToolName, Value)>,
}

fn tools(calls: Vec<(&'static str, ToolName, Value)>) -> Turn {
    Turn {
        content: None,
        calls,
    }
}

fn answer(content: &'static str) -> Turn {
    Turn {
        content: Some(content),
        calls: Vec::new(),
    }
}

#[async_trait]
impl AiProvider for ScriptedModel {
    fn provider_id(&self) -> &str {
        "scripted"
    }

    fn model(&self) -> &str {
        "scripted"
    }

    async fn complete(
        &self,
        request: CompletionRequest,
        _credentials: ProviderCredentials<'_>,
    ) -> Result<CompletionResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(ProviderError::MockExhausted)
    }
}

const QUESTION: &str =
    "本项目一共有多少只动物？其中基因型为 Prox1 Cre+/Cre- 且 Ai9 tdT+/tdT- 的有多少只？";

fn tool_payload(request: &CompletionRequest, call_id: &str) -> Value {
    let message = request
        .messages
        .iter()
        .find(|message| {
            message.role == ChatRole::Tool && message.tool_call_id.as_deref() == Some(call_id)
        })
        .unwrap_or_else(|| panic!("tool result {call_id} missing"));
    serde_json::from_str(&message.content).unwrap()
}

fn assert_every_request_within_budget(model: &ScriptedModel) {
    let requests = model.requests();
    assert!(!requests.is_empty());
    for (index, request) in requests.iter().enumerate() {
        let estimate = estimate_completion_input_tokens(request);
        assert!(
            estimate <= u64::from(biotree_runtime().max_input_tokens),
            "provider request {index} estimated {estimate} input tokens"
        );
    }
}

#[tokio::test]
async fn paging_a_large_project_never_exceeds_the_profile_input_budget() {
    let project = large_project().await;
    let project_id = project.project_id;
    let model = ScriptedModel::new(vec![
        tools(vec![(
            "context",
            ToolName::ProjectContext,
            json!({"project_id": project_id, "limit": 100}),
        )]),
        tools(vec![
            (
                "genotypes",
                ToolName::GenotypingQuery,
                json!({"project_id": project_id, "limit": 100}),
            ),
            (
                "animals",
                ToolName::ResourceSearch,
                json!({"resource": "animals", "project_id": project_id, "limit": 100}),
            ),
        ]),
        tools(vec![(
            "genotypes-2",
            ToolName::GenotypingQuery,
            json!({"project_id": project_id, "limit": 100, "offset": 100}),
        )]),
        answer("这些分页结果不完整，需要使用统计工具。"),
    ]);
    let service = AssistantService::new(model.clone(), project_executor(&project))
        .with_runtime_config(biotree_runtime())
        .unwrap();

    let response = service
        .run(
            AssistantRequest::new(Uuid::new_v4(), QUESTION),
            &AccessGrant::local_user(ScopeSet::new([ToolScope::Read])),
            ProviderCredentials::none(),
        )
        .await
        .expect("large paged reads must not fail the turn with context_exceeded");

    assert!(response.incomplete_reason.is_none());
    assert_eq!(response.tool_runs.len(), 4);
    assert_every_request_within_budget(&model);
    assert!(response.citations.len() <= service.limits().max_citations);
    let requests = model.requests();
    let context = tool_payload(&requests[1], "context");
    assert_eq!(context["truncated"], true);
    assert!(
        context["truncation"]["notice"]
            .as_str()
            .unwrap()
            .contains("animal_statistics")
    );
    assert!(
        response
            .context
            .trim_reasons
            .iter()
            .any(|reason| reason == "tool_result_truncated")
    );
}

#[tokio::test]
async fn counting_question_over_643_animals_is_answered_by_bounded_statistics() {
    let project = large_project().await;
    let project_id = project.project_id;
    let executor = project_executor(&project);
    assert!(
        executor
            .supported_tools()
            .contains(&ToolName::AnimalStatistics)
    );
    let model = ScriptedModel::new(vec![
        // The model first reaches for the broad context read that overflowed
        // in production, then switches to the fixed aggregate tool.
        tools(vec![(
            "context",
            ToolName::ProjectContext,
            json!({"project_id": project_id, "limit": 100}),
        )]),
        tools(vec![
            (
                "by-genotype",
                ToolName::AnimalStatistics,
                json!({"project_id": project_id, "group_by": ["genotype_definition"]}),
            ),
            (
                "combo",
                ToolName::AnimalStatistics,
                json!({"project_id": project_id, "genotype_definitions": [COMBO]}),
            ),
        ]),
        answer("本项目共有 643 只动物。"),
    ]);
    let service = AssistantService::new(model.clone(), executor)
        .with_runtime_config(biotree_runtime())
        .unwrap();
    let access = AccessGrant::local_user(ScopeSet::new([ToolScope::Read]));
    assert!(
        service
            .visible_tools(&access)
            .iter()
            .any(|tool| tool.name == "animal_statistics")
    );

    let response = service
        .run(
            AssistantRequest::new(Uuid::new_v4(), QUESTION),
            &access,
            ProviderCredentials::none(),
        )
        .await
        .unwrap();

    assert!(response.incomplete_reason.is_none());
    assert_every_request_within_budget(&model);
    let expected_combo = (0..ANIMALS)
        .filter(|index| genotype_of(*index) == Some(COMBO))
        .count();
    let final_request = model.requests().pop().unwrap();

    let grouped = tool_payload(&final_request, "by-genotype");
    assert!(grouped.get("truncated").is_none(), "{grouped}");
    assert_eq!(grouped["data"]["animals_in_scope"], ANIMALS);
    assert_eq!(grouped["data"]["matching_animals"], ANIMALS);
    let groups = grouped["data"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| {
            (
                group["key"]["genotype_definition"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                group["animals"].as_u64().unwrap() as usize,
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(groups[COMBO], expected_combo);
    assert_eq!(groups.values().sum::<usize>(), ANIMALS);

    let combo = tool_payload(&final_request, "combo");
    assert!(combo.get("truncated").is_none(), "{combo}");
    assert_eq!(combo["data"]["animals_in_scope"], ANIMALS);
    assert_eq!(combo["data"]["matching_animals"], expected_combo);
    // Aggregates cite the project and definitions, never 643 animal rows.
    assert!(combo["citations"].as_array().unwrap().len() <= 8);
}
