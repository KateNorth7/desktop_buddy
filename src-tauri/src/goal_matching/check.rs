//! Measures the production cloud matcher, not the local matching cascade.
use super::{parse, request_at};
use crate::{accuracy_check as output, goal_analysis::Service, http, models::ActivitySnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Goal {
    id: i64,
    title: String,
    steps: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Activity {
    app: String,
    window_title: String,
    domain: Option<String>,
    visible_text: Option<String>,
    page_title: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    open_goals: Vec<Goal>,
    activity: Activity,
    #[serde(deserialize_with = "required_label")]
    expected_goal_id: Option<i64>,
}

// Missing annotations are not negative examples: null must be explicit.
fn required_label<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<i64>, D::Error> {
    Option::<i64>::deserialize(deserializer)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    id: String,
    expected_goal_id: Option<i64>,
    goal_id: Option<i64>,
    confidence: Option<f64>,
    reason: String,
    correct: bool,
    error: Option<String>,
}

async fn one(case: &Case, service: Option<&Service<'_>>) -> Row {
    let goals = json!(case
        .open_goals
        .iter()
        .map(|g| json!({"id":g.id,"title":g.title,"steps":g.steps}))
        .collect::<Vec<_>>());
    // Explicit fixture input stands in for an opted-in Accessibility sample.
    // Never read the real desktop or the installed preferences in this CLI.
    let sample = ActivitySnapshot {
        process_name: case.activity.app.clone(),
        window_title: case.activity.window_title.clone(),
        browser: (case.activity.domain.is_some()
            || case.activity.page_title.is_some()
            || crate::browser::browser_name(&case.activity.app).is_some())
        .then(|| crate::browser::BrowserContext {
            browser: crate::browser::browser_name(&case.activity.app)
                .unwrap_or("browser")
                .into(),
            domain: case.activity.domain.clone(),
            page_title: case
                .activity
                .page_title
                .clone()
                .unwrap_or_else(|| case.activity.window_title.clone()),
            source: "fixture".into(),
        }),
        ..Default::default()
    };
    let result = if let Some(service) = service {
        tokio::time::timeout(
            std::time::Duration::from_secs(90),
            request_at(
                service.client,
                &service.endpoint,
                &service.key,
                &service.model,
                &goals,
                &sample,
                true,
                case.activity.visible_text.as_deref(),
                &json!({"trail":[],"last_matched_goal_id":null,"last_matched_minutes_ago":null}),
            ),
        )
        .await
        .unwrap_or_else(|_| Err("Matching check timed out".into()))
    } else {
        // Fixed canned prediction, independent of the expected label.
        parse(
            &output::envelope(
                json!({"goal_id":null,"confidence":0.0,"reason":"AI_MOCK canned unassigned prediction"}),
            ),
            &goals,
            0,
        )
    };
    match result {
        Ok(result) => Row {
            id: case.id.clone(),
            expected_goal_id: case.expected_goal_id,
            goal_id: result.goal_id,
            confidence: Some(result.confidence),
            reason: result.reason,
            correct: result.goal_id == case.expected_goal_id,
            error: None,
        },
        Err(error) => Row {
            id: case.id.clone(),
            expected_goal_id: case.expected_goal_id,
            goal_id: None,
            confidence: None,
            reason: "Provider result unavailable".into(),
            correct: false,
            error: Some(output::error(&error)),
        },
    }
}

fn metrics(rows: &[Row]) -> Value {
    let valid: Vec<_> = rows.iter().filter(|r| r.error.is_none()).collect();
    let negatives = valid
        .iter()
        .filter(|r| r.expected_goal_id.is_none())
        .count();
    let positives = valid.len() - negatives;
    let fp = valid
        .iter()
        .filter(|r| r.expected_goal_id.is_none() && r.goal_id.is_some())
        .count();
    let fn_count = valid
        .iter()
        .filter(|r| r.expected_goal_id.is_some() && r.goal_id.is_none())
        .count();
    let correct = rows
        .iter()
        .filter(|r| r.correct && r.error.is_none())
        .count();
    json!({"total":rows.len(),"correct":correct,"errors":rows.len()-valid.len(),
        "accuracy_percent":output::rate(correct, rows.len()),"false_positives":fp,"false_negatives":fn_count,
        "scored_negative_cases":negatives,"scored_positive_cases":positives,
        "false_positive_rate_percent":output::rate(fp,negatives),"false_negative_rate_percent":output::rate(fn_count,positives)})
}

pub(crate) async fn run() -> Result<bool, String> {
    let cases: Vec<Case> = output::cases("matching-cases.json")?;
    output::unique_ids(cases.iter().map(|c| c.id.as_str()))?;
    for case in &cases {
        let ids: std::collections::HashSet<_> = case.open_goals.iter().map(|g| g.id).collect();
        if ids.len() != case.open_goals.len()
            || case.expected_goal_id.is_some_and(|id| !ids.contains(&id))
        {
            return Err(
                "Goal IDs must be unique and expected_goal_id must be a supplied ID or null".into(),
            );
        }
    }
    let mock = http::enabled("AI_MOCK");
    let client = reqwest::Client::new();
    let service = if mock {
        None
    } else {
        Some(Service::configured(&client)?)
    };
    println!(
        "Goal matching — {}\ncase id | expected | got | confidence | reason",
        if mock {
            "MOCK, not live accuracy"
        } else {
            "real Claude"
        }
    );
    let mut rows = Vec::new();
    for case in &cases {
        let row = one(case, service.as_ref()).await;
        println!(
            "{} | {} | {} | {} | {}",
            output::cell(&row.id),
            row.expected_goal_id
                .map_or("null".into(), |id| id.to_string()),
            if row.error.is_some() {
                "ERROR".into()
            } else {
                row.goal_id.map_or("null".into(), |id| id.to_string())
            },
            row.confidence.map_or("N/A".into(), |n| format!("{n:.3}")),
            output::cell(&row.reason)
        );
        rows.push(row);
    }
    let summary = metrics(&rows);
    println!("Accuracy: {}\nFalse-positive rate: {}\nFalse-negative rate: {}\nProvider errors: {} (excluded from FP/FN rates; included as incorrect in accuracy)",
        output::percentage(summary["accuracy_percent"].as_f64()),output::percentage(summary["false_positive_rate_percent"].as_f64()),
        output::percentage(summary["false_negative_rate_percent"].as_f64()),summary["errors"]);
    let path = output::save(
        "matching",
        &json!({"mock":mock,"model":service.as_ref().map(|s|&s.model),"summary":summary,"cases":rows,
        "scope":"Production request_at cloud matcher only; app, window_title and page_title (160 characters each), domain, and opted-in redacted visible_text (400 characters). Synthetic fixture text stands in for the shared Accessibility sample; no live desktop read. Not the full matching cascade; placeholder/mock scores are not real-world accuracy."}),
    )?;
    println!("Results: {}", path.display());
    Ok(summary["errors"] == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn parses_matching_result_table_and_rates_without_counting_errors_as_null_predictions() {
        assert!(serde_json::from_value::<Case>(json!({"id":"missing-label","open_goals":[],"activity":{"app":"Browser","window_title":"New tab","domain":null,"visible_text":""}})).is_err());
        let cases: Vec<Case> =
            serde_json::from_str(include_str!("../../test-data/matching-cases.json")).unwrap();
        let mut rows = vec![one(&cases[0], None).await];
        rows[0].expected_goal_id = None;
        rows[0].correct = true;
        rows.push(serde_json::from_value(json!({"id":"fp","expected_goal_id":null,"goal_id":1,"confidence":0.9,"reason":"Direct evidence","correct":false,"error":null})).unwrap());
        rows.push(serde_json::from_value(json!({"id":"fn","expected_goal_id":1,"goal_id":null,"confidence":0.1,"reason":"Ambiguous","correct":false,"error":null})).unwrap());
        rows.push(serde_json::from_value(json!({"id":"error","expected_goal_id":null,"goal_id":null,"confidence":null,"reason":"Unavailable","correct":false,"error":"timeout"})).unwrap());
        let parsed: Vec<Row> =
            serde_json::from_str(&serde_json::to_string(&rows).unwrap()).unwrap();
        let summary = metrics(&parsed);
        assert_eq!(summary["accuracy_percent"], 25.0);
        assert_eq!(summary["false_positive_rate_percent"], 50.0);
        assert_eq!(summary["false_negative_rate_percent"], 100.0);
        assert_eq!(summary["errors"], 1);
        assert!(metrics(&[])["accuracy_percent"].is_null());
        assert!(parse(
            &output::envelope(json!({"goal_id":99999,"confidence":0.9,"reason":"Unknown ID"})),
            &json!([]),
            0
        )
        .is_err());
        assert_eq!(output::cell("case\n|id"), "case \\|id");
    }
}
