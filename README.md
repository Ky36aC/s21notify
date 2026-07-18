# s21notify

Мультипользовательский сервис уведомлений **Школы 21** в **Telegram и MAX**
(на Rust). Регистрация и настройки — через miniapp внутри мессенджера.

Для любого числа студентов и в двух мессенджерах одновременно: 🔔 новые записи
на проверку, 🔁/❌ переносы и отмены, ⏰ каскад
напоминаний с кнопкой «✅ Я за компом» и 🚨 будильником, 📅 дедлайны и экзамены,
🏫 лента платформы без дублей. Команды `/reviews /agenda /deadlines /status
/check /help` работают в обоих мессенджерах.

## Модель безопасности

Пароли студентов **не хранятся**. При входе в miniapp пароль один раз
отправляется на платформу, та выдаёт **offline-токен** (Keycloak,
`scope=offline_access`), сервис шифрует его AES-256-GCM и хранит только токен;
пароль отбрасывается. Отзыв доступа = смена пароля на платформе (сервис попросит
войти заново). Токены ботов и ключ шифрования — в `.env` на сервере, вне git.

## Архитектура (cargo workspace)

| Крейт | Назначение |
|---|---|
| `s21-core` | Чистый диффинг и форматирование (порт `watcher.py`), без I/O. `run_cycle()` |
| `s21-platform` | Keycloak (offline-токены), GraphQL (дословный whitelist), REST context |
| `s21-adapters` | Trait `MessengerAdapter` + Telegram (teloxide-core) и MAX (свой клиент) |
| `s21-server` | axum, SQLite (sqlx), watcher/alarm-таски, HTTP API, вебхуки |
| `miniapp` | Leptos CSR (wasm) — регистрация, настройки, статус |

Один процесс: HTTP-сервер (:80) + tokio-таска опроса на пользователя (джиттер,
семафор `MAX_CONCURRENT_POLLS`, rate-limit платформы) + одна alarm-таска (тик 5 с)
+ housekeeping (чистка журнала, перестановка вебхука MAX). Состояние — SQLite
(WAL); access-токены только в памяти, перерефрешиваются после рестарта.

## Сборка

Требуется тулчейн для двух целей (musl-бинарь сервера + wasm miniapp):

```sh
rustup target add x86_64-unknown-linux-musl wasm32-unknown-unknown
cargo install trunk
```

- Тесты: `cargo test --workspace --exclude s21-miniapp`
- Сервер (статический): `cargo build --release --target x86_64-unknown-linux-musl -p s21-server`
- Miniapp: `cd miniapp && trunk build --release` (в `miniapp/dist/`)

На Windows нужен линкер: GNU-тулчейн (`rustup override set stable-x86_64-pc-windows-gnu`)
+ `gcc` из mingw. Всё это делает **GitHub Actions** (`.github/workflows/build.yml`) —
компилировать на LXC (512 МБ) нельзя.

### Живые проверки

- `cargo run -p s21-platform --example live_check` — логин/refresh/GraphQL на своём
  аккаунте (креды берутся из `config.json` в корне).
- `cargo run -p s21-adapters --example send_test` — отправка себе в TG/MAX (нужны
  `TG_BOT_TOKEN`+`TG_TEST_CHAT_ID` / `MAX_BOT_TOKEN`+`MAX_TEST_CHAT_ID`).

### Локальная отладка miniapp

Запусти сервер с `DEV_FAKE_AUTH=1` на :8021, затем `cd miniapp && trunk serve` и
открой `http://127.0.0.1:8080/?dev=<любой_id>&m=telegram`. В dev-режиме
`/api/auth` принимает `init_data="dev:<id>"` без подписи.

## Локальный запуск (на своём ПК, без домена)

Если не хочешь пользоваться общим ботом — запусти свой экземпляр у себя. Домен
не нужен: апдейты идут long polling'ом, настройки открываются в браузере.

1. Скачай **[`s21notify-windows.zip` из последнего релиза](https://github.com/Ky36aC/s21notify/releases/latest)**
   и распакуй. Внутри — `s21notify.exe` (miniapp уже встроена, ничего ставить не
   нужно) и рядом готовый `.env`.
2. Открой `.env` блокнотом и впиши в `TG_BOT_TOKEN=` токен своего бота от
   [@BotFather](https://t.me/BotFather). Сохрани.
3. Запусти `s21notify.exe`. `ENCRYPTION_KEY`/`JWT_SECRET` сгенерируются сами.
4. Открой своего бота в Telegram, нажми **/start**.
5. Открой `http://127.0.0.1:8080`, введи логин/пароль Школы 21 — готово,
   уведомления пойдут в твоего бота.

Пароль Школы 21 вводится один раз и не сохраняется (хранится только шифрованный
offline-токен, ключ — в локальном `.env`). Всё работает офлайн-приватно на твоём ПК.

## Релизы

Готовые сборки лежат в [релизах](https://github.com/Ky36aC/s21notify/releases).
Выпуск — пуш тега `vX.Y.Z` (workflow `.github/workflows/release.yml` соберёт обе
платформы и приложит архивы):

```sh
git tag v3.0.0 && git push origin v3.0.0
```

- `s21notify-windows.zip` — папка с `s21notify.exe` и готовым `.env` (для учеников,
  локальный режим);
- `s21notify-linux.tar.gz` — `s21-server` + `static/` + юнит + `env.example` (сервер).

## Деплой

Сервис — один статический бинарь + `static/` под systemd; ставится в
`/opt/s21notify`. Компиляция — только в CI, на сервер едет готовый артефакт
(на слабой машине вроде 512 МБ LXC компилировать нельзя).

```sh
# Linux/macOS (bash)
export DEPLOY_HOST=root@адрес-твоего-сервера   # можно и DEPLOY_DIR (дефолт /opt/s21notify)
./deploy/deploy.sh              # берёт последний успешный build ветки main
./deploy/deploy.sh <run_id>     # конкретный запуск CI
```

```powershell
# Windows (PowerShell) — то же самое
$env:DEPLOY_HOST = "root@адрес-твоего-сервера"
./deploy/deploy.ps1             # последний успешный build ветки main
./deploy/deploy.ps1 <run_id>    # конкретный запуск CI
```

Обе версии делают одно и то же и запускаются **с твоей машины** (не на сервере):
качают артефакт через `gh`, копируют по `scp`/`ssh` и рестартят сервис. Нужны
установленные `gh` (авторизован) и OpenSSH-клиент (в Windows 10/11 есть из коробки).

Скрипт качает артефакт (`gh run download`), кладёт бинарь + `static/` + юнит на
сервер, рестартит сервис и проверяет `/healthz`. Первый деплой попросит заполнить
`.env` из `deploy/env.example` (там расписано, откуда какие значения брать).

Публичный https-домен нужно направить на сервис самому (обратный прокси или
проброс порта) и указать его в `PUBLIC_URL`. Вебхуки ставятся автоматически при
старте; MAX-подписку housekeeping переустанавливает ежечасно (MAX снимает её
после ~8 ч недоступности).

## Настройка ботов (делается один раз руками)

- **Telegram**: у [@BotFather](https://t.me/BotFather) создать бота, токен → `.env`
  `TG_BOT_TOKEN`; кнопку miniapp сервис шлёт сам (тип `web_app`).
- **MAX**: в кабинете [dev.max.ru](https://dev.max.ru) создать бота, токен → `.env`
  `MAX_BOT_TOKEN`; URL miniapp прописать в кабинете.
