//! Тексты GraphQL-операций платформы.
//!
//! ВНИМАНИЕ: GraphQL-шлюз platform.21-school.ru принимает только запросы из
//! белого списка — текст должен ДОСЛОВНО (байт в байт) совпадать с тем, что шлёт
//! официальный веб-клиент, иначе HTTP 400 с причиной в заголовке `x-bad-request`
//! (REQUEST_ABSENT_IN_WHITELISTS). Файлы queries/*.graphql выгружены скриптом из
//! s21notify/queries.py (первоисточник — github.com/s21toolkit/s21schema, MIT);
//! .gitattributes фиксирует для них LF. Не менять форматирование и набор полей.

pub const BOOKINGS_OP: &str = "calendarGetMyReviews";
pub const BOOKINGS_QUERY: &str = include_str!("../queries/bookings.graphql");

pub const NOTIFICATIONS_OP: &str = "getUserNotifications";
pub const NOTIFICATIONS_QUERY: &str = include_str!("../queries/notifications.graphql");

pub const DEADLINES_OP: &str = "deadlinesGetDeadlines";
pub const DEADLINES_QUERY: &str = include_str!("../queries/deadlines.graphql");

pub const EXAMS_OP: &str = "calendarGetExams";
pub const EXAMS_QUERY: &str = include_str!("../queries/exams.graphql");

pub const AGENDA_OP: &str = "getAgendaEvents";
pub const AGENDA_QUERY: &str = include_str!("../queries/agenda.graphql");

pub const EXPERIENCE_OP: &str = "getCurrentUserExperience";
pub const EXPERIENCE_QUERY: &str = include_str!("../queries/experience.graphql");

#[cfg(test)]
mod tests {
    use super::*;

    /// CRLF в тексте запроса = сломанный whitelist (грабля Windows-checkout).
    #[test]
    fn тексты_без_crlf_и_без_хвостовой_пустой_строки() {
        for (name, q) in [
            ("bookings", BOOKINGS_QUERY),
            ("notifications", NOTIFICATIONS_QUERY),
            ("deadlines", DEADLINES_QUERY),
            ("exams", EXAMS_QUERY),
            ("agenda", AGENDA_QUERY),
            ("experience", EXPERIENCE_QUERY),
        ] {
            assert!(!q.contains('\r'), "{name}: CRLF в тексте запроса");
            assert!(
                q.ends_with('}'),
                "{name}: лишний хвост после закрывающей скобки"
            );
        }
    }
}
