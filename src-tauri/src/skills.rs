//! Structured, backend-owned chat actions. No provider output is executable code.
use crate::{ai_chat, core::GoalDraft, goals::Step, storage::Storage};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
mod enhancements;
pub const WORKFLOW_PROMPT: &str = "For new goals prefer create_planned_goal: date=null keeps a backlog goal; an explicit local YYYY-MM-DD date schedules it. New goals have no ID in the supplied workspace. In an ordered creation plan, subsequent steps, scheduling and focus for that new goal must use last_created (or previous_action when its immediately preceding result is the new goal), never an existing goal ID or a guessed next ID. Creation must precede those dependent actions. Earlier last_created/active/parked references do not identify a goal that has not been created yet. Existing goals can remain parked while another goal is created. Existing-goal mutations belong before creation; after creation, mutate only goals reached through that creation result. Existing-goal reads remain available. Preserve a different existing target only when the user actually requested it. Use state.date and local_timezone_offset for relative dates and clarify ambiguity. update_goal changes only explicitly requested fields; null keeps a field and clear_fields removes it. Scheduling is distinct from deadlines and focus. reopen_goal preserves progress; restart_goal resets all steps only when starting over was explicitly requested and requires backend confirmation. complete_goal also creates a backend confirmation preview. find_goals searches all goals beyond the bounded workspace: use filters and pagination instead of requesting an unbounded index. Retain last_goal_search filters and advance offset for the next page. reorder_steps must include every real step ID exactly once; set_current_step selects an unfinished step without a timer. update_work_context records explicit user facts without completion, parking or focus. A reported result is not verified completion. inspect_research reads a saved ready receipt; add_research_step requires explicit approval to add its next action as unfinished. undo_last_goal_edit undoes the latest supported single edit in this chat; reads do not grant mutation permission or invalidate that journal. It is a separate operation; deletes use restore_goal. Goal sharing off allows only creation from the user's supplied content and conversation reads. Preserve existing scope, duplicate-target, authorization, state and response review guards. Never infer object IDs or completion.";

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Safety {
    ReadOnly,
    Reversible,
    Confirmation,
}

