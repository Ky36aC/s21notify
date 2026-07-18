//! Библиотечная часть сервера — чтобы интеграционные тесты могли собирать
//! приложение по кускам.

pub mod alarm;
pub mod commands;
pub mod config;
pub mod crypto;
pub mod db;
pub mod http;
pub mod poll;
pub mod polling;
pub mod sender;
pub mod sessions;
pub mod state;
pub mod texts;
pub mod watcher;
