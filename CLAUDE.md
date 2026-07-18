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

Боты: Telegram `@s21notify_bot`, MAX `@id482618491734_4_bot`.

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

Dev-отладка miniapp: сервер с `DEV_FAKE_AUTH=1`, затем `cd miniapp && trunk serve`
и `http://127.0.0.1:8080/?dev=<id>&m=telegram` — `/api/auth` принимает
`init_data="dev:<id>"` без подписи.

## Архитектура (cargo workspace)

- `crates/s21-core` — чистый диффинг и форматирование, без I/O.
  `run_cycle(prev, fetched, settings, me, now, first_cycle, acked) → {snapshot, events, active}`.
- `crates/s21-platform` — Keycloak (offline-токены), GraphQL, REST context-info.
  Тексты запросов — `queries/*.graphql` (`include_str!`).
- `crates/s21-adapters` — trait `MessengerAdapter` + `telegram` (teloxide-core) и
  `max` (свой тонкий клиент). Общая проверка initData (WebAppData), троттлинг.
- `crates/s21-server` — бинарь: axum (HTTP API + вебхуки + статика), SQLite (sqlx),
  `watcher` (таска опроса платформы на юзера), `alarm` (тик 5 с), `sender` (доставка
  во все привязки), `polling` (приём апдейтов мессенджеров long polling'ом).
  Приём апдейтов: по умолчанию **long polling** (бот сам ходит к API — без домена и
  входящих портов; `TG_TRANSPORT`/`MAX_TRANSPORT=polling`), опционально webhook.
  Режимы (`APP_MODE`): `server` (мультипользовательский) и `local` (модуль `local`:
  single-user на ПК ученика — localhost:8080, доверяем localhost, секреты
  генерируются в `.env` рядом с бинарём; личность = привязка, нажавшая /start).
  CI собирает и самодостаточный Windows `.exe` (артефакт `s21notify-windows`).
- `miniapp` — Leptos CSR (не в default-members; собирается только под wasm32/trunk).

БД (`migrations/`): `users` (offline_token_enc BLOB, token_status), `messenger_accounts`
(user_id NULL = /start был, регистрации нет), `settings`, `user_state` (JSON-снапшот
состояния watcher'а), `active_bookings` (для будильника), `deliveries`.

## Переменные окружения (`config.rs::from_env`)

`APP_MODE` server|local · `BIND_ADDR` (server `0.0.0.0:80`, local `127.0.0.1:8080`) ·
`PUBLIC_URL` (обязателен; local ставит сам) · `STATIC_DIR` (`static`) ·
`DATABASE_URL` (`sqlite://s21notify.db`) · `ENCRYPTION_KEY` (base64, 32 байта) ·
`JWT_SECRET` — в local `ENCRYPTION_KEY`/`JWT_SECRET` генерируются в `.env` сами.
`ENABLED_MESSENGERS` (`telegram,max`) · `TG_BOT_TOKEN` · `MAX_BOT_TOKEN` ·
`TG_TRANSPORT`/`MAX_TRANSPORT` polling|webhook (дефолт polling) ·
`TG_WEBHOOK_SECRET`/`MAX_WEBHOOK_SECRET` · `MAX_API_URL` · `MAX_HTML` (1) ·
`POLL_INTERVAL_SEC` (90) · `DEADLINE_POLL_EVERY` (10) · `MAX_CONCURRENT_POLLS` (8) ·
`PLATFORM_RPS` (5.0) · `ADMIN_TG_CHAT_ID` · `DEV_FAKE_AUTH` (0/1).

Настройка ботов (руками, один раз): Telegram — токен у @BotFather → `TG_BOT_TOKEN`
(кнопку miniapp сервис шлёт сам, тип `web_app`); MAX — бот в кабинете dev.max.ru →
`MAX_BOT_TOKEN`, URL miniapp прописать в кабинете.

## Деплой

Прод — systemd-сервис `s21notify`, каталог `/opt/s21notify` на своём сервере
(слабый LXC/VPS — компилировать на нём нельзя). Собирает GitHub Actions
(`.github/workflows/build.yml`: musl-бинарь + trunk/WASM → артефакт),
`deploy/deploy.sh` (bash) / `deploy/deploy.ps1` (PowerShell, для Windows-хоста)
качают артефакт (`gh run download`) и раскладывают на сервер. Хост — в окружении
`DEPLOY_HOST`. Оба скрипта делают одно и то же, запускаются с машины разработчика.

```sh
export DEPLOY_HOST=root@<сервер>   # Windows: $env:DEPLOY_HOST = "root@<сервер>"
./deploy/deploy.sh                 # Windows: ./deploy/deploy.ps1
ssh "$DEPLOY_HOST" "journalctl -u s21notify -n 50 --no-pager"
```

Здоровье живого инстанса — `GET /healthz`. Публичный https-домен направляется на
сервис самостоятельно (прокси/проброс) и указывается в `PUBLIC_URL`. `.env`
(токены ботов, ENCRYPTION_KEY, JWT_SECRET) — только на сервере, chmod 600; шаблон
`deploy/env.example`.

## Грабли (проверены на практике)

- **Polling vs webhook**: long polling и вебхук взаимоисключающи — Telegram отдаёт
  409 на getUpdates при активном вебхуке. `polling::run` снимает подписку
  (`delete_webhook`) перед стартом. Клиент MAX собран с общим timeout 15 с, поэтому
  long-poll запрос (`GET /updates`) переопределяет timeout на запрос.
- **GraphQL whitelist**: шлюз принимает только дословные тексты операций из белого
  списка (иначе HTTP 400, причина в заголовке `x-bad-request`). Тексты — в
  `crates/s21-platform/queries/*.graphql`, `.gitattributes` фиксирует им LF.
  **Не редактировать**; новые операции брать дословно из github.com/s21toolkit/s21schema.
- **MAX**: база `platform-api2.max.ru` (отзыв GlobalSign); TLS-цепочка от Russian
  Trusted Root CA (НУЦ Минцифры) — свой reqwest-Client с вшитым PEM
  (`crates/s21-adapters/certs/`), НЕ подмешивать этот CA клиентам платформы/TG.
  Токен в `Authorization` без `Bearer`; POST /messages — chat_id в query;
  `bot_started` кладёт user_id скаляром и приходит только при ПЕРВОМ старте
  (вернувшиеся шлют `message_created` с текстом `/start`). **Кнопки MAX сейчас не
  отображаются** (issue #3): код шлёт клавиатуру top-level полем `keyboard`
  (`max.rs`), но MAX/TamTam, вероятно, ждёт её вложением
  `attachments:[{type:"inline_keyboard"}]` — формат под вопросом, чинится.
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

## Ветки и релизы

Trunk-based: работаем в `main` + короткие фича-ветки → PR. Тест перед merge —
локальный режим (`APP_MODE=local`), отдельного staging нет. Релиз — пуш тега
`vX.Y.Z` (`.github/workflows/release.yml` собирает архивы Windows/Linux и
публикует GitHub Release; версия воркспейса = версия тега без `-dev`). Задачи и
роадмап — GitHub Issues + Projects (`scripts/setup-github-project.sh`).

## Секреты

Репозиторий публичный. `.env`, `*.db`, PEM с приватными данными и токены — вне git
(в .gitignore). Пароли студентов не хранятся и не логируются. Перед push проверять
diff на секреты; токены ботов в чат/логи не выводить.
