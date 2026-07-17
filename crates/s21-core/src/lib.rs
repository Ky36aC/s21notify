//! Чистая доменная логика s21notify — порт семантики watcher.py v2.1.
//! Ни I/O, ни async: только данные на входе, события на выходе.

mod cycle;
mod fmt;
mod types;

pub use cycle::{
    parse_remind_minutes, run_cycle, ALARM_BEFORE_SEC, ALARM_REPEAT_SEC, DEADLINE_REMIND_HOURS,
    DEADLINE_WINDOW_DAYS, SKIP_FEED_TYPES,
};
pub use fmt::{
    ack_payload, alarm_message, days_left, esc, fmt_booking_line, fmt_time, parse_ts, strip_html,
    ACK_BUTTON_TEXT,
};
pub use types::*;
