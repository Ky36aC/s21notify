//! Проверка подписи initData / launch-params по схеме Telegram WebAppData.
//! MAX MWA использует ту же схему (подтверждено боевым NotifyBot,
//! dev.max.ru/docs/webapps/validation) — отличается только окно auth_date.

use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::types::MiniappUser;

type HmacSha256 = Hmac<Sha256>;

/// Разбор querystring в пары. Если `hash` не нашёлся — MAX мог прислать строку,
/// url-закодированную целиком: декодируем и пробуем ещё раз.
fn parse_pairs(init_data: &str) -> Vec<(String, String)> {
    let pairs: Vec<(String, String)> = form_urlencoded::parse(init_data.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if pairs.iter().any(|(k, _)| k == "hash") {
        return pairs;
    }
    let decoded = percent_encoding::percent_decode_str(init_data)
        .decode_utf8_lossy()
        .into_owned();
    form_urlencoded::parse(decoded.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// Подпись data_check_string ключом bot_token (схема WebAppData) — hex.
pub fn sign(data_check_string: &str, bot_token: &str) -> String {
    let mut secret = HmacSha256::new_from_slice(b"WebAppData").unwrap();
    secret.update(bot_token.as_bytes());
    let secret_key = secret.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&secret_key).unwrap();
    mac.update(data_check_string.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Проверяет подпись и окно auth_date; возвращает пользователя из поля `user`.
pub fn verify(
    init_data: &str,
    bot_token: &str,
    max_age_secs: i64,
    now_ts: i64,
) -> Option<MiniappUser> {
    if init_data.is_empty() {
        return None;
    }
    let mut pairs = parse_pairs(init_data);
    let hash_pos = pairs.iter().position(|(k, _)| k == "hash")?;
    let (_, received_hash) = pairs.remove(hash_pos);

    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let data_check_string = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");

    let calc = sign(&data_check_string, bot_token);
    if calc.as_bytes().ct_eq(received_hash.as_bytes()).unwrap_u8() != 1 {
        return None;
    }

    // anti-replay: auth_date в окне (и не из будущего)
    if let Some((_, auth_date)) = pairs.iter().find(|(k, _)| k == "auth_date") {
        let ts: i64 = auth_date.parse().ok()?;
        let age = now_ts - ts;
        if age > max_age_secs || age < -max_age_secs {
            return None;
        }
    }

    let (_, user_raw) = pairs.iter().find(|(k, _)| k == "user")?;
    let user: Value = serde_json::from_str(user_raw).ok()?;
    let id = match user.get("id").or_else(|| user.get("user_id"))? {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    let full_name = {
        let joined = [
            user.get("first_name").and_then(Value::as_str),
            user.get("last_name").and_then(Value::as_str),
        ]
        .iter()
        .flatten()
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
        let joined = joined.trim().to_string();
        if !joined.is_empty() {
            Some(joined)
        } else {
            user.get("name").and_then(Value::as_str).map(String::from)
        }
    };
    Some(MiniappUser {
        ext_user_id: id,
        username: user
            .get("username")
            .and_then(Value::as_str)
            .map(String::from),
        full_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "12345:test-token";

    /// Собирает валидный init_data, подписывая его так же, как это делает мессенджер.
    fn make_init_data(auth_date: i64) -> String {
        let user = r#"{"id":456,"first_name":"Флор","username":"floriato"}"#;
        let user_enc: String = form_urlencoded::byte_serialize(user.as_bytes()).collect::<String>();
        let dcs = format!("auth_date={auth_date}\nuser={user}");
        let hash = sign(&dcs, TOKEN);
        format!("auth_date={auth_date}&user={user_enc}&hash={hash}")
    }

    #[test]
    fn валидная_подпись_проходит() {
        let now = 1_800_000_000;
        let u = verify(&make_init_data(now - 100), TOKEN, 300, now).unwrap();
        assert_eq!(u.ext_user_id, "456");
        assert_eq!(u.username.as_deref(), Some("floriato"));
        assert_eq!(u.full_name.as_deref(), Some("Флор"));
    }

    #[test]
    fn порченая_подпись_и_чужой_токен_отклоняются() {
        let now = 1_800_000_000;
        let good = make_init_data(now);
        let tampered = good.replace("floriato", "hacker");
        assert!(verify(&tampered, TOKEN, 300, now).is_none());
        assert!(verify(&good, "другой:токен", 300, now).is_none());
        assert!(verify("", TOKEN, 300, now).is_none());
        assert!(verify("мусор без хэша", TOKEN, 300, now).is_none());
    }

    #[test]
    fn просроченный_auth_date_отклоняется() {
        let now = 1_800_000_000;
        // старше окна 300 c (Telegram)
        assert!(verify(&make_init_data(now - 301), TOKEN, 300, now).is_none());
        // но проходит в окне суток (MAX)
        assert!(verify(&make_init_data(now - 301), TOKEN, 86400, now).is_some());
        // из будущего — тоже отказ
        assert!(verify(&make_init_data(now + 400), TOKEN, 300, now).is_none());
    }

    #[test]
    fn целиком_закодированная_строка_max() {
        let now = 1_800_000_000;
        let raw = make_init_data(now);
        let whole_enc: String =
            percent_encoding::utf8_percent_encode(&raw, percent_encoding::NON_ALPHANUMERIC)
                .to_string();
        assert!(verify(&whole_enc, TOKEN, 86400, now).is_some());
    }
}
