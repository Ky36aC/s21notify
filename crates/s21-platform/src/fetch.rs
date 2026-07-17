//! Типизированные выборки: GraphQL-ответ → типы s21-core.
//! Маппинг повторяет booking_info()/fetch_deadlines() и команды бота v2.1.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use s21_core::{strip_html, Booking, BookingInfo, DeadlineItem, ExamItem, FeedItem};

use crate::error::Result;
use crate::gql::{GqlClient, PlatformSession};
use crate::queries;

/// Статусы дедлайнов: у платформы они обычно SHIFTED (перенесённый), а не OPEN.
pub const DEADLINE_STATUSES: [&str; 3] = ["OPEN", "SHIFTED", "OVERDUE"];

const T30: Duration = Duration::from_secs(30);
/// deadlinesGetDeadlines отвечает очень долго — увеличенный таймаут.
const T90: Duration = Duration::from_secs(90);

fn s(v: &Value, ptr: &str) -> String {
    v.pointer(ptr)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// id может прийти и числом, и строкой.
fn id_str(v: &Value, ptr: &str) -> String {
    match v.pointer(ptr) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Компактная выжимка по брони (порт booking_info из watcher.py).
pub fn booking_info(b: &Value) -> BookingInfo {
    let verifier = {
        let v = s(b, "/verifierUser/login");
        if v.is_empty() {
            "?".into()
        } else {
            v
        }
    };
    let verifiable = {
        let v = s(b, "/verifiableStudent/user/login");
        if !v.is_empty() {
            v
        } else {
            let t = s(b, "/team/teamName");
            if t.is_empty() {
                "?".into()
            } else {
                t
            }
        }
    };
    let task = {
        let g = s(b, "/task/goalName");
        if !g.is_empty() {
            g
        } else {
            let t = s(b, "/task/title");
            if t.is_empty() {
                "?".into()
            } else {
                t
            }
        }
    };
    BookingInfo {
        start: s(b, "/eventSlot/start"),
        task,
        verifier,
        verifiable,
        online: b.get("isOnline").and_then(Value::as_bool).unwrap_or(false),
        status: s(b, "/bookingStatus"),
    }
}

pub async fn fetch_bookings(
    gql: &GqlClient,
    session: &PlatformSession,
    now: DateTime<Utc>,
) -> Result<Vec<Booking>> {
    let to = (now + chrono::Duration::days(14)).to_rfc3339();
    let data = gql
        .gql(
            session,
            queries::BOOKINGS_OP,
            queries::BOOKINGS_QUERY,
            json!({"to": to, "limit": 50}),
            T30,
        )
        .await?;
    let list = data
        .pointer("/student/getMyUpcomingBookings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(list
        .iter()
        .map(|b| Booking {
            id: id_str(b, "/id"),
            info: booking_info(b),
        })
        .collect())
}

pub async fn fetch_feed(gql: &GqlClient, session: &PlatformSession) -> Result<Vec<FeedItem>> {
    let data = gql
        .gql(
            session,
            queries::NOTIFICATIONS_OP,
            queries::NOTIFICATIONS_QUERY,
            json!({"paging": {"offset": 0, "limit": 20}}),
            T30,
        )
        .await?;
    let list = data
        .pointer("/s21Notification/getS21Notifications/notifications")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(list
        .iter()
        .map(|n| FeedItem {
            id: id_str(n, "/id"),
            group_name: s(n, "/groupName"),
            message: s(n, "/message"),
            time: s(n, "/time"),
            related_object_type: s(n, "/relatedObjectType"),
        })
        .collect())
}

/// Дедлайны, отсортированы по близости. Окна дат не передавать (null!) —
/// с ними шлюз отвечает ошибкой.
pub async fn fetch_deadlines(
    gql: &GqlClient,
    session: &PlatformSession,
) -> Result<Vec<DeadlineItem>> {
    let data = gql
        .gql(
            session,
            queries::DEADLINES_OP,
            queries::DEADLINES_QUERY,
            json!({
                "deadlineStatuses": DEADLINE_STATUSES,
                "page": {"offset": 0, "limit": 50},
                "deadlinesFrom": null,
                "deadlinesTo": null,
                "sorting": null,
            }),
            T90,
        )
        .await?;
    let list = data
        .pointer("/student/getDeadlines")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<DeadlineItem> = list
        .iter()
        .map(|it| {
            let names: Vec<String> = it
                .pointer("/deadlineGoal/goalProjects")
                .and_then(Value::as_array)
                .map(|gs| {
                    gs.iter()
                        .map(|g| s(g, "/project/goalName"))
                        .filter(|n| !n.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let title = if !names.is_empty() {
                names.join(" / ")
            } else {
                let d = strip_html(&s(it, "/deadline/description"));
                if d.is_empty() {
                    "дедлайн".into()
                } else {
                    d
                }
            };
            DeadlineItem {
                id: id_str(it, "/deadline/deadlineId"),
                ts: s(it, "/deadline/deadlineTs"),
                title,
            }
        })
        .collect();
    out.sort_by(|a, b| a.ts.cmp(&b.ts));
    Ok(out)
}

pub async fn fetch_exams(
    gql: &GqlClient,
    session: &PlatformSession,
    now: DateTime<Utc>,
) -> Result<Vec<ExamItem>> {
    let data = gql
        .gql(
            session,
            queries::EXAMS_OP,
            queries::EXAMS_QUERY,
            json!({
                "from": now.to_rfc3339(),
                "to": (now + chrono::Duration::days(s21_core::DEADLINE_WINDOW_DAYS)).to_rfc3339(),
            }),
            T30,
        )
        .await?;
    let list = data
        .pointer("/student/getExams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(list
        .iter()
        .map(|e| {
            let name = {
                let n = s(e, "/name");
                if !n.is_empty() {
                    n
                } else {
                    let g = s(e, "/goalName");
                    if g.is_empty() {
                        "экзамен".into()
                    } else {
                        g
                    }
                }
            };
            ExamItem {
                id: id_str(e, "/examId"),
                ts: s(e, "/beginDate"),
                title: name,
            }
        })
        .collect())
}

/// Событие агенды для /agenda.
#[derive(Debug, Clone)]
pub struct AgendaEvent {
    pub start: String,
    pub label: String,
    pub description: String,
    pub event_type: String,
}

pub async fn fetch_agenda(
    gql: &GqlClient,
    session: &PlatformSession,
    now: DateTime<Utc>,
) -> Result<Vec<AgendaEvent>> {
    let data = gql
        .gql(
            session,
            queries::AGENDA_OP,
            queries::AGENDA_QUERY,
            json!({
                "from": now.to_rfc3339(),
                "to": (now + chrono::Duration::days(7)).to_rfc3339(),
                "limit": 30,
            }),
            T30,
        )
        .await?;
    let list = data
        .pointer("/calendarEventS21/getMyAgendaEvents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(list
        .iter()
        .map(|e| AgendaEvent {
            start: s(e, "/start"),
            label: s(e, "/label"),
            description: s(e, "/description"),
            event_type: s(e, "/agendaEventType"),
        })
        .collect())
}

/// Уровень/печеньки/коины для /status.
#[derive(Debug, Clone, Default)]
pub struct Experience {
    pub level_code: String,
    pub cookies: String,
    pub code_review_points: String,
    pub coins: String,
}

pub async fn fetch_experience(gql: &GqlClient, session: &PlatformSession) -> Result<Experience> {
    let data = gql
        .gql(
            session,
            queries::EXPERIENCE_OP,
            queries::EXPERIENCE_QUERY,
            json!({}),
            T30,
        )
        .await?;
    let xp = data
        .pointer("/student/getExperience")
        .cloned()
        .unwrap_or(Value::Null);
    let num = |ptr: &str| match xp.pointer(ptr) {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(x)) => x.clone(),
        _ => "?".into(),
    };
    Ok(Experience {
        level_code: num("/level/range/levelCode"),
        cookies: num("/cookiesCount"),
        code_review_points: num("/codeReviewPoints"),
        coins: num("/coinsCount"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn booking_info_маппинг_и_фолбэки() {
        let b = json!({
            "id": 123,
            "eventSlot": {"start": "2026-07-20T09:30:00Z"},
            "task": {"goalName": null, "title": "P02D01"},
            "verifierUser": null,
            "verifiableStudent": null,
            "team": {"teamName": "team_x"},
            "isOnline": true,
            "bookingStatus": "OPEN"
        });
        let info = booking_info(&b);
        assert_eq!(info.task, "P02D01");
        assert_eq!(info.verifier, "?");
        assert_eq!(info.verifiable, "team_x");
        assert!(info.online);
        assert_eq!(id_str(&b, "/id"), "123");
    }
}
