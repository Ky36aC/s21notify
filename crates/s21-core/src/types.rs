//! Типы домена: вход (снятое с платформы), состояние (снапшот) и выход (события).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Компактная выжимка по брони — хранится в снапшоте для отмен/переносов
/// (аналог booking_info() из v2.1, ключи совпадают со state.json).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BookingInfo {
    pub start: String,
    pub task: String,
    pub verifier: String,
    pub verifiable: String,
    pub online: bool,
    pub status: String,
}

/// Бронь с платформы: id + выжимка.
#[derive(Debug, Clone)]
pub struct Booking {
    pub id: String,
    pub info: BookingInfo,
}

/// Запись ленты уведомлений платформы.
#[derive(Debug, Clone)]
pub struct FeedItem {
    pub id: String,
    pub group_name: String,
    /// Сырое сообщение платформы (может содержать HTML — чистится при выводе).
    pub message: String,
    pub time: String,
    pub related_object_type: String,
}

/// Дедлайн: title уже собран из goalName'ов через « / » (это делает s21-platform).
#[derive(Debug, Clone)]
pub struct DeadlineItem {
    pub id: String,
    pub ts: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct ExamItem {
    pub id: String,
    pub ts: String,
    pub title: String,
}

/// Что удалось снять с платформы за один тик.
/// None = секцию в этот тик не опрашивали — соответствующая часть снапшота не трогается
/// (дедлайны/экзамены опрашиваются реже броней).
#[derive(Debug, Clone, Default)]
pub struct Fetched {
    pub bookings: Option<Vec<Booking>>,
    pub feed: Option<Vec<FeedItem>>,
    pub deadlines: Option<Vec<DeadlineItem>>,
    pub exams: Option<Vec<ExamItem>>,
}

/// Настройки пользователя (дефолты = DEFAULTS из config.py v2.1).
#[derive(Debug, Clone)]
pub struct UserSettings {
    pub remind_minutes: String,
    pub notify_bookings: bool,
    pub notify_changes: bool,
    pub notify_reminders: bool,
    pub notify_feed: bool,
    pub notify_deadlines: bool,
    pub notify_alarm: bool,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            remind_minutes: "30, 15, 3".into(),
            notify_bookings: true,
            notify_changes: true,
            notify_reminders: true,
            notify_feed: true,
            notify_deadlines: true,
            notify_alarm: true,
        }
    }
}

/// Запись «ts + заголовок» для дедлайнов и экзаменов в снапшоте.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TsTitle {
    pub ts: String,
    pub title: String,
}

/// Снапшот состояния пользователя — структура повторяет state.json v2.1.
/// Option = «ключа ещё не было»: секция инициализируется молча при первом появлении.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UserSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bookings: Option<IndexMap<String, BookingInfo>>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub reminded_bookings: IndexMap<String, Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seen_notifications: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadlines: Option<IndexMap<String, TsTitle>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reminded_deadlines: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exams: Option<IndexMap<String, TsTitle>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reminded_exams: Vec<String>,
}

/// Вид события — идёт в deliveries.kind и настройки-фильтры уже применены.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    BookingNew,
    BookingMoved,
    BookingCancelled,
    Reminder,
    Feed,
    DeadlineNew,
    DeadlineMoved,
    DeadlineSoon,
    ExamNew,
    ExamSoon,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BookingNew => "booking_new",
            Self::BookingMoved => "booking_moved",
            Self::BookingCancelled => "booking_cancelled",
            Self::Reminder => "reminder",
            Self::Feed => "feed",
            Self::DeadlineNew => "deadline_new",
            Self::DeadlineMoved => "deadline_moved",
            Self::DeadlineSoon => "deadline_soon",
            Self::ExamNew => "exam_new",
            Self::ExamSoon => "exam_soon",
        }
    }
}

/// Готовое к отправке событие. `ack_booking_id` = приложить кнопку
/// «✅ Я за компом» с payload `ack:<bid>`.
#[derive(Debug, Clone)]
pub struct OutEvent {
    pub kind: EventKind,
    pub html: String,
    pub ack_booking_id: Option<String>,
}

/// Предстоящая бронь для таблицы active_bookings (будильник читает её, не API).
#[derive(Debug, Clone)]
pub struct ActiveBooking {
    pub booking_id: String,
    pub start: String,
    pub info: BookingInfo,
}

/// Результат одного цикла диффинга.
#[derive(Debug, Clone)]
pub struct CycleOutput {
    pub snapshot: UserSnapshot,
    pub events: Vec<OutEvent>,
    pub active: Vec<ActiveBooking>,
}