pub struct Skill {
    pub name: &'static str,
    pub description: &'static str,
    pub safety: Safety,
    pub input_schema: Value,
    pub result_schema: Value,
    pub validator: fn(&Skill, &Value) -> Result<Input, String>,
    pub executor:
        fn(&Skill, &mut Storage, Input, &mut References, i64) -> Result<SkillResult, String>,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct References {
    pub last_referenced_goal_id: Option<i64>,
    pub last_referenced_step_id: Option<String>,
    pub last_created_goal_id: Option<i64>,
    pub last_created_step_id: Option<String>,
    pub last_created_step_goal_id: Option<i64>,
    pub last_action: Option<Value>,
    pub last_referenced_goal_created_at: Option<String>,
    pub last_created_goal_created_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalTarget {
    pub goal_id: Option<i64>,
    pub reference: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepTarget {
    pub step_id: Option<String>,
    pub ordinal: Option<usize>,
    pub reference: Option<String>,
}
#[derive(Clone, Default, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(skip)]
    pub extension: Option<enhancements::Input>,
    pub goal: Option<GoalTarget>,
    pub step: Option<StepTarget>,
    pub title: Option<String>,
    pub text: Option<String>,
    pub steps: Option<Vec<String>>,
    pub backup_id: Option<i64>,
    pub checkpoint: Option<crate::work_context::CheckpointDraft>,
    pub current_app: Option<String>,
    pub set_focus: Option<bool>,
    pub research_id: Option<i64>,
    pub helpful: Option<bool>,
    pub view: Option<crate::continuity::View>,
    pub query: Option<String>,
    pub before_message_id: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub name: String,
    pub input: Value,
    pub instruction: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub kind: String,
    pub state_version: String,
    pub actions: Vec<Action>,
    pub clarification: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillResult {
    pub skill: String,
    pub goal_id: Option<i64>,
    pub step_id: Option<String>,
    pub data: Value,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: String,
    pub data: Value,
    pub references: References,
    pub conversation: i64,
    pub privacy_revision: Option<u64>,
}
#[derive(Serialize, Deserialize)]
struct Pending {
    plan: Plan,
    snapshot: Snapshot,
    expires_at: i64,
}
#[derive(Serialize)]
pub struct Execution {
    pub status: &'static str,
    pub results: Vec<SkillResult>,
    pub message: Option<String>,
}
impl Execution {
    pub fn committed_changes(&self) -> bool {
        self.status == "executed"
            && self.results.iter().any(|r| {
                registry()
                    .iter()
                    .any(|s| s.name == r.skill && s.safety != Safety::ReadOnly)
            })
    }
    pub fn fallback_message(&self) -> String {
        let mut lines = Vec::new();
        if self.status == "confirmation_required" {
            lines.push("Proposed changes only. Nothing has changed yet.".into());
        }
        for r in &self.results {
            let title = r.data["title"].as_str().unwrap_or("goal");
            let text = r.data["removed_step"]["text"]
                .as_str()
                .or_else(|| {
                    r.data["steps"]
                        .as_array()
                        .and_then(|steps| {
                            steps
                                .iter()
                                .find(|s| s["id"].as_str() == r.step_id.as_deref())
                        })
                        .and_then(|s| s["text"].as_str())
                })
                .unwrap_or("step");
            let line=enhancements::receipt(r, self.status).unwrap_or_else(|| match r.skill.as_str() {
                "inspect_conversation" => "Read retained messages from this conversation.".into(),
                "inspect_work_queue" => work_queue_receipt(&r.data),
                "inspect_continuity" => r.data["report"].as_str().unwrap_or_default().to_string(),
                "search_web" => format!("Search the web for \"{}\"{} after confirmation.",r.data["title"].as_str().unwrap_or("this goal"),r.data["step_text"].as_str().map(|s|format!(" — step: {s}")).unwrap_or_default()),
                "attach_research_to_context" => "Attached the validated research to this work context.".into(),
                "record_research_feedback" => "Recorded your research feedback.".into(),
                "create_goal"=>format!("Created \"{title}\" and added it to Today."),
                "rename_goal"=>format!("Renamed the goal to \"{title}\"."),
                "delete_goal"=>format!("Deleted \"{title}\". You can ask to restore it within 30 days, subject to your history retention settings."),
                "restore_goal"=>format!("Restored \"{title}\"."),
                "create_step"=>format!("Added \"{text}\" to \"{title}\"."),
                "rename_step"=>format!("Renamed the step to \"{text}\" in \"{title}\"."),
                "complete_step"=>format!("Marked \"{text}\" complete in \"{title}\"."),
                "reopen_step"=>format!("Reopened \"{text}\" in \"{title}\"."),
                "delete_step"=>format!("{} \"{text}\" from \"{title}\".",if self.status=="executed" {"Deleted"} else {"Delete"}),
                "complete_goal"=>format!("Completed \"{title}\" and its steps."),
                "reopen_goal"=>format!("Reopened \"{title}\"."),
                "add_goal_to_today"=>format!("Added \"{title}\" to Today."),
                "remove_goal_from_today"=>format!("Removed \"{title}\" from Today."),
                "set_focus_goal"=>format!("Selected \"{title}\" for activity tracking."),
                "park_context" | "resume_context" | "inspect_work_context" => crate::work_context::receipt(&r.skill,title,&r.data["checkpoint"]),
                "inspect_goal"=>format!("{title}\n{}\n{}",r.data["steps"].as_array().unwrap().iter().enumerate().map(|(i,s)|format!("{}. {}{}",i+1,s["text"].as_str().unwrap_or_default(),if s["done"]==true {" (complete)"} else {""})).collect::<Vec<_>>().join("\n"), crate::work_context::receipt("inspect_work_context",title,&r.data["checkpoint"])),
                "inspect_today"=>format!("Today ({})\n{}\nRelevant activity: {} min. Observed activity: {} min. Manual timer: {} min. Time alone does not confirm completion.",
                    r.data["day"].as_str().unwrap_or_default(),
                    if r.data["confirmed_results"].as_array().unwrap().is_empty() { "No confirmed completion is recorded today.".into() }
                    else { r.data["confirmed_results"].as_array().unwrap().iter().map(|v|format!("• {}",v.as_str().unwrap_or_default())).collect::<Vec<_>>().join("\n") },
                    r.data["relevant_seconds"].as_u64().unwrap_or_default()/60,
                    r.data["observed_seconds"].as_u64().unwrap_or_default()/60,
                    r.data["timer_seconds"].as_u64().unwrap_or_default()/60),
                "list_goals"=>r.data["goals"].as_array().unwrap().iter().map(|g|g["title"].as_str().unwrap_or_default()).collect::<Vec<_>>().join("\n"),
                _=>"Completed the requested action.".into(),
            });
            let line = if self.status == "confirmation_required" {
                let mut proposed = line.clone();
                for (past, future) in [
                    ("Created ", "Create "),
                    ("Renamed ", "Rename "),
                    ("Deleted ", "Delete "),
                    ("Restored ", "Restore "),
                    ("Added ", "Add "),
                    ("Marked ", "Mark "),
                    ("Reopened ", "Reopen "),
                    ("Completed ", "Complete "),
                    ("Removed ", "Remove "),
                    ("Selected ", "Select "),
                    ("Updated ", "Update "),
                    ("Reordered ", "Reorder "),
                    ("Restarted ", "Restart "),
                    ("Undid ", "Undo "),
                    ("Cleared ", "Clear "),
                    ("Saved ", "Save "),
                    ("Parked ", "Park "),
                    ("Resumed ", "Resume "),
                ] {
                    if let Some(rest) = line.strip_prefix(past) {
                        proposed = format!("{future}{rest}");
                        break;
                    }
                }
                proposed
            } else {
                line
            };
            lines.push(line);
        }
        if let Some(message) = &self.message {
            lines.push(message.clone());
        }
        if lines.iter().all(String::is_empty) {
            "No goals found.".into()
        } else {
            lines.join("\n\n")
        }
    }
}

fn result_schema(name: &str) -> Value {
    let queue_item = shape(
        json!({"goal_id":{"type":"integer"},"title":{"type":"string"},"remaining_step_count":{"type":"integer"},
        "next_step":{"anyOf":[shape(json!({"step_id":{"type":"string"},"text":{"type":"string"}})),{"type":"null"}]}}),
    );
    let data = match name {
        "inspect_conversation" => shape(
            json!({"messages":{"type":"array","maxItems":8,"items":shape(json!({
            "id":{"type":"integer"},"role":{"type":"string"},"text":{"type":"string"},"clipped":{"type":"boolean"}}))},
            "next_before_message_id":{"type":["integer","null"]},"has_more":{"type":"boolean"},"scanned_messages":{"type":"integer"}}),
        ),
        "inspect_continuity" => shape(
            json!({"day":{"type":"string"},"report":{"type":"string"},"omitted_lanes":{"type":"integer"}}),
        ),
        "search_web" | "attach_research_to_context" | "record_research_feedback" => shape(
            json!({"research_id":{"type":"integer"},"status":{"type":"string"},"title":{"type":"string"},"created_at":{"type":"string"},"step_text":{"type":["string","null"]}}),
        ),
        "park_context" | "resume_context" | "inspect_work_context" => shape(
            json!({"title":{"type":"string"},"status":{"type":"string"},"created_at":{"type":"string"},"checkpoint":{"anyOf":[crate::work_context::public_checkpoint_schema(),{"type":"null"}]}}),
        ),
        "inspect_today" => shape(json!({
            "day":{"type":"string"},"confirmed_results":{"type":"array","items":{"type":"string"}},
            "completed_goals":{"type":"array","items":{"type":"string"}},
            "timer_seconds":{"type":"integer"},"observed_seconds":{"type":"integer"},"relevant_seconds":{"type":"integer"},
            "remaining_goals":{"type":"array","items":shape(json!({"title":{"type":"string"},"steps":{"type":"array","items":{"type":"string"}}}))},
            "waiting_goals":{"type":"array","items":shape(json!({"title":{"type":"string"},"waiting_for":{"type":"string"}}))},"omitted_goals":{"type":"integer"}
        })),
        "inspect_work_queue" => shape(
            json!({"day":{"type":"string"},"in_focus":{"anyOf":[queue_item.clone(),{"type":"null"}]},
            "later_today":{"type":"array","maxItems":40,"items":queue_item},
            "up_next_goal_id":{"type":["integer","null"]},"omitted_goals":{"type":"integer"}}),
        ),
        "list_goals" => shape(
            json!({"goals":{"type":"array","items":shape(json!({"goal_id":{"type":"integer"},"title":{"type":"string"},"status":{"type":"string"}}))}}),
        ),
        "delete_goal" => shape(
            json!({"title":{"type":"string"},"backup_id":{"type":"integer"},"undo":{"type":"string","const":"restore_goal"},"undo_days":{"type":"integer","const":30}}),
        ),
        _ => enhancements::goal_schema_result(),
    };
    shape(
        json!({"skill":{"type":"string","const":name},"goal_id":{"type":["integer","null"]},"step_id":{"type":["string","null"]},"data":data}),
    )
}

/// Validate the small JSON Schema subset used by backend result contracts.
fn valid_result(schema: &Value, value: &Value) -> bool {
    if let Some(alternatives) = schema["anyOf"].as_array() {
        return alternatives.iter().any(|s| valid_result(s, value));
    }
    if schema.get("const").is_some_and(|c| c != value) {
        return false;
    }
    let matches = |kind: &str| match kind {
        "null" => value.is_null(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => false,
    };
    let correct_type = if let Some(kind) = schema["type"].as_str() {
        matches(kind)
    } else {
        schema["type"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v.as_str().is_some_and(matches)))
    };
    if !correct_type {
        return false;
    }
    if let Some(fields) = value.as_object() {
        let properties = schema["properties"].as_object().unwrap();
        if fields.len() != properties.len() {
            return false;
        }
        return fields
            .iter()
            .all(|(key, value)| properties.get(key).is_some_and(|s| valid_result(s, value)));
    }
    if let Some(items) = value.as_array() {
        return items.iter().all(|v| valid_result(&schema["items"], v));
    }
    true
}

fn object(properties: Value, required: Vec<String>) -> Value {
    json!({"type":"object","additionalProperties":false,"properties":properties,"required":required})
}
fn shape(properties: Value) -> Value {
    let keys = properties.as_object().unwrap().keys().cloned().collect();
    object(properties, keys)
}
fn goal_schema() -> Value {
    let reference = json!({"type":"string","description":"For a new goal created earlier in this ordered plan, use last_created. A numeric goal_id from the workspace refers to an existing goal, never that new goal.","enum":["last_referenced","last_created","previous_action","only_goal","active_goal","only_parked"]});
    json!({"anyOf":[
        shape(json!({"goal_id":{"type":"integer","minimum":1},"reference":{"type":"null"}})),
        shape(json!({"goal_id":{"type":"null"},"reference":reference}))
    ]})
}
fn step_schema() -> Value {
    json!({"anyOf":[
        shape(json!({"step_id":{"type":"string","minLength":1,"maxLength":80},"ordinal":{"type":"null"},"reference":{"type":"null"}})),
        shape(json!({"step_id":{"type":"null"},"ordinal":{"type":"integer","minimum":1,"maximum":20},"reference":{"type":"null"}})),
        shape(json!({"step_id":{"type":"null"},"ordinal":{"type":"null"},"reference":{"type":"string","enum":["last_referenced","last_created","previous_action"]}}))
    ]})
}
pub fn registry() -> Vec<Skill> {
    let text = json!({"type":"string","minLength":1,"maxLength":500});
    let goal = goal_schema();
    let step = step_schema();
    let mut skills: Vec<Skill> = [
        ("inspect_conversation", "Read older retained messages ONLY from this chat. Use a short literal query (Unicode case insensitive), or empty query for a page; before_message_id is a pagination cursor or null. Results may be clipped; never treat assistant guesses as user facts.", Safety::ReadOnly, json!({"query":{"type":"string","maxLength":200},"before_message_id":{"type":["integer","null"],"minimum":1}})),
        ("search_web", "Search for information for this goal and optional step; requires explicit web-search confirmation before any provider call", Safety::Confirmation, json!({"goal":goal,"step":{"anyOf":[step,{"type":"null"}]},"text":text})),
        ("attach_research_to_context", "Attach an existing validated research receipt to its original goal checkpoint; never invent research content", Safety::Reversible, json!({"goal":goal,"research_id":{"type":"integer","minimum":1}})),
        ("record_research_feedback", "Record whether this exact goal research helped; no completion is implied", Safety::Reversible, json!({"goal":goal,"research_id":{"type":"integer","minimum":1},"helpful":{"type":"boolean"}})),
        ("inspect_continuity", "Read grounded daily continuity: priorities, yesterday results/checkpoints, waiting items, remaining goal steps or end-of-day summary; optional exact goal target", Safety::ReadOnly, json!({"view":{"type":"string","enum":["priorities","yesterday","waiting","remaining","end_of_day"]},"goal":{"anyOf":[goal,{"type":"null"}]}})),
        ("inspect_today", "Read today's confirmed results and separate timer, observed and relevant time; time alone is not completion", Safety::ReadOnly, json!({})),
        ("inspect_work_queue", "Read the actual Today queue: In focus, ordered Later today, and Up next; does not change focus", Safety::ReadOnly, json!({})),
        ("park_context","Save a short checkpoint and park this goal before switching work",Safety::Reversible,json!({"goal":goal,"checkpoint":{"anyOf":[crate::work_context::checkpoint_schema(),{"type":"null"}]},"current_app":{"type":["string","null"],"maxLength":120}})),
        ("resume_context","Change a parked goal back to active, record its resumption and restore its saved checkpoint; optionally focus it without starting a timer. Reading a checkpoint alone does not resume or focus the goal",Safety::Reversible,json!({"goal":goal,"set_focus":{"type":"boolean"}})),
        ("inspect_work_context","Read where work stopped, the last meaningful result, next action and waiting state",Safety::ReadOnly,json!({"goal":goal})),
        (
            "list_goals",
            "List actual goals and stable IDs",
            Safety::ReadOnly,
            json!({}),
        ),
        (
            "inspect_goal",
            "Inspect a specific goal and ordered steps with IDs",
            Safety::ReadOnly,
            json!({"goal":goal}),
        ),
        (
            "create_goal",
            "Create a goal with optional steps and add it to Today",
            Safety::Reversible,
            json!({"title":text,"steps":{"type":"array","maxItems":20,"items":text}}),
        ),
        (
            "rename_goal",
            "Rename one goal by ID",
            Safety::Reversible,
            json!({"goal":goal,"title":text}),
        ),
        (
            "delete_goal",
            "Delete a goal with a 30 day local undo backup",
            Safety::Reversible,
            json!({"goal":goal}),
        ),
        (
            "restore_goal",
            "Restore the exact deleted goal backup by backup_id",
            Safety::Reversible,
            json!({"backup_id":{"type":"integer","minimum":1}}),
        ),
        (
            "add_goal_to_today",
            "Add an open goal to Today",
            Safety::Reversible,
            json!({"goal":goal}),
        ),
        (
            "remove_goal_from_today",
            "Remove a goal from Today",
            Safety::Reversible,
            json!({"goal":goal}),
        ),
        (
            "set_focus_goal",
            "Choose an open Today goal for activity tracking without starting a timer",
            Safety::Reversible,
            json!({"goal":goal}),
        ),
        (
            "complete_goal",
            "Complete a goal and ALL steps; preview consequences and require confirmation",
            Safety::Confirmation,
            json!({"goal":goal}),
        ),
        (
            "reopen_goal",
            "Reopen a goal preserving step progress; use restart_goal for an explicit full reset",
            Safety::Reversible,
            json!({"goal":goal}),
        ),
        (
            "create_step",
            "Append one step to a goal",
            Safety::Reversible,
            json!({"goal":goal,"text":text}),
        ),
        (
            "rename_step",
            "Rename a step belonging to the supplied goal",
            Safety::Reversible,
            json!({"goal":goal,"step":step,"text":text}),
        ),
        (
            "delete_step",
            "Delete a step only after explicit confirmation of the exact batch",
            Safety::Confirmation,
            json!({"goal":goal,"step":step}),
        ),
        (
            "complete_step",
            "Complete a step belonging to the supplied goal",
            Safety::Reversible,
            json!({"goal":goal,"step":step}),
        ),
        (
            "reopen_step",
            "Reopen a step belonging to the supplied goal",
            Safety::Reversible,
            json!({"goal":goal,"step":step}),
        ),
    ]
    .into_iter()
    .map(|(name, description, safety, properties)| Skill {
        name,
        description,
        safety,
        input_schema: shape(properties),
        result_schema: result_schema(name),
        validator: validate_input,
        executor: execute_one,
    })
    .collect();
    skills.extend(enhancements::registry());
    skills
}
/// Argument/result contracts already exist in the response schema and Rust registry.
/// Avoid duplicating their full schemas in every user message to the planner.
pub fn planning_catalog() -> Value {
    json!(registry()
        .into_iter()
        .map(|s| json!({"name":s.name,"description":s.description,"permission":s.safety}))
        .collect::<Vec<_>>())
}
pub fn plan_schema() -> Value {
    let alternatives: Vec<_> = registry().into_iter().map(|s|shape(json!({"name":{"type":"string","const":s.name},"input":s.input_schema,"instruction":{"type":["string","null"],"maxLength":4000}}))).collect();
    shape(
        json!({"kind":{"type":"string","enum":["chat","actions","clarification"]},"state_version":{"type":"string"},"actions":{"type":"array","maxItems":20,"items":{"anyOf":alternatives}},"clarification":{"type":["string","null"],"maxLength":500}}),
    )
}
#[cfg(test)]
pub fn parse_plan(response: &Value) -> Result<Plan, String> {
    let plan: Plan = serde_json::from_str(crate::goal_analysis::response_text(response)?)
        .map_err(|_| "sumi returned an invalid action plan; nothing was changed")?;
    if !["chat", "actions", "clarification"].contains(&plan.kind.as_str())
        || plan.state_version.is_empty()
        || plan.actions.len() > 20
        || (plan.kind == "actions") != !plan.actions.is_empty()
        || (plan.kind == "clarification") != plan.clarification.is_some()
        || plan
            .clarification
            .as_ref()
            .is_some_and(|q| q.trim().is_empty() || q.chars().count() > 500)
    {
        return Err("Invalid action plan fields; nothing was changed".into());
    }
    for action in &plan.actions {
        let skill = registry()
            .into_iter()
            .find(|s| s.name == action.name)
            .ok_or("Unknown sumi skill")?;
        (skill.validator)(&skill, &action.input)?;
    }
    Ok(plan)
}
fn validate_input(skill: &Skill, value: &Value) -> Result<Input, String> {
    if enhancements::is_skill(skill.name) {
        return enhancements::validate(skill, value).map(|extension| Input {
            goal: extension.goal.clone(),
            step: extension.step.clone(),
            extension: Some(extension),
            ..Input::default()
        });
    }
    let fields = value.as_object().ok_or("Skill input must be an object")?;
    let properties = skill.input_schema["properties"].as_object().unwrap();
    if fields.len() != properties.len()
        || fields.keys().any(|k| !properties.contains_key(k))
        || fields.iter().any(|(key, value)| {
            value.is_null()
                && !(skill.name == "park_context"
                    && ["checkpoint", "current_app"].contains(&key.as_str()))
                && !(skill.name == "search_web" && key == "step")
                && !(skill.name == "inspect_continuity" && key == "goal")
                && !(skill.name == "inspect_conversation" && key == "before_message_id")
        })
    {
        return Err("Skill input does not match its schema".into());
    }
    let input: Input =
        serde_json::from_value(value.clone()).map_err(|_| "Invalid skill arguments")?;
    if skill.name == "inspect_conversation"
        && (input.query.as_ref().is_none_or(|s| s.chars().count() > 200)
            || input.before_message_id.is_some_and(|id| id <= 0))
    {
        return Err("Invalid conversation history query".into());
    }
    for value in [&input.title, &input.text].into_iter().flatten() {
        crate::goals::text(value, true)?;
    }
    if input.research_id.is_some_and(|id| id <= 0) {
        return Err("Invalid research ID".into());
    }
    if skill.name == "search_web" {
        crate::memory::safe_text(
            input
                .text
                .as_deref()
                .ok_or("Describe the information needed")?,
            500,
        )?;
    }
    if let Some(steps) = &input.steps {
        if steps.len() > 20 {
            return Err("Use at most 20 steps".into());
        }
        for step in steps {
            crate::goals::text(step, true)?;
        }
    }
    if let Some(goal) = &input.goal {
        if value["goal"].as_object().is_none_or(|o| o.len() != 2) {
            return Err("Invalid goal reference schema".into());
        }
        if goal.goal_id.is_some_and(|id| id <= 0)
            || (goal.goal_id.is_some() == goal.reference.is_some())
            || goal.reference.as_ref().is_some_and(|r| {
                ![
                    "last_referenced",
                    "last_created",
                    "previous_action",
                    "only_goal",
                    "active_goal",
                    "only_parked",
                ]
                .contains(&r.as_str())
            })
        {
            return Err("Supply one unambiguous goal reference".into());
        }
    }
    if let Some(step) = &input.step {
        if value["step"].as_object().is_none_or(|o| o.len() != 3) {
            return Err("Invalid step reference schema".into());
        }
        if usize::from(step.step_id.is_some())
            + usize::from(step.ordinal.is_some())
            + usize::from(step.reference.is_some())
            != 1
            || step.ordinal.is_some_and(|n| !(1..=20).contains(&n))
            || step
                .step_id
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.len() > 80)
            || step.reference.as_ref().is_some_and(|r| {
                !["last_referenced", "last_created", "previous_action"].contains(&r.as_str())
            })
        {
            return Err("Supply one unambiguous step reference".into());
        }
    }
    if input
        .goal
        .as_ref()
        .and_then(|g| g.reference.as_deref())
        .is_some_and(|r| ["active_goal", "only_parked"].contains(&r))
        && !["park_context", "resume_context", "inspect_work_context"].contains(&skill.name)
    {
        return Err("Work-context references are only available to workflow skills".into());
    }
    if let Some(checkpoint) = &input.checkpoint {
        checkpoint.clone().validate()?;
    }
    if let Some(app) = &input.current_app {
        crate::memory::safe_text(app, 120)?;
    }
    if input.backup_id.is_some_and(|id| id <= 0) {
        return Err("Invalid backup ID".into());
    }
    Ok(input)
}

impl Storage {
    pub fn initialize_skills(&self) -> Result<(), String> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS buddy_action_state(conversation_id INTEGER PRIMARY KEY REFERENCES buddy_conversations(id) ON DELETE CASCADE,references_json TEXT NOT NULL,pending_json TEXT);
            CREATE TABLE IF NOT EXISTS buddy_action_version(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL);
            INSERT OR IGNORE INTO buddy_action_version(id,revision) VALUES(1,0);").map_err(|e|e.to_string())?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS buddy_goal_edit_undo(conversation_id INTEGER PRIMARY KEY REFERENCES buddy_conversations(id) ON DELETE CASCADE,payload TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS buddy_goal_search(conversation_id INTEGER PRIMARY KEY REFERENCES buddy_conversations(id) ON DELETE CASCADE,payload TEXT NOT NULL);").map_err(|e|e.to_string())?;
        // Changes through existing UI/IPC operations must invalidate plans too. Time ticks do not.
        for table in [
            "goals",
            "goal_plans",
            "core_goals",
            "core_day_items",
            "core_days",
            "core_goal_details",
            "core_goal_reopen_overrides",
            "deleted_goal_backups",
            "work_contexts",
            "contextual_research",
        ] {
            let exists: bool = self
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                    [table],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !exists {
                continue;
            }
            for event in ["INSERT", "UPDATE", "DELETE"] {
                self.connection.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS buddy_version_{table}_{event} AFTER {event} ON {table} BEGIN UPDATE buddy_action_version SET revision=revision+1 WHERE id=1; END;")).map_err(|e|e.to_string())?;
            }
        }
        Ok(())
    }
    pub fn action_references(&self, conversation: i64) -> Result<References, String> {
        let row: Option<String> = self
            .connection
            .query_row(
                "SELECT references_json FROM buddy_action_state WHERE conversation_id=?1",
                [conversation],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        row.map(|s| serde_json::from_str(&s).map_err(|_| "Invalid conversation references".into()))
            .unwrap_or(Ok(References::default()))
    }
    pub(crate) fn save_references(
        &self,
        conversation: i64,
        refs: &References,
    ) -> Result<(), String> {
        self.connection.execute("INSERT INTO buddy_action_state(conversation_id,references_json) VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET references_json=excluded.references_json",params![conversation,serde_json::to_string(refs).map_err(|e|e.to_string())?]).map(|_|()).map_err(|e|e.to_string())
    }
}
pub fn snapshot(storage: &Storage, conversation: i64) -> Result<Snapshot, String> {
    if !storage.conversation_exists(conversation) {
        return Err("That chat no longer exists".into());
    }
    let core = storage.core_snapshot(None)?;
    let refs = storage.action_references(conversation)?;
    let goals: Vec<_> = core.goals.iter().map(|g|json!({"goal_id":g.id,"created_at":g.created_at,"title":g.title,"status":g.status,"area_id":g.area_id,"plan":g.plan,"description":g.description,"due_at":g.due_at,"priority":g.priority,"has_work_context":g.work_context.is_some(),"parked":g.status=="deferred" && g.work_context.as_ref().is_some_and(|c|c.parked_at.is_some())})).collect();
    let mut q = storage
        .connection
        .prepare("SELECT id,payload FROM deleted_goal_backups WHERE deleted_at>=?1 ORDER BY id")
        .map_err(|e| e.to_string())?;
    let backups = q.query_map([chrono::Utc::now().timestamp()-ai_chat::KEEP_DELETED_SECONDS],|r| {
        let payload:String=r.get(1)?;
        let backup:ai_chat::GoalBackup=serde_json::from_str(&payload).map_err(|e|rusqlite::Error::FromSqlConversionFailure(1,rusqlite::types::Type::Text,Box::new(e)))?;
        Ok(json!({"backup_id":r.get::<_,i64>(0)?,"original_goal_id":backup.plan.goal_id,"title":backup.title}))
    }).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let database_revision: i64 = storage
        .connection
        .query_row(
            "SELECT revision FROM buddy_action_version WHERE id=1",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(Snapshot {
        version: format!(
            "{}-{}",
            conversation,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ),
        data: json!({"goals":goals,"today":core.today,"focused_goal_id":core.focused_goal_id,"work_queue":work_queue_from_core(&core),"date":core.date,"backups":backups,"references":refs,"database_revision":database_revision,"last_goal_edit":enhancements::journal_context(storage,conversation)?,"last_goal_search":enhancements::search_context(storage,conversation)?}),
        references: refs,
        conversation,
        privacy_revision: None,
    })
}
#[cfg(test)]
pub fn planning_context(snap: &Snapshot, share: bool) -> Value {
    if !share {
        return json!({"state_version":snap.version,"sharing":false,"goals":[],"references":References::default(),"deleted_goals":[]});
    }
    let mut goals = snap.data["goals"].as_array().cloned().unwrap_or_default();
    goals.sort_by_key(|g| {
        Some(g["goal_id"].as_i64().unwrap_or_default()) != snap.references.last_referenced_goal_id
    });
    goals.truncate(40);
    let deleted = &snap.data["backups"];
    json!({"state_version":snap.version,"sharing":true,"goals":goals,"today":snap.data["today"],"work_queue":snap.data["work_queue"],"references":snap.references,"deleted_goals":deleted,"focused_goal_id":snap.data["focused_goal_id"],"total_goals":snap.data["goals"].as_array().unwrap().len()})
}

fn work_queue_from_core(core: &crate::core::CoreSnapshot) -> Value {
    let item = |g: &crate::core::CoreGoal| {
        json!({"goal_id":g.id,"title":g.title,
        "remaining_step_count":g.plan.steps.iter().filter(|s| !s.done).count(),
        "next_step":g.plan.steps.iter().find(|s| !s.done).map(|s| json!({"step_id":s.id,"text":s.text}))})
    };
    let open: Vec<_> = core
        .goals
        .iter()
        .filter(|g| g.status == "open" && core.today.contains(&g.id))
        .collect();
    let focus = open
        .iter()
        .find(|g| Some(g.id) == core.focused_goal_id)
        .map(|g| item(g));
    let later: Vec<_> = open
        .iter()
        .filter(|g| Some(g.id) != core.focused_goal_id)
        .collect();
    json!({"day":core.date,"in_focus":focus,"later_today":later.iter().take(40).map(|g| item(g)).collect::<Vec<_>>(),
        "up_next_goal_id":later.first().map(|g|g.id),"omitted_goals":later.len().saturating_sub(40)})
}

fn work_queue_receipt(queue: &Value) -> String {
    let mut lines = vec![format!(
        "In focus: {}",
        queue["in_focus"]["title"]
            .as_str()
            .unwrap_or("No open goal is in focus.")
    )];
    let later = queue["later_today"].as_array().unwrap();
    if later.is_empty() {
        lines.push("Later today: No open goals are queued.".into());
    } else {
        lines.push("Later today (Up next is the first goal):".into());
        for (i, g) in later.iter().enumerate() {
            lines.push(format!(
                "{}. {}{}",
                i + 1,
                g["title"].as_str().unwrap_or_default(),
                g["next_step"]["text"]
                    .as_str()
                    .map(|s| format!(" — next step: {s}"))
                    .unwrap_or_default()
            ));
        }
    }
    if queue["omitted_goals"].as_u64().unwrap_or_default() > 0 {
        lines.push(format!(
            "{} additional goals are not shown.",
            queue["omitted_goals"]
        ));
    }
    lines.push("Focus and goal order are unchanged.".into());
    lines.join("\n")
}

fn resolve_goal(storage: &Storage, target: &GoalTarget, refs: &References) -> Result<i64, String> {
    let id = match target.reference.as_deref() {
        Some("last_referenced") => refs.last_referenced_goal_id,
        Some("last_created") => refs.last_created_goal_id,
        Some("previous_action") => refs
            .last_action
            .as_ref()
            .and_then(|a| a["goal_id"].as_i64()),
        Some("active_goal") => storage.goal()?.map(|g| g.id),
        Some("only_parked") => {
            let parked: Vec<i64> = storage
                .core_snapshot(None)?
                .goals
                .iter()
                .filter(|g| {
                    g.status == "deferred"
                        && g.work_context
                            .as_ref()
                            .is_some_and(|c| c.parked_at.is_some())
                })
                .map(|g| g.id)
                .collect();
            if parked.len() != 1 {
                return Err(
                    "Which parked goal do you mean? Please select or name one goal.".into(),
                );
            }
            parked.first().copied()
        }
        Some("only_goal") => {
            let goals: Vec<_> = storage
                .core_snapshot(None)?
                .goals
                .into_iter()
                .filter(|g| g.status != "deleted")
                .collect();
            if goals.len() != 1 {
                return Err("Which goal do you mean? Please select or name one goal.".into());
            }
            Some(goals[0].id)
        }
        _ => target.goal_id,
    }
    .ok_or("Which goal do you mean? There is no unambiguous goal in this chat.")?;
    let core = storage.core_snapshot(None)?;
    let goal = core
        .goals
        .iter()
        .find(|g| g.id == id && g.status != "deleted")
        .ok_or("That goal no longer exists. Which goal do you mean?")?;
    let identity = match target.reference.as_deref() {
        Some("last_referenced") => refs.last_referenced_goal_created_at.as_deref(),
        Some("last_created") => refs.last_created_goal_created_at.as_deref(),
        Some("previous_action") => refs
            .last_action
            .as_ref()
            .and_then(|a| a["data"]["created_at"].as_str()),
        _ => None,
    };
    if matches!(
        target.reference.as_deref(),
        Some("last_referenced" | "last_created" | "previous_action")
    ) && identity != Some(goal.created_at.as_str())
    {
        return Err("The referenced goal changed or was deleted. Which goal do you mean?".into());
    }
    Ok(id)
}
fn resolve_step(
    plan: &crate::goals::GoalPlan,
    target: &StepTarget,
    refs: &References,
) -> Result<String, String> {
    let id = if let Some(n) = target.ordinal {
        plan.steps.get(n - 1).map(|s| s.id.clone())
    } else {
        match target.reference.as_deref() {
            Some("last_referenced") => (refs.last_referenced_goal_id == Some(plan.goal_id))
                .then(|| refs.last_referenced_step_id.clone())
                .flatten(),
            Some("last_created") => (refs.last_created_step_goal_id == Some(plan.goal_id))
                .then(|| refs.last_created_step_id.clone())
                .flatten(),
            Some("previous_action") => refs
                .last_action
                .as_ref()
                .filter(|a| a["goal_id"].as_i64() == Some(plan.goal_id))
                .and_then(|a| a["step_id"].as_str().map(str::to_string)),
            _ => target.step_id.clone(),
        }
    }
    .ok_or("Which step do you mean? Please specify its position or ID.")?;
    if !plan.steps.iter().any(|s| s.id == id) {
        return Err("That step does not belong to the selected goal or no longer exists.".into());
    }
    Ok(id)
}
fn execute_one(
    skill: &Skill,
    storage: &mut Storage,
    input: Input,
    refs: &mut References,
    conversation: i64,
) -> Result<SkillResult, String> {
    if let Some(extension) = input.extension {
        return enhancements::execute(skill, storage, extension, refs, conversation);
    }
    let mut result = SkillResult {
        skill: skill.name.into(),
        goal_id: None,
        step_id: None,
        data: json!({}),
    };
    if skill.name == "inspect_conversation" {
        result.data = storage.inspect_conversation(
            conversation,
            input.query.as_deref().unwrap_or_default(),
            input.before_message_id,
        )?;
        return Ok(result);
    }
    if skill.name == "inspect_continuity" {
        let goal_id = input
            .goal
            .as_ref()
            .map(|g| resolve_goal(storage, g, refs))
            .transpose()?;
        let report = storage.continuity_report(
            input.view.ok_or("Choose a continuity view")?,
            goal_id,
            chrono::Local::now().date_naive(),
        )?;
        result.goal_id = goal_id;
        result.data =
            json!({"day":report.day,"report":report.render(),"omitted_lanes":report.omitted_lanes});
        return Ok(result);
    }
    if skill.name == "inspect_today" {
        let core = storage.core_snapshot(None)?;
        let controls = storage.memory_controls()?;
        let remaining:Vec<_>=core.goals.iter().filter(|g|g.status!="completed" && core.today.contains(&g.id)).map(|g|json!({"title":g.title,"steps":g.plan.steps.iter().filter(|s|!s.done).map(|s|&s.text).collect::<Vec<_>>()})).collect();
        let waiting: Vec<_> = if controls.enabled && controls.remember_work_context {
            core.goals
                .iter()
                .filter(|g| g.status != "completed")
                .filter_map(|g| {
                    g.work_context
                        .as_ref()
                        .and_then(|c| c.context.waiting_for.as_ref())
                        .map(|w| json!({"title":g.title,"waiting_for":w}))
                })
                .collect()
        } else {
            vec![]
        };
        let omitted = remaining.len().saturating_sub(40) + waiting.len().saturating_sub(40);
        let summary = core.summary;
        result.data = json!({"day":summary.day,"confirmed_results":summary.completed,"completed_goals":summary.completed_goals,
            "timer_seconds":summary.goals.iter().map(|g|g.seconds).sum::<u64>(),
            "observed_seconds":summary.goals.iter().map(|g|g.tracked_seconds).sum::<u64>(),
            "relevant_seconds":summary.goals.iter().map(|g|g.relevant_seconds).sum::<u64>(),
            "remaining_goals":remaining.into_iter().take(40).collect::<Vec<_>>(),"waiting_goals":waiting.into_iter().take(40).collect::<Vec<_>>(),"omitted_goals":omitted});
        return Ok(result);
    }
    if skill.name == "inspect_work_queue" {
        result.data = work_queue_from_core(&storage.core_snapshot(None)?);
        return Ok(result);
    }
    if skill.name == "list_goals" {
        result.data = json!({"goals":storage.core_snapshot(None)?.goals.into_iter().filter(|g|g.status!="deleted").map(|g|json!({"goal_id":g.id,"title":g.title,"status":g.status})).collect::<Vec<_>>()});
        return Ok(result);
    }
    if skill.name == "create_goal" {
        let title = input.title.ok_or("Goal title is required")?;
        let batch = format!(
            "skill-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        storage.core_add_goals(
            vec![GoalDraft {
                title,
                area: "General".into(),
                steps: input.steps.unwrap_or_default(),
            }],
            &batch,
        )?;
        let id: i64 = storage
            .connection
            .query_row("SELECT MAX(id) FROM goals", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        storage.core_today(id, true, &chrono::Local::now().date_naive().to_string())?;
        result.goal_id = Some(id);
        refs.last_created_goal_id = Some(id);
        refs.last_created_step_id = None;
        refs.last_created_step_goal_id = None;
    } else if skill.name == "restore_goal" {
        let id = restore(
            storage,
            input.backup_id.ok_or("Choose a deleted goal backup")?,
        )?;
        result.goal_id = Some(id);
        refs.last_created_goal_id = Some(id);
        refs.last_created_step_id = None;
        refs.last_created_step_goal_id = None;
    } else {
        let id = resolve_goal(storage, input.goal.as_ref().ok_or("Choose a goal")?, refs)?;
        let snapshot = storage.core_snapshot(None)?;
        let goal = snapshot
            .goals
            .iter()
            .find(|g| g.id == id)
            .ok_or("Goal no longer exists")?;
        result.goal_id = Some(id);
        match skill.name {
            "search_web" | "attach_research_to_context" | "record_research_feedback" => {
                let step = input
                    .step
                    .as_ref()
                    .map(|s| resolve_step(&storage.goal_plan(id)?, s, refs))
                    .transpose()?;
                let receipt = match skill.name {
                    "search_web" => storage.queue_research(
                        id,
                        step,
                        input.text.as_deref().unwrap_or_default(),
                    )?,
                    "attach_research_to_context" => {
                        let receipt = input.research_id.ok_or("Choose research")?;
                        storage.attach_research(id, receipt)?;
                        receipt
                    }
                    _ => {
                        let receipt = input.research_id.ok_or("Choose research")?;
                        storage.research_feedback(
                            id,
                            receipt,
                            input.helpful.ok_or("Choose helpful or not helpful")?,
                        )?;
                        receipt
                    }
                };
                result.step_id = storage.research_record(id, receipt)?.step_id;
                let step_text = goal
                    .plan
                    .steps
                    .iter()
                    .find(|s| Some(&s.id) == result.step_id.as_ref())
                    .map(|s| s.text.clone());
                result.data = json!({"research_id":receipt,"status":if skill.name=="search_web" {"authorized"} else {"saved"},"title":goal.title,"created_at":goal.created_at,"step_text":step_text});
                refs.last_action =
                    Some(serde_json::to_value(&result).map_err(|_| "Invalid research receipt")?);
                refs.last_referenced_goal_id = Some(id);
                refs.last_referenced_goal_created_at = Some(goal.created_at.clone());
                refs.last_referenced_step_id = result.step_id.clone();
                return Ok(result);
            }
            "inspect_goal" => {}
            "park_context" => {
                let context = storage.park_work_context(
                    id,
                    input
                        .checkpoint
                        .ok_or("Generate a checkpoint before parking work")?,
                    input.current_app,
                )?;
                result.step_id = context.context.step_id.clone();
            }
            "resume_context" => {
                let context = storage.resume_work_context(
                    id,
                    input
                        .set_focus
                        .ok_or("Choose whether to focus the resumed goal")?,
                )?;
                result.step_id = context.context.step_id.clone();
            }
            "inspect_work_context" => {
                storage.inspect_work_context(id)?;
            }
            "delete_goal" => {
                let backup = delete(storage, id)?;
                result.data = json!({"title":goal.title,"backup_id":backup,"undo":"restore_goal","undo_days":30});
                if refs.last_created_goal_id == Some(id) {
                    refs.last_created_goal_id = None;
                    refs.last_created_goal_created_at = None;
                }
                if refs.last_created_step_goal_id == Some(id) {
                    refs.last_created_step_id = None;
                    refs.last_created_step_goal_id = None;
                }
                refs.last_referenced_goal_id = None;
                refs.last_referenced_goal_created_at = None;
                refs.last_referenced_step_id = None;
                refs.last_action = Some(serde_json::to_value(&result).map_err(|e| e.to_string())?);
                return Ok(result);
            }
            "add_goal_to_today" | "remove_goal_from_today" => {
                storage.core_today(id, skill.name == "add_goal_to_today", &snapshot.date)?
            }
            "complete_goal" => storage.core_transition(id, "complete")?,
            "reopen_goal" => storage.core_reopen_preserving_steps(id)?,
            "set_focus_goal" => {
                if goal.status != "open" || !snapshot.today.contains(&id) {
                    return Err("Choose an open goal from Today".into());
                }
                if snapshot.focused_goal_id != Some(id) {
                    storage.transition_goal(id, "resume")?;
                }
                storage.mark_explicit_focus(id)?;
            }
            kind => {
                let mut plan = goal.plan.clone();
                let mut title = goal.title.clone();
                if kind == "rename_goal" {
                    title = input.title.ok_or("Supply a new title")?;
                } else if kind == "create_step" {
                    let step_id = format!(
                        "skill-{}",
                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
                    );
                    plan.steps.push(Step {
                        id: step_id.clone(),
                        text: input.text.ok_or("Supply step text")?,
                        done: false,
                    });
                    result.step_id = Some(step_id.clone());
                    refs.last_created_step_id = Some(step_id);
                    refs.last_created_step_goal_id = Some(id);
                } else {
                    let step_id =
                        resolve_step(&plan, input.step.as_ref().ok_or("Choose a step")?, refs)?;
                    let i = plan.steps.iter().position(|s| s.id == step_id).unwrap();
                    match kind {
                        "rename_step" => {
                            plan.steps[i].text = input.text.ok_or("Supply step text")?
                        }
                        "complete_step" => plan.steps[i].done = true,
                        "reopen_step" => plan.steps[i].done = false,
                        "delete_step" => {
                            result.data["removed_step"] = json!(plan.steps[i]);
                            plan.steps.remove(i);
                            if refs.last_created_step_goal_id == Some(id)
                                && refs.last_created_step_id.as_ref() == Some(&step_id)
                            {
                                refs.last_created_step_id = None;
                                refs.last_created_step_goal_id = None;
                            }
                        }
                        _ => return Err("Unknown skill executor".into()),
                    }
                    if plan.current_step.as_ref() == Some(&step_id)
                        && (kind == "delete_step" || kind == "complete_step")
                    {
                        plan.current_step = None;
                    }
                    result.step_id = Some(step_id);
                }
                let area = snapshot
                    .areas
                    .iter()
                    .find(|a| a.id == goal.area_id)
                    .ok_or("Goal area unavailable")?;
                storage.core_save_goal_details(&title, &area.title, plan, None)?;
            }
        }
    }
    let id = result.goal_id.unwrap();
    let core = storage.core_snapshot(None)?;
    let goal = core
        .goals
        .iter()
        .find(|g| g.id == id)
        .ok_or("Goal no longer exists")?;
    let removed = result.data["removed_step"].clone();
    result.data = enhancements::goal_data(storage, id)?;
    result.data["removed_step"] = removed;
    if ["park_context", "resume_context", "inspect_work_context"].contains(&skill.name) {
        result.data = json!({"title":goal.title,"status":goal.status,"created_at":goal.created_at,"checkpoint":storage.inspect_work_context(id)?.map(|c|crate::work_context::public_checkpoint(&c))});
    }
    if refs.last_referenced_goal_id != Some(id) {
        refs.last_referenced_step_id = None;
    }
    refs.last_referenced_goal_id = Some(id);
    refs.last_referenced_goal_created_at = Some(goal.created_at.clone());
    if matches!(skill.name, "create_goal" | "restore_goal") {
        refs.last_created_goal_created_at = Some(goal.created_at.clone());
    }
    if result.step_id.is_some() {
        refs.last_referenced_step_id = if skill.name == "delete_step" {
            None
        } else {
            result.step_id.clone()
        };
    }
    refs.last_action = Some(serde_json::to_value(&result).map_err(|e| e.to_string())?);
    Ok(result)
}

pub(crate) fn delete(storage: &mut Storage, id: i64) -> Result<i64, String> {
    let snapshot = storage.core_snapshot(None)?;
    let goal = snapshot
        .goals
        .iter()
        .find(|g| g.id == id)
        .ok_or("Goal no longer exists")?;
    let mut rows = std::collections::BTreeMap::new();
    for table in ai_chat::BACKUP_TABLES.into_iter().chain([
        "core_goal_details",
        "app_rules",
        "core_goal_reopen_overrides",
    ]) {
        rows.insert(table.into(), ai_chat::dump_rows(storage, table, id)?);
    }
    let backup = ai_chat::GoalBackup {
        title: goal.title.clone(),
        area: snapshot
            .areas
            .iter()
            .find(|a| a.id == goal.area_id)
            .ok_or("Goal area unavailable")?
            .title
            .clone(),
        status: goal.status.clone(),
        on_today: snapshot.today.contains(&id),
        plan: goal.plan.clone(),
        rows,
    };
    storage
        .connection
        .execute(
            "INSERT INTO deleted_goal_backups(title,deleted_at,payload) VALUES(?1,?2,?3)",
            params![
                backup.title,
                chrono::Utc::now().timestamp(),
                serde_json::to_string(&backup).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
    let backup_id = storage.connection.last_insert_rowid();
    storage.core_transition(id, "delete")?;
    storage.ensure_today_focus()?;
    Ok(backup_id)
}
pub(crate) fn restore(storage: &mut Storage, backup_id: i64) -> Result<i64, String> {
    // Restoring historical completed steps is not a newly achieved result.
    let revision: u64 = storage
        .read_setting("mascot_completion_revision")?
        .unwrap_or(0);
    storage
        .connection
        .execute_batch("SAVEPOINT mascot_restore")
        .map_err(|e| e.to_string())?;
    let result = restore_inner(storage, backup_id).and_then(|id| {
        storage.write_setting("mascot_completion_revision", &revision)?;
        Ok(id)
    });
    match result {
        Ok(id) => {
            storage
                .connection
                .execute_batch("RELEASE mascot_restore")
                .map_err(|e| e.to_string())?;
            Ok(id)
        }
        Err(error) => {
            storage
                .connection
                .execute_batch("ROLLBACK TO mascot_restore; RELEASE mascot_restore")
                .map_err(|e| e.to_string())?;
            Err(error)
        }
    }
}
fn restore_inner(storage: &mut Storage, backup_id: i64) -> Result<i64, String> {
    let payload: String = storage
        .connection
        .query_row(
            "SELECT payload FROM deleted_goal_backups WHERE id=?1 AND deleted_at>=?2",
            params![
                backup_id,
                chrono::Utc::now().timestamp() - ai_chat::KEEP_DELETED_SECONDS
            ],
            |r| r.get(0),
        )
        .map_err(|_| "That undo backup is unavailable or expired")?;
    let backup: ai_chat::GoalBackup =
        serde_json::from_str(&payload).map_err(|_| "Invalid undo backup")?;
    storage.core_add_goals(
        vec![GoalDraft {
            title: backup.title.clone(),
            area: backup.area.clone(),
            steps: vec![],
        }],
        &format!("skill-restore-{backup_id}"),
    )?;
    let id: i64 = storage
        .connection
        .query_row("SELECT MAX(id) FROM goals", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let mut plan = backup.plan;
    plan.goal_id = id;
    plan.revision = storage.goal_plan(id)?.revision;
    if backup
        .rows
        .get("core_goal_reopen_overrides")
        .is_some_and(|rows| !rows.is_empty())
    {
        storage
            .connection
            .execute(
                "INSERT OR IGNORE INTO core_goal_reopen_overrides(goal_id) VALUES(?1)",
                [id],
            )
            .map_err(|e| e.to_string())?;
    }
    storage.core_restore_goal_details(&backup.title, &backup.area, plan)?;
    if backup.status == "completed" {
        storage.core_transition(id, "complete")?;
    } else if backup.status == "deferred" {
        storage.core_transition(id, "defer")?;
    }
    storage
        .connection
        .execute("DELETE FROM core_events WHERE goal_id=?1", [id])
        .map_err(|e| e.to_string())?;
    for table in ai_chat::BACKUP_TABLES.into_iter().chain([
        "core_goal_details",
        "app_rules",
        "core_goal_reopen_overrides",
    ]) {
        if let Some(rows) = backup.rows.get(table) {
            ai_chat::load_rows(storage, table, id, rows)?;
        }
    }
    if backup.on_today && backup.status == "open" {
        storage.core_today(id, true, &chrono::Local::now().date_naive().to_string())?;
    }
    storage
        .connection
        .execute("DELETE FROM deleted_goal_backups WHERE id=?1", [backup_id])
        .map_err(|e| e.to_string())?;
    Ok(id)
}

/// All actions, references and undo records commit together or roll back together.
// Prevent pre-creation IDs from masquerading as a newly created goal. Existing-goal
// mutations may precede creation; dependent mutations must use its result reference.
fn validate_creation_targets(plan: &Plan) -> Result<(), String> {
    let mut created = false;
    let mut last_reference_is_new = false;
    let mut last_result_is_new = false;
    for action in &plan.actions {
        if matches!(action.name.as_str(), "create_goal" | "create_planned_goal") {
            created = true;
            last_reference_is_new = true;
            last_result_is_new = true;
            continue;
        }
        let Some(target) = action.input.get("goal") else {
            last_result_is_new = false;
            continue;
        };
        let targets_new = match target["reference"].as_str() {
            Some("last_created") => created,
            Some("last_referenced") => last_reference_is_new,
            Some("previous_action") => last_result_is_new,
            _ => false,
        };
        let mutation = registry()
            .iter()
            .any(|skill| skill.name == action.name && skill.safety != Safety::ReadOnly);
        if created && mutation && !targets_new {
            return Err("A mutation after goal creation targets an earlier goal instead of the creation result. For the new goal use goal_id=null, reference=last_created. Put intentionally requested existing-goal mutations before creation, or use a separate plan. No changes were made.".into());
        }
        last_reference_is_new = targets_new;
        last_result_is_new = targets_new;
    }
    Ok(())
}

fn batch(
    storage: &mut Storage,
    snap: &Snapshot,
    plan: &Plan,
    commit: bool,
) -> Result<Vec<SkillResult>, String> {
    validate_creation_targets(plan)?;
    storage
        .connection
        .execute_batch("SAVEPOINT buddy_skill_batch")
        .map_err(|e| e.to_string())?;
    let outcome = (|| {
        // Re-check inside the SQLite snapshot as well as under the application mutex.
        if snapshot(storage, snap.conversation)?.data != snap.data {
            return Err("The goals changed. Please retry with the current state.".into());
        }
        let mut refs = snap.references.clone();
        let undo = enhancements::capture_undo(storage, plan, &refs)?;
        let mut results = Vec::new();
        for action in &plan.actions {
            let skill = registry()
                .into_iter()
                .find(|s| s.name == action.name)
                .ok_or("Unknown skill")?;
            let input = (skill.validator)(&skill, &action.input)?;
            let result = (skill.executor)(&skill, storage, input, &mut refs, snap.conversation)?;
            if !valid_result(
                &skill.result_schema,
                &serde_json::to_value(&result).map_err(|e| e.to_string())?,
            ) {
                return Err("The skill result failed validation; nothing was changed".into());
            }
            if skill.safety == Safety::ReadOnly {
                if let Some(id) = result.goal_id {
                    refs.last_referenced_goal_id = Some(id);
                    refs.last_referenced_goal_created_at = Some(
                        storage
                            .connection
                            .query_row("SELECT created_at FROM goals WHERE id=?1", [id], |r| {
                                r.get(0)
                            })
                            .map_err(|e| e.to_string())?,
                    );
                    refs.last_referenced_step_id = result.step_id.clone();
                }
                refs.last_action = Some(serde_json::to_value(&result).map_err(|e| e.to_string())?);
            }
            results.push(result);
        }
        storage.save_references(snap.conversation, &refs)?;
        if commit {
            enhancements::save_undo(storage, snap.conversation, plan, undo)?;
        }
        Ok(results)
    })();
    if outcome.is_err() || !commit {
        storage
            .connection
            .execute_batch("ROLLBACK TO buddy_skill_batch; RELEASE buddy_skill_batch")
            .map_err(|e| e.to_string())?;
    } else {
        storage
            .connection
            .execute_batch("RELEASE buddy_skill_batch")
            .map_err(|e| e.to_string())?;
    }
    outcome
}
pub fn execute(
    storage: &mut Storage,
    snap: &Snapshot,
    plan: &Plan,
    latest: &str,
    share: bool,
    confirmed: bool,
) -> Result<Execution, String> {
    execute_for_event(
        storage,
        snap,
        plan,
        latest,
        share,
        confirmed,
        crate::agent_runtime::EventKind::UserMessage,
    )
}
/// Text and voice share all validation/execution; speech additionally confirms deletions.
pub fn execute_for_event(
    storage: &mut Storage,
    snap: &Snapshot,
    plan: &Plan,
    latest: &str,
    share: bool,
    confirmed: bool,
    event: crate::agent_runtime::EventKind,
) -> Result<Execution, String> {
    if !event.explicit() {
        return Err("An explicit user action is required".into());
    }
    if plan.state_version != snap.version || snapshot(storage, snap.conversation)?.data != snap.data
    {
        return Err(
            "The goals or conversation changed. Please retry with the current state.".into(),
        );
    }
    if plan.kind != "actions"
        || plan.actions.is_empty()
        || plan.actions.len() > 20
        || plan.clarification.is_some()
    {
        return Err("No skill actions to execute".into());
    }
    for action in &plan.actions {
        let skill = registry()
            .into_iter()
            .find(|s| s.name == action.name)
            .ok_or("Unknown skill")?;
        (skill.validator)(&skill, &action.input)?;
        if !share
            && !["create_goal", "create_planned_goal", "inspect_conversation"].contains(&skill.name)
        {
            return Err("Enable goal context sharing before acting on existing goals.".into());
        }
        if skill.safety != Safety::ReadOnly && !confirmed {
            let answer = latest.trim().trim_end_matches(['.', '!']).to_lowercase();
            if ["confirm", "yes", "подтверждаю", "да"].contains(&answer.as_str()) {
                return Err(
                    "There is no pending action to confirm. Please give an explicit instruction."
                        .into(),
                );
            }

            let quote = action
                .instruction
                .as_deref()
                .filter(|q| q.trim().chars().count() >= 3 && latest.contains(*q));
            if quote.is_none() {
                return Err(
                    "Please give an explicit instruction before changing goals or steps.".into(),
                );
            }
        }
    }
    if plan.actions.iter().any(|a| a.name == "undo_last_goal_edit") && plan.actions.len() != 1 {
        return Err("Undo the last goal edit separately from other changes".into());
    }
    if plan.actions.iter().any(|a| a.name == "search_web") && plan.actions.len() != 1 {
        return Err("Confirm web research separately from other actions".into());
    }
    let mut parked = std::collections::HashSet::new();
    for action in plan.actions.iter().filter(|a| a.name == "park_context") {
        let id = resolve_workflow_goal(storage, &action.input, snap)?;
        if !parked.insert(id) {
            return Err("Park each goal once per request".into());
        }
        for other in plan.actions.iter().filter(|a| a.name != "park_context") {
            if registry()
                .iter()
                .any(|s| s.name == other.name && s.safety != Safety::ReadOnly)
            {
                if let Some(target) = other.input.get("goal") {
                    let target: GoalTarget = serde_json::from_value(target.clone())
                        .map_err(|_| "Invalid goal reference")?;
                    if resolve_goal(storage, &target, &snap.references)
                        .is_ok_and(|other_id| other_id == id)
                    {
                        return Err("Park separately from other changes to the same goal so the checkpoint uses current work".into());
                    }
                }
            }
        }
    }
    if plan.actions.iter().any(|a| {
        registry().iter().any(|s| {
            s.name == a.name
                && (s.safety == Safety::Confirmation
                    || (event == crate::agent_runtime::EventKind::VoiceMessage
                        && a.name == "delete_goal"))
        })
    }) && !confirmed
    {
        let preview = batch(storage, snap, plan, false)?;
        let pending = Pending {
            plan: plan.clone(),
            snapshot: snap.clone(),
            expires_at: chrono::Utc::now().timestamp() + 300,
        };
        storage.save_references(snap.conversation, &snap.references)?;
        storage
            .connection
            .execute(
                "UPDATE buddy_action_state SET pending_json=?1 WHERE conversation_id=?2",
                params![
                    serde_json::to_string(&pending).map_err(|e| e.to_string())?,
                    snap.conversation
                ],
            )
            .map_err(|e| e.to_string())?;
        return Ok(Execution{status:"confirmation_required",results:preview,message:Some(if plan.actions.iter().any(|a|a.name=="search_web") {"Allow web research for this goal and step? Only a short search query goes to Claude web search. Reply Confirm or Cancel. No search has run yet."} else {"Confirm the proposed changes above? Reply Confirm or Cancel. Nothing has changed yet."}.into())});
    }
    Ok(Execution {
        status: "executed",
        results: batch(storage, snap, plan, true)?,
        message: None,
    })
}
pub fn pending_reply(
    storage: &mut Storage,
    conversation: i64,
    text: &str,
    share: bool,
    privacy_revision: u64,
) -> Result<Option<Execution>, String> {
    let pending: Option<String> = storage
        .connection
        .query_row(
            "SELECT pending_json FROM buddy_action_state WHERE conversation_id=?1",
            [conversation],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let Some(pending) = pending else {
        return Ok(None);
    };
    storage
        .connection
        .execute(
            "UPDATE buddy_action_state SET pending_json=NULL WHERE conversation_id=?1",
            [conversation],
        )
        .map_err(|e| e.to_string())?;
    let answer = text.trim().trim_end_matches(['.', '!']).to_lowercase();
    if ["cancel", "no", "отмена", "нет"].contains(&answer.as_str()) {
        return Ok(Some(Execution {
            status: "cancelled",
            results: vec![],
            message: Some("Cancelled. Nothing was changed.".into()),
        }));
    }
    if !["confirm", "yes", "подтверждаю", "да"].contains(&answer.as_str()) {
        return Ok(None);
    }
    let pending: Pending =
        serde_json::from_str(&pending).map_err(|_| "Invalid pending confirmation")?;
    if pending
        .snapshot
        .privacy_revision
        .is_some_and(|r| r != privacy_revision)
    {
        return Err("Sharing settings changed. Please repeat your request.".into());
    }
    if pending.expires_at < chrono::Utc::now().timestamp() {
        return Err("That confirmation expired. Please repeat your request.".into());
    }
    execute(storage, &pending.snapshot, &pending.plan, text, share, true).map(Some)
}

/// The opaque token is compared again under the mutation lock after inference.
pub fn pending_confirmation(
    storage: &Storage,
    conversation: i64,
) -> Result<Option<(String, Value)>, String> {
    let raw: Option<String> = storage
        .connection
        .query_row(
            "SELECT pending_json FROM buddy_action_state WHERE conversation_id=?1",
            [conversation],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    raw.map(|raw| {
        let pending: Pending =
            serde_json::from_str(&raw).map_err(|_| "Invalid pending confirmation")?;
        Ok((
            raw,
            json!({"actions":pending.plan.actions,"expires_at":pending.expires_at}),
        ))
    })
    .transpose()
}

#[cfg(test)]
mod tests;

pub fn resolve_workflow_goal(
    storage: &Storage,
    input: &Value,
    snapshot: &Snapshot,
) -> Result<i64, String> {
    let skill = registry()
        .into_iter()
        .find(|s| s.name == "park_context")
        .unwrap();
    let input = (skill.validator)(&skill, input)?;
    resolve_goal(
        storage,
        input.goal.as_ref().ok_or("Choose a goal")?,
        &snapshot.references,
    )
}

/// Validate a background proposal's exact ownership without executing or previewing any mutation.
pub(crate) fn validate_offer(
    storage: &Storage,
    snapshot: &Snapshot,
    action: &Action,
    goal: Option<i64>,
) -> Result<(), String> {
    let skill = registry()
        .into_iter()
        .find(|s| s.name == action.name)
        .ok_or("Unknown agent skill")?;
    let input = (skill.validator)(&skill, &action.input)?;
    let target = input.goal.ok_or("An offer requires a goal")?;
    if target.goal_id != goal || target.reference.is_some() || goal.is_none() {
        return Err("An offer requires the exact event goal".into());
    }
    let id = resolve_goal(storage, &target, &snapshot.references)?;
    if let Some(step) = input.step {
        resolve_step(&storage.goal_plan(id)?, &step, &snapshot.references)?;
    }
    Ok(())
}
