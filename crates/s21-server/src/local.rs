//! Локальный режим (APP_MODE=local): запуск на ПК ученика без домена.
//!
//! Секреты (ENCRYPTION_KEY/JWT_SECRET) генерируются при первом запуске и
//! дописываются в .env рядом с программой; апдейты идут long polling'ом;
//! настройки открываются на http://127.0.0.1:8080 (доверяем localhost).
//! Ученику остаётся вписать только TG_BOT_TOKEN своего бота от @BotFather.

use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use rand::RngCore;

/// Готовит окружение локального режима ДО чтения AppConfig. Возвращает `false`,
/// если не задан ни один токен бота — тогда main завершается с подсказкой.
pub fn prepare() -> anyhow::Result<bool> {
    let env_path = exe_dir().join(".env");
    // .env рядом с бинарём (dotenvy в main грузит только из cwd; при двойном
    // клике cwd обычно и есть каталог программы, но подстрахуемся)
    let _ = dotenvy::from_path(&env_path);

    // секреты: сгенерировать один раз и сохранить в .env
    ensure_secret(&env_path, "ENCRYPTION_KEY")?;
    ensure_secret(&env_path, "JWT_SECRET")?;

    // разумные дефолты локального запуска (не перетирают заданное вручную)
    set_default("BIND_ADDR", "127.0.0.1:8080");
    set_default("PUBLIC_URL", "http://127.0.0.1:8080");
    set_default("ENABLED_MESSENGERS", "telegram");
    set_default("TG_TRANSPORT", "polling");
    set_default("MAX_TRANSPORT", "polling");
    // относительный путь → БД ляжет в каталог запуска (у .exe — рядом с ним)
    set_default("DATABASE_URL", "sqlite:s21notify.db");

    if !present("TG_BOT_TOKEN") && !present("MAX_BOT_TOKEN") {
        ensure_token_placeholder(&env_path)?;
        eprintln!(
            "\n[s21notify] Локальный режим.\n\
             Впиши токен своего бота в файл:\n  {}\n\
             (создай бота у @BotFather в Telegram, скопируй токен в строку TG_BOT_TOKEN)\n\
             Затем запусти программу снова.\n",
            env_path.display()
        );
        return Ok(false);
    }
    Ok(true)
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn present(name: &str) -> bool {
    std::env::var(name)
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

fn set_default(name: &str, val: &str) {
    if !present(name) {
        std::env::set_var(name, val);
    }
}

fn gen_key() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    base64::engine::general_purpose::STANDARD.encode(b)
}

/// Секрет отсутствует → сгенерировать, выставить в env и дописать в .env.
fn ensure_secret(env_path: &Path, name: &str) -> anyhow::Result<()> {
    if present(name) {
        return Ok(());
    }
    let val = gen_key();
    std::env::set_var(name, &val);
    append_line(env_path, &format!("{name}={val}"))?;
    Ok(())
}

/// Добавить пустую строку токена, если её ещё нет — ученику будет куда вписать.
fn ensure_token_placeholder(env_path: &Path) -> anyhow::Result<()> {
    let existing = std::fs::read_to_string(env_path).unwrap_or_default();
    if !existing.contains("TG_BOT_TOKEN") {
        append_line(env_path, "# токен бота от @BotFather:")?;
        append_line(env_path, "TG_BOT_TOKEN=")?;
    }
    Ok(())
}

fn append_line(path: &Path, line: &str) -> anyhow::Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}
