use super::*;
use crate::{history::UsageInterval, relevance::ActivityMatch, tracking::ActivityState};
use rusqlite::OptionalExtension;

pub(super) const GROUP_KEY: &str = "CASE WHEN i.status='assigned' AND i.goal_id IS NOT NULL THEN 'goal:'||i.goal_id||':'||i.context_key WHEN i.status='pending' AND c.proposed_title!='' THEN 'proposal:'||lower(trim(c.proposed_title)) ELSE i.status||':'||i.context_key END";

impl Storage {
    pub(crate) fn review_interval_available(
        &self,
        interval: &UsageInterval,
    ) -> Result<bool, String> {
        let overlaps:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM review_intervals WHERE julianday(started_at)<julianday(?2) AND julianday(ended_at)>julianday(?1))",params![interval.start.to_rfc3339(),interval.end.to_rfc3339()],|r|r.get(0)).map_err(db)?;
        Ok(!overlaps)
    }
    /// Reuse the collector's already measured interval, including when no goal is
    /// selected. Retries and overlapping observations cannot inflate totals.
    pub(crate) fn record_review_interval(
        &self,
        interval: &UsageInterval,
        attribution: &ActivityMatch,
    ) -> Result<(), String> {
        let ms = (interval.end - interval.start).num_milliseconds();
        if !(1..=15000).contains(&ms) || interval.state == ActivityState::Paused {
            return Ok(());
        }
        let overlap:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM review_intervals WHERE julianday(started_at)<julianday(?2) AND julianday(ended_at)>julianday(?1))",params![interval.start.to_rfc3339(),interval.end.to_rfc3339()],|r|r.get(0)).map_err(db)?;
        if overlap {
            return Ok(());
        }
        let period = self.review_period(interval.start)?;
        let prefix = format!("{}:", context_key(&interval.activity));
        let previous:Option<(String,String)>=self.connection.query_row("SELECT context_key,ended_at FROM review_intervals WHERE period_id=?1 AND substr(context_key,1,length(?2))=?2 ORDER BY id DESC LIMIT 1",params![period,prefix],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db)?;
        // Never let a later screenshot explain an entire hour with a generic
        // title. Retained evidence is scoped to a contiguous, bounded segment.
        let key = previous
            .filter(|(_, end)| {
                DateTime::parse_from_rfc3339(end).is_ok_and(|end| {
                    end.with_timezone(&Utc) == interval.start
                        && end.timestamp().div_euclid(300)
                            == interval.end.timestamp().div_euclid(300)
                })
            })
            .map(|(key, _)| key)
            .unwrap_or_else(|| format!("{prefix}{}", interval.start.timestamp_millis()));
        let goal = attribution
            .goal_id
            .as_ref()
            .and_then(|id| id.parse::<i64>().ok())
            .filter(|_| {
                attribution.confidence >= crate::relevance::THRESHOLD
                    && interval.state == ActivityState::Focused
            });
        let app = interval.activity.process_name.to_lowercase();
        let goal = goal.filter(|id| {
            self.category(*id, &app).ok().flatten() != Some(crate::insights::Category::Distraction)
                && self.validate_review_goal(*id).is_ok()
        });
        let state = if goal.is_some() {
            "assigned"
        } else {
            "pending"
        };
        let source = self
            .connection
            .query_row(
                "SELECT id FROM usage_intervals WHERE started_at=?1 AND ended_at=?2",
                params![interval.start.to_rfc3339(), interval.end.to_rfc3339()],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(db)?;
        let source_goal = source
            .map(|id| {
                self.connection
                    .query_row(
                        "SELECT goal_id FROM usage_intervals WHERE id=?1",
                        [id],
                        |r| r.get::<_, i64>(0),
                    )
                    .map_err(db)
            })
            .transpose()?;
        let step = source_goal
            .map(|id| self.attributed_step(id))
            .transpose()?
            .unwrap_or_default();
        let title = crate::browser::minimize_title(&interval.activity.window_title);
        crate::memory::atomic(&self.connection, || {
            self.connection.execute("INSERT OR IGNORE INTO review_contexts(period_id,context_key,app,title) VALUES(?1,?2,?3,?4)",params![period,key,app,title]).map_err(db)?;
            self.connection.execute("INSERT INTO review_intervals(period_id,context_key,started_at,ended_at,milliseconds,app,title,domain,source_id,source_step,goal_id,status,reason) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![period,key,interval.start.to_rfc3339(),interval.end.to_rfc3339(),ms,app,title,interval.activity.browser.as_ref().and_then(|b|b.domain.as_deref()),source,step,goal,state,attribution.reason]).map_err(db)?;
            if goal.is_some() && source_goal != goal {
                self.assign_review_interval(
                    self.connection.last_insert_rowid(),
                    goal,
                    "assigned",
                    false,
                    &attribution.reason,
                    attribution.confidence,
                )?;
            }
            Ok(())
        })
    }
    pub(super) fn review_blocks(&self, period: Option<i64>) -> Result<Vec<Block>, String> {
        let sql=format!("SELECT {GROUP_KEY},i.period_id,p.day,MIN(i.started_at),MAX(i.ended_at),SUM(i.milliseconds)/1000,group_concat(DISTINCT i.app),MIN(i.title),MIN(c.description),MIN(c.proposed_title),CASE WHEN i.status='pending' THEN MIN(c.reason) ELSE MIN(i.reason) END,i.status,i.goal_id,g.text,p.summary IS NOT NULL FROM review_intervals i JOIN review_periods p ON p.id=i.period_id JOIN review_contexts c ON c.period_id=i.period_id AND c.context_key=i.context_key LEFT JOIN goals g ON g.id=i.goal_id WHERE (?1 IS NULL OR i.period_id=?1) AND (?1 IS NOT NULL OR p.summary IS NULL) GROUP BY i.period_id,{GROUP_KEY},i.status,i.goal_id ORDER BY i.period_id DESC,MIN(i.started_at)");
        let mut q = self.connection.prepare(&sql).map_err(db)?;
        let mut blocks = q
            .query_map([period], |r| {
                Ok(Block {
                    key: r.get(0)?,
                    period_id: r.get(1)?,
                    day: r.get(2)?,
                    started_at: r.get(3)?,
                    ended_at: r.get(4)?,
                    seconds: r.get(5)?,
                    apps: r.get(6)?,
                    title: r.get(7)?,
                    description: r.get(8)?,
                    proposed_title: r.get(9)?,
                    reason: r.get(10)?,
                    status: r.get(11)?,
                    goal_id: r.get(12)?,
                    goal_title: r.get(13)?,
                    evidence_ids: vec![],
                    finalized: r.get(14)?,
                })
            })
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        for block in &mut blocks {
            if !block.finalized {
                let keys = self.review_keys(block.period_id, &block.key)?;
                for key in keys {
                    let mut q=self.connection.prepare("SELECT id FROM review_evidence WHERE period_id=?1 AND context_key=?2 AND kind='image' ORDER BY id DESC LIMIT 3").map_err(db)?;
                    block.evidence_ids.extend(
                        q.query_map(params![block.period_id, key], |r| r.get::<_, i64>(0))
                            .map_err(db)?
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(db)?,
                    );
                }
                block.evidence_ids.truncate(6);
            }
        }
        Ok(blocks)
    }
    pub(super) fn review_keys(&self, period: i64, key: &str) -> Result<Vec<String>, String> {
        let mut q=self.connection.prepare(&format!("SELECT DISTINCT i.context_key FROM review_intervals i JOIN review_contexts c ON c.period_id=i.period_id AND c.context_key=i.context_key WHERE i.period_id=?1 AND {GROUP_KEY}=?2")).map_err(db)?;
        let keys = q
            .query_map(params![period, key], |r| r.get(0))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        Ok(keys)
    }
    pub(crate) fn activity_review(&self) -> Result<Overview, String> {
        let mut q=self.connection.prepare("SELECT summary FROM review_periods WHERE summary IS NOT NULL ORDER BY ends_at DESC LIMIT 14").map_err(db)?;
        let encoded = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        let summaries = encoded
            .into_iter()
            .map(|s| {
                serde_json::from_str(&s).map_err(|_| "Saved daily summary unavailable".to_string())
            })
            .collect::<Result<Vec<Summary>, _>>()?;
        let mut blocks = self.review_blocks(None)?;
        blocks.retain(|b| !b.finalized);
        let pending_seconds = blocks
            .iter()
            .filter(|b| b.status == "pending")
            .map(|b| b.seconds)
            .sum();
        Ok(Overview {
            pending_seconds,
            pending_summary_count:self.connection.query_row("SELECT COUNT(*) FROM review_periods WHERE summary IS NULL AND julianday(ends_at)<=julianday('now')",[],|r|r.get(0)).map_err(db)?,
            blocks,
            summaries,
        })
    }
    /// A successful classifier can label several app contexts with the same new
    /// work title. This groups only independently supported descriptions, never
    /// every visit to an application. User decisions remain interval-scoped.
    pub(crate) fn classify_review_context(
        &self,
        a: &ActivitySnapshot,
        goal: Option<i64>,
        proposal: Option<&str>,
        description: &str,
    ) -> Result<(), String> {
        let now = DateTime::parse_from_rfc3339(&a.timestamp)
            .map_err(|_| "Invalid observation time")?
            .with_timezone(&Utc);
        let period = self.review_period(now)?;
        let Some(key) = self.review_context_key(period, a)? else {
            return Ok(());
        };
        let description = crate::companion::redact_sample(description)
            .chars()
            .take(1000)
            .collect::<String>();
        self.connection.execute("UPDATE review_contexts SET description=CASE WHEN ?1='Activity could not be identified confidently' AND description!='' THEN description ELSE ?1 END,proposed_title=?2,reason=?3 WHERE period_id=?4 AND context_key=?5 AND EXISTS(SELECT 1 FROM review_periods p WHERE p.id=?4 AND p.summary IS NULL)",params![description,proposal.unwrap_or(""),if goal.is_some(){"AI identified this existing goal"}else if proposal.is_some(){"Work noticed outside the goal list"}else{"Not enough evidence to identify a goal"},period,key]).map_err(db)?;
        if let Some(goal) = goal {
            self.validate_review_goal(goal)?;
            let cutoff = (now - Duration::seconds(90)).to_rfc3339();
            let mut q=self.connection.prepare("SELECT i.id FROM review_intervals i JOIN review_periods p ON p.id=i.period_id WHERE i.period_id=?1 AND context_key=?2 AND i.status='pending' AND i.manual=0 AND julianday(started_at)>=julianday(?3) AND p.summary IS NULL").map_err(db)?;
            let ids = q
                .query_map(params![period, key, cutoff], |r| r.get::<_, i64>(0))
                .map_err(db)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db)?;
            drop(q);
            crate::memory::atomic(&self.connection, || {
                for id in ids {
                    self.assign_review_interval(
                        id,
                        Some(goal),
                        "assigned",
                        false,
                        "AI identified the current work context",
                        1.0,
                    )?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }
    /// Once the goal behind a stable context is known, the whole context counts: every
    /// still-pending interval of the same app, window and page since the context began
    /// moves to that goal with the actual match confidence. Manual review decisions,
    /// saved summaries and other contexts are untouched. Returns the moved interval count.
    /// `until` bounds an earlier stage of the same task; `correct_automatic` also moves
    /// intervals an automatic match had assigned to another goal. Manual decisions never move.
    #[allow(
        clippy::too_many_arguments,
        reason = "One atomic move carries the context, the target, both time bounds and the evidence policy"
    )]
    pub(crate) fn assign_context_between(
        &self,
        a: &ActivitySnapshot,
        goal: i64,
        since: DateTime<Utc>,
        until: Option<DateTime<Utc>>,
        confidence: f64,
        reason: &str,
        correct_automatic: bool,
    ) -> Result<usize, String> {
        self.validate_review_goal(goal)?;
        let prefix = format!("{}:", context_key(a));
        let mut q=self.connection.prepare("SELECT i.id FROM review_intervals i JOIN review_periods p ON p.id=i.period_id WHERE substr(i.context_key,1,length(?1))=?1 AND i.manual=0 AND (i.status='pending' OR (?5 AND i.status='assigned' AND (i.goal_id IS NULL OR i.goal_id!=?4))) AND julianday(i.started_at)>=julianday(?2) AND (?3 IS NULL OR julianday(i.started_at)<=julianday(?3)) AND p.summary IS NULL ORDER BY i.id").map_err(db)?;
        let ids = q
            .query_map(
                params![
                    prefix,
                    since.to_rfc3339(),
                    until.map(|t| t.to_rfc3339()),
                    goal,
                    correct_automatic
                ],
                |r| r.get::<_, i64>(0),
            )
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        drop(q);
        let moved = ids.len();
        crate::memory::atomic(&self.connection, || {
            for id in ids {
                self.assign_review_interval(id, Some(goal), "assigned", false, reason, confidence)?;
            }
            Ok(())
        })?;
        Ok(moved)
    }
    pub(super) fn validate_review_goal(&self, id: i64) -> Result<(), String> {
        let valid:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM goals g JOIN core_goals c ON c.goal_id=g.id WHERE g.id=?1 AND c.state='open' AND g.status!='completed')",[id],|r|r.get(0)).map_err(db)?;
        if valid {
            Ok(())
        } else {
            Err("Choose an existing open goal".into())
        }
    }
    pub(crate) fn validate_planning_goal(&self, id: i64) -> Result<(), String> {
        self.validate_review_goal(id)
    }
    pub(super) fn review_action(
        &mut self,
        period: i64,
        key: &str,
        action: &str,
        goal: Option<i64>,
        title: Option<&str>,
    ) -> Result<(), String> {
        if action == "later" {
            return Ok(());
        }
        if !["assign", "create", "outside", "pending"].contains(&action) {
            return Err("Choose a review action".into());
        }
        let mut q=self.connection.prepare(&format!("SELECT i.id FROM review_intervals i JOIN review_contexts c ON c.period_id=i.period_id AND c.context_key=i.context_key WHERE i.period_id=?1 AND {GROUP_KEY}=?2")).map_err(db)?;
        let ids = q
            .query_map(params![period, key], |r| r.get::<_, i64>(0))
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        drop(q);
        if ids.is_empty() {
            return Err("This activity changed. Refresh the review before continuing.".into());
        }
        crate::memory::atomic(&self.connection, || {
            let target = match action {
                "assign" => {
                    let id = goal.ok_or("Choose a goal")?;
                    self.validate_review_goal(id)?;
                    Some(id)
                }
                "create" => Some(self.create_review_goal(period, title.unwrap_or_default())?),
                _ => None,
            };
            let status = match action {
                "assign" | "create" => "assigned",
                "outside" => "outside",
                _ => "pending",
            };
            for id in ids {
                self.assign_review_interval(
                    id,
                    target,
                    status,
                    true,
                    "User reviewed this activity",
                    1.0,
                )?;
            }
            self.refresh_review_summary(period)?;
            Ok(())
        })
    }
    fn create_review_goal(&self, period: i64, title: &str) -> Result<i64, String> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 120 || title.chars().any(char::is_control) {
            return Err("Enter a goal title of up to 120 characters".into());
        }
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM goals", [], |r| r.get(0))
            .map_err(db)?;
        if count >= 500 {
            return Err("Keep up to 500 goals; remove an older goal before adding more".into());
        }
        let duplicate:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM goals g JOIN core_goals c ON c.goal_id=g.id WHERE lower(trim(g.text))=lower(?1) AND c.state='open')",[title],|r|r.get(0)).map_err(db)?;
        if duplicate {
            return Err("This goal already exists. Choose it from the goal list.".into());
        }
        let day: String = self
            .connection
            .query_row(
                "SELECT day FROM review_periods WHERE id=?1",
                [period],
                |r| r.get(0),
            )
            .map_err(db)?;
        self.connection
            .execute(
                "INSERT INTO goals(text,status,created_at) VALUES(?1,'deferred',?2)",
                params![title, Utc::now().to_rfc3339()],
            )
            .map_err(db)?;
        let id = self.connection.last_insert_rowid();
        self.connection.execute("INSERT INTO core_goals(goal_id,area_id,state) VALUES(?1,(SELECT id FROM core_areas WHERE title='General'),'open')",[id]).map_err(db)?;
        self.connection
            .execute("INSERT INTO core_goal_analyses(goal_id) VALUES(?1)", [id])
            .map_err(db)?;
        self.connection
            .execute(
                "INSERT INTO core_day_items(day,goal_id) VALUES(?1,?2)",
                params![day, id],
            )
            .map_err(db)?;
        self.connection
            .execute(
                "INSERT OR IGNORE INTO core_day_items(day,goal_id) VALUES(?1,?2)",
                params![Local::now().date_naive().to_string(), id],
            )
            .map_err(db)?;
        for date in [&day, &Local::now().date_naive().to_string()] {
            self.connection.execute("INSERT INTO core_days(day,mode) VALUES(?1,'plan') ON CONFLICT(day) DO UPDATE SET mode='plan'",[date]).map_err(db)?;
        }
        let plan = crate::goals::GoalPlan {
            goal_id: id,
            revision: 1,
            done_when: String::new(),
            current_step: None,
            steps: vec![],
        };
        self.connection
            .execute(
                "INSERT INTO goal_plans(goal_id,revision,payload) VALUES(?1,1,?2)",
                params![
                    id,
                    serde_json::to_string(&plan).map_err(|_| "Could not create goal")?
                ],
            )
            .map_err(db)?;
        self.connection.execute("INSERT INTO work_events(evidence_key,goal_id,title,app,occurred_at,source,status) VALUES(?1,?2,?3,'Activity review',?4,'user_planned','planned')",params![format!("review-goal:{period}:{id}"),id,title,Utc::now().to_rfc3339()]).map_err(db)?;
        Ok(id)
    }
    pub(crate) fn refresh_review_summary(&self, period: i64) -> Result<(), String> {
        let saved: Option<String> = self
            .connection
            .query_row(
                "SELECT summary FROM review_periods WHERE id=?1",
                [period],
                |r| r.get(0),
            )
            .map_err(db)?;
        if let Some(saved) = saved {
            let mut summary: Summary =
                serde_json::from_str(&saved).map_err(|_| "Saved daily summary unavailable")?;
            summary.blocks = self.review_blocks(Some(period))?;
            self.connection
                .execute(
                    "UPDATE review_periods SET summary=?1 WHERE id=?2",
                    params![
                        serde_json::to_string(&summary)
                            .map_err(|_| "Could not update daily summary")?,
                        period
                    ],
                )
                .map_err(db)?;
        }
        Ok(())
    }
    /// Moves the still pending, automatically recorded intervals of one episode window to
    /// a goal. Assigned, outside and manually reviewed time never moves, so nothing is
    /// counted twice. Returns the moved seconds.
    pub(crate) fn assign_pending_context(
        &self,
        period: i64,
        context: &str,
        goal: i64,
        reason: &str,
        confidence: f64,
    ) -> Result<i64, String> {
        let mut q = self
            .connection
            .prepare("SELECT id,milliseconds FROM review_intervals WHERE period_id=?1 AND context_key=?2 AND status='pending' AND manual=0")
            .map_err(db)?;
        let rows = q
            .query_map(params![period, context], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(db)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db)?;
        let mut moved = 0;
        for (id, ms) in rows {
            self.assign_review_interval(id, Some(goal), "assigned", false, reason, confidence)?;
            moved += ms;
        }
        Ok(moved / 1000)
    }
    pub(super) fn assign_review_interval(
        &self,
        id: i64,
        target: Option<i64>,
        status: &str,
        manual: bool,
        reason: &str,
        confidence: f64,
    ) -> Result<(), String> {
        let confidence = if confidence.is_finite() {
            confidence.clamp(crate::relevance::THRESHOLD, 1.0)
        } else {
            1.0
        };
        let row=self.connection.query_row("SELECT started_at,ended_at,milliseconds,app,title,domain,source_id,source_step,goal_id,status,manual FROM review_intervals WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<i64>>(6)?,r.get::<_,String>(7)?,r.get::<_,Option<i64>>(8)?,r.get::<_,String>(9)?,r.get::<_,bool>(10)?))).map_err(db)?;
        if !manual && row.10 {
            return Ok(());
        }
        let mut cursor = DateTime::parse_from_rfc3339(&row.0)
            .map_err(|_| "Invalid recorded interval")?
            .timestamp_millis();
        let end_ms = cursor + row.2;
        let mut parts = Vec::new();
        while cursor < end_ms {
            let local = DateTime::from_timestamp_millis(cursor)
                .ok_or("Invalid recorded interval")?
                .with_timezone(&Local);
            let midnight = local_boundary(&Local, local.date_naive() + Duration::days(1), 0)?
                .timestamp_millis();
            let next = midnight.min(end_ms);
            parts.push((local.date_naive().to_string(), next - cursor));
            cursor = next;
        }
        let step = target
            .map(|id| self.attributed_step(id))
            .transpose()?
            .unwrap_or_default();
        // Remove the exact old contribution before transferring it. An interval
        // has one owner; repeat assignments cannot duplicate its duration.
        if let Some(source) = row.6 {
            let old = self
                .connection
                .query_row(
                    "SELECT goal_id,match_confidence,state FROM usage_intervals WHERE id=?1",
                    [source],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, f64>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(db)?;
            if let Some((old, confidence, state)) = old {
                for (date, ms) in &parts {
                    self.connection.execute("UPDATE usage_daily SET milliseconds=MAX(0,milliseconds-?1) WHERE day=?2 AND goal_id=?3 AND process_name=?4",params![ms,date,old,row.3]).map_err(db)?;
                    if confidence >= crate::relevance::THRESHOLD && state == "\"focused\"" {
                        self.connection.execute("UPDATE goal_relevant_daily SET milliseconds=MAX(0,milliseconds-?1) WHERE day=?2 AND goal_id=?3 AND process_name=?4",params![ms,date,old,row.3]).map_err(db)?;
                        self.connection.execute("UPDATE goal_relevant_step_daily SET milliseconds=MAX(0,milliseconds-?1) WHERE day=?2 AND goal_id=?3 AND step_id=?4",params![ms,date,old,row.7]).map_err(db)?;
                    }
                }
            }
        }
        let source = if let Some(goal) = target {
            if let Some(source) = row.6 {
                self.connection.execute("UPDATE usage_intervals SET goal_id=?1,state='\"focused\"',category='\"work\"',match_confidence=?4,match_reason=?2 WHERE id=?3",params![goal,reason,source,confidence]).map_err(db)?;
                source
            } else {
                self.connection.execute("INSERT INTO usage_intervals(goal_id,started_at,ended_at,milliseconds,process_name,window_title,domain,state,category,match_confidence,match_reason) VALUES(?1,?2,?3,?4,?5,?6,?7,'\"focused\"','\"work\"',?9,?8)",params![goal,row.0,row.1,row.2,row.3,row.4,row.5,reason,confidence]).map_err(db)?;
                self.connection.last_insert_rowid()
            }
        } else {
            if let Some(source) = row.6 {
                self.connection
                    .execute("DELETE FROM usage_intervals WHERE id=?1", [source])
                    .map_err(db)?;
            }
            0
        };
        if let Some(goal) = target {
            for (date, ms) in &parts {
                self.connection.execute("INSERT INTO usage_daily(day,goal_id,process_name,milliseconds) VALUES(?1,?2,?3,?4) ON CONFLICT(day,goal_id,process_name) DO UPDATE SET milliseconds=milliseconds+excluded.milliseconds",params![date,goal,row.3,ms]).map_err(db)?;
                self.connection.execute("INSERT INTO goal_relevant_daily(day,goal_id,process_name,milliseconds) VALUES(?1,?2,?3,?4) ON CONFLICT(day,goal_id,process_name) DO UPDATE SET milliseconds=milliseconds+excluded.milliseconds",params![date,goal,row.3,ms]).map_err(db)?;
                self.connection.execute("INSERT INTO goal_relevant_step_daily(day,goal_id,step_id,milliseconds) VALUES(?1,?2,?3,?4) ON CONFLICT(day,goal_id,step_id) DO UPDATE SET milliseconds=milliseconds+excluded.milliseconds",params![date,goal,step,ms]).map_err(db)?;
            }
        }
        self.connection.execute("UPDATE review_intervals SET source_id=?1,source_step=?2,goal_id=?3,status=?4,reason=?5,manual=?6 WHERE id=?7",params![(source!=0).then_some(source),step,target,status,reason,manual,id]).map_err(db)?;
        // Assignments teach the goal's lane profile (a browser's site; the review keeps no
        // document identity), and a manual move away withdraws the old goal's evidence.
        let activity = ActivitySnapshot {
            process_name: row.3.clone(),
            window_title: row.4.clone(),
            browser: crate::browser::context(&row.3, &row.4, row.5.clone()),
            ..Default::default()
        };
        if let Some(goal) = target {
            if status == "assigned" {
                self.learn_lane_profile(
                    goal,
                    &activity,
                    row.2,
                    if manual { 1.0 } else { confidence },
                )?;
            }
        }
        if manual {
            if let Some(old) = row.8.filter(|old| Some(*old) != target) {
                self.unlearn_lane_profile(old, &activity, row.2)?;
            }
        }
        Ok(())
    }
}
