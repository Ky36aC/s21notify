//! Клиент платформы Школы 21: логин через Keycloak (offline-токены),
//! GraphQL-запросы (дословные тексты из белого списка) и REST context-info.

mod auth;
mod error;
mod fetch;
mod gql;
pub mod queries;
mod urls;

pub use auth::{jwt_payload, token_valid, AuthClient, TokenSet};
pub use error::{PlatformError, Result};
pub use fetch::{
    booking_info, fetch_agenda, fetch_bookings, fetch_deadlines, fetch_exams, fetch_experience,
    fetch_feed, AgendaEvent, Experience, DEADLINE_STATUSES,
};
pub use gql::{GqlClient, PlatformSession};
pub use urls::PlatformUrls;
