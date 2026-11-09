//! Synthetic live acceptance check; never opens the installed database or samples the desktop.
use super::*;
pub(crate) async fn run() -> Result<Value, String> {
    if crate::http::enabled("AI_MOCK") {
        return Err("Live unplanned-work check requires real provider responses".into());
    }
    let client = reqwest::Client::new();
    let service = Service::configured(&client)?;
    let cases=[
        ("unlisted_email", "Google Chrome", "Customer support inbox – Gmail", "mail.google.com", "Customer messages: order received, delivery address, shipping question. Sent: reply to delivery inquiry. Sent: reply to shipping question. Inbox contains more customer messages.", json!([{"id":7,"title":"Prepare the hackathon pitch deck","steps":["Draft the slides"]}]),None,true),
        ("existing_vat_goal", "Google Chrome", "Re: VAT figures – Gmail", "mail.google.com", "Re: Q3 VAT figures. Accountant: please check the attached VAT return figures. Sent: my reply about the Q3 VAT return.", json!([{"id":8,"title":"Reply to the accountant about VAT","steps":[]}]),Some(8),false),
        ("music_stream", "Google Chrome", "lofi hip hop radio – YouTube", "youtube.com", "lofi hip hop radio. Live stream. Music for studying. Playlist and channel suggestions.", json!([{"id":7,"title":"Prepare the hackathon pitch deck","steps":[]}]),None,false),
    ];
    let mut rows = Vec::new();
    for (id, app, title, domain, text, goals, expected_id, expected_new) in cases {
        let a = ActivitySnapshot {
            process_name: app.into(),
            window_title: title.into(),
            window_id: Some(1),
            browser: crate::browser::context(app, title, Some(format!("https://{domain}"))),
            ..Default::default()
        };
        let result = classify(
            &service,
            &a,
            text,
            &goals,
            120,
            &Value::Null,
            &Value::Null,
            &|| Ok(()),
        )
        .await;
        match result {
            Ok(d)=>{
                let offered=d.title.is_some() && d.existing_goal_id.is_none() && d.confidence>=MIN_CONFIDENCE;
                rows.push(json!({"id":id,"passed":offered==expected_new && d.existing_goal_id==expected_id,"new_work_offered":offered,"existing_goal_id":d.existing_goal_id,"confidence":d.confidence,"suggested_title":d.title}));
            }
            Err(e)=>rows.push(json!({"id":id,"passed":false,"error":crate::engine_observability::error_category(&e)})),
        }
    }
    Ok(
        json!({"all_passed":rows.iter().all(|r|r["passed"]==true),"fixture":"synthetic_in_memory","provider":"real_claude","cases":rows,"scope":"Classifier only; interval transfer, consent and completion persistence are covered by Rust regressions. No installed data or actual screen text was used."}),
    )
}
