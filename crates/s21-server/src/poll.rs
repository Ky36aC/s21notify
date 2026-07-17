//! Канал управления опросом: HTTP-слой шлёт команды, watcher (фаза 7) исполняет.

use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollCommand {
    /// Поднять таску опроса (регистрация/перелогин)
    Start(i64),
    /// Остановить и забыть (удаление аккаунта)
    Stop(i64),
    /// Внеочередной цикл (/check)
    CheckNow(i64),
}

pub type PollSender = mpsc::UnboundedSender<PollCommand>;
pub type PollReceiver = mpsc::UnboundedReceiver<PollCommand>;

pub fn channel() -> (PollSender, PollReceiver) {
    mpsc::unbounded_channel()
}
