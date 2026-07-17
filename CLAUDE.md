# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Проект русскоязычный: README, комментарии, тексты уведомлений и коммиты — на русском.

## Что это

Мультипользовательский сервис (Rust): пересылает события platform.21-school.ru
(«Школа 21») в **Telegram и MAX** — записи на проверку, каскад напоминаний с
кнопкой «✅ Я за компом» и спам-будильником, дедлайны/экзамены, лента. Регистрация
и настройки — через **miniapp** (Leptos/WASM) внутри мессенджера. Пароли студентов
**не хранятся**: пароль вводится один раз, платформа выдаёт offline-токен (Keycloak,
`scope=offline_access`), он шифруется AES-256-GCM, пароль отбрасывается.

История: v1/v2 были однопользовательским Python-демоном (`@floriato_bot`); v3 —
переписан на Rust целиком (бэкенд + фронт), мультипользовательский, два мессенджера.

## Команды

```sh
# тесты (нужен линкер — см. «Грабли»)
cargo test --workspace --exclude s21-miniapp

# статический бинарь сервера (в CI; локально на Windows не собрать — см. ниже)
cargo build --release --target x86_64-unknown-linux-musl -p s21-server

# miniapp (WASM)
cd miniapp && trunk build --release          # → miniapp/dist/

# живые проверки на своём аккаунте
cargo run -p s21-platform --example live_check   # логин/refresh/GraphQL (creds из config.json)
cargo run -p s21-adapters --example send_test     # отправка себе (нужны токены в env)
```

Сборку miniapp **проверять локально** (`trunk build --release`) до пуша — иначе
ошибки wasm-opt всплывают только в CI.

## Архитектура (cargo workspace)

- `crates/s21-core` — чистый диффинг и форматирование (порт `watcher.py` v2.1),
  без I/O. `run_cycle(prev, fetched, settings, me, now, first_cycle, acked) → {snapshot, events, active}`.
- `crates/s21-platform` — Keycloak (offline-токены), GraphQL, REST context-info.
  Тексты запросов — `queries/*.graphql` (`include_str!`).
- `crates/s21-adapters` — trait `MessengerAdapter` + `telegram` (teloxide-core) и
  `max` (свой тонкий клиент). Общая проверка initData (WebAppData), троттлинг.
- `crates/s21-server` — бинарь: axum (HTTP API + вебхуки + статика), SQLite (sqlx),
  `watcher` (таска опроса на юзера), `alarm` (тик 5 с), `sender` (доставка во все привязки).
- `miniapp` — Leptos CSR (не в default-members; собирается только под wasm32/trunk).

БД (`migrations/`): `users` (offline_token_enc BLOB, token_status), `messenger_accounts`
(user_id NULL = /start был, регистрации нет), `settings`, `user_state` (JSON-снапшот =
аналог state.json), `active_bookings` (для будильника), `deliveries`.

## Деплой

Прод — LXC `ssh root@s21notify.lan` (Debian, 512 МБ), systemd-сервис `s21notify`,
каталог `/opt/s21notify`. **Компилировать на LXC нельзя** (памяти нет) — собирает
GitHub Actions (`.github/workflows/build.yml`: musl-бинарь + trunk/WASM → артефакт),
`deploy/deploy.sh` качает артефакт (`gh run download`) и раскладывает на LXC.

```sh
./deploy/deploy.sh                 # последний успешный build ветки
ssh root@s21notify.lan "journalctl -u s21notify -n 50 --no-pager"
```

Здоровье живого инстанса — `GET /healthz`. Домен `https://s21notify.tobitrix.ru`
→ `http://10.0.0.128:80` (проброс на роутере). `.env` (токены ботов, ENCRYPTION_KEY,
JWT_SECRET) — только на LXC, chmod 600; шаблон `deploy/env.example`.

## Грабли (проверены на практике)

- **GraphQL whitelist**: шлюз принимает только дословные тексты операций из белого
  списка (иначе HTTP 400, причина в заголовке `x-bad-request`). Тексты — в
  `crates/s21-platform/queries/*.graphql`, `.gitattributes` фиксирует им LF.
  **Не редактировать**; новые операции брать дословно из github.com/s21toolkit/s21schema.
- **MAX**: база `platform-api2.max.ru` (отзыв GlobalSign); TLS-цепочка от Russian
  Trusted Root CA (НУЦ Минцифры) — свой reqwest-Client с вшитым PEM
  (`crates/s21-adapters/certs/`), НЕ подмешивать этот CA клиентам платформы/TG.
  Токен в `Authorization` без `Bearer`; POST /messages — chat_id в query; клавиатура
  top-level `keyboard`; `bot_started` кладёт user_id скаляром и приходит только при
  ПЕРВОМ старте (вернувшиеся шлют `message_created` с текстом `/start`).
- **wasm-opt / bulk-memory**: Rust 1.82+ эмитит `memory.copy`, wasm-opt `-O` валится
  на валидации. Лечится `data-wasm-opt-params="--enable-bulk-memory"` в `miniapp/index.html`.
- **Сборка на Windows-хосте**: нет MSVC-линкера → GNU-тулчейн
  (`rustup override set stable-x86_64-pc-windows-gnu`) + `gcc` из mingw
  (`scoop install mingw`, добавить `~/scoop/apps/mingw/current/bin` в PATH).
  `trunk` — прекомпилированным бинарём (cargo install долгий).
- **Offline-токены Keycloak**: `scope=openid offline_access` → JWT typ=Offline без exp;
  refresh без пароля даёт access на 24 ч. Отзыв = смена пароля → `needs_relogin`.
- **deadlinesGetDeadlines** медленный — таймаут 90 с, статусы `["OPEN","SHIFTED","OVERDUE"]`,
  окна дат null; опрашивать раз в ~15 мин (каждый 10-й тик).
- **Лента**: типы `CALENDAR` и `DASHBOARD` дублируют сообщения watcher'а и фильтруются;
  `PROJECT` («выставлена оценка») — уникален, проходит.

## Секреты

Репозиторий публичный. `.env`, `*.db`, PEM с приватными данными и токены — вне git
(в .gitignore). Пароли студентов не хранятся и не логируются. Перед push проверять
diff на секреты; токены ботов в чат/логи не выводить.
