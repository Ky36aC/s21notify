# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Проект русскоязычный: README, комментарии, тексты уведомлений и коммиты — на русском.

## Что это

Демон, пересылающий события platform.21-school.ru («Школа 21») в Telegram: записи на проверку, переносы/отмены, каскад напоминаний с кнопкой «✅ Я за компом» и спам-будильником, дедлайны/экзамены, лента уведомлений платформы. Один процесс, четыре потока: Watcher (опрос раз в 60 с), Bot (long polling), Alarm (тик раз в 5 с), Flask-веб на :8021. Без сторонних Telegram-библиотек — сырой Bot API через requests.

## Команды

```sh
py run.py                      # запуск локально (Windows); Linux: python3 run.py
```

Тестов и линтера в репозитории нет; логика проверяется офлайн-скриптами с фейками (FakeConfig/FakeState/FakeApi + send_fn-заглушка — образец был в scratchpad, test_v21.py) и живым прогоном.

Продакшен — LXC `ssh root@s21notify.lan` (Debian 13, py3.13), systemd-сервис `s21notify`, каталог `/opt/s21notify`. **Это не git-клон** — деплой руками:

```sh
scp s21notify/watcher.py root@s21notify.lan:/opt/s21notify/s21notify/
ssh root@s21notify.lan "systemctl restart s21notify && sleep 3 && systemctl is-active s21notify"
ssh root@s21notify.lan "journalctl -u s21notify -n 50 --no-pager"   # логи
```

После рестарта `is-active` может показать `activating` — если сервис падает и рестартится, причина в journalctl. Здоровье живого инстанса видно на `http://s21notify.lan:8021` (last_poll, last_error).

## Архитектура

- `s21notify/main.py` — собирает Config, State, Journal, S21Api, запускает потоки; Flask крутится в главном потоке.
- `s21notify/config.py` — `Config` (config.json, горячая перезагрузка по mtime — веб-форма и демон видят изменения друг друга без рестарта), `State` (state.json: seen-списки, снапшоты броней `{bid: info}`, `reminded_bookings` — dict {bid: [сработавшие пороги]}, `acked_bookings`), `parse_remind_minutes` («30, 15, 3» → список порогов).
- `s21notify/s21api.py` — эмуляция логина через форму Keycloak (client_id=school21, парсинг `window.loginAction`) + `gql()`; сам перелогинивается по истечении JWT, context-headers берёт из REST `/edu-context/context-info`. Потокобезопасен (Watcher и Bot ходят из разных потоков).
- `s21notify/queries.py` — тексты GraphQL-операций. **НЕ РЕДАКТИРОВАТЬ тексты запросов**: шлюз платформы принимает только дословные операции из белого списка официального клиента (иначе HTTP 400, причина — в заголовке ответа `x-bad-request`). Тексты взяты из github.com/s21toolkit/s21schema; переменные менять можно, текст — нет. Новые операции брать оттуда же дословно.
- `s21notify/watcher.py` — `Watcher` (диффинг со снапшотом в state: новые/отменённые/перенесённые брони, каскад напоминаний со схлопыванием пропущенных порогов, дедлайны, лента), `Alarm` (отдельный поток: в API не ходит, читает снапшот броней из state; за `ALARM_BEFORE_SEC` до старта неподтверждённой брони шлёт 🚨 каждые `ALARM_REPEAT_SEC`), `Journal` (лента событий для веб-страницы).
- `s21notify/bot.py` — команды (/reviews /agenda /deadlines /status /check), привязка chat_id по первому /start, callback `ack:<bid>` пишет в `state.data["acked_bookings"]` и гасит будильник. Чужие chat_id игнорируются.
- `s21notify/web.py` — одна страница настройки/статуса. Пустое поле логина/пароля/токена в POST = «не менять» (защита от затирания).

Поток уведомлений: Watcher/Alarm → `send_fn=bot.send_to_user` → Telegram. Связь между потоками — через Config/State/Journal, не напрямую.

## Грабли (проверены на практике)

- **Подклассы `threading.Thread`**: нельзя атрибуты `_handle`, `_started` и прочие приватные имена Thread — py3.13 их затирает/читает сам. Ловилось дважды. Офлайн-тест с прямым вызовом методов это НЕ ловит — только реальный `.start()`.
- **Логин платформы** — только короткий ник (без `@student.21-school.ru`), иначе «Invalid login and/or password».
- **Дедлайны** приходят со статусом `SHIFTED` (не OPEN); запрос `deadlinesGetDeadlines` медленный — таймаут 90 с, окно дат не передавать.
- **Лента**: типы `CALENDAR` и `DASHBOARD` дублируют собственные сообщения watcher'а и фильтруются (`SKIP_FEED_TYPES`); `PROJECT` («выставлена оценка») — уникален, должен проходить.
- **Windows-консоль cp1251**: эмодзи в `print` тестовых скриптов роняют UnicodeEncodeError — либо reconfigure на utf-8 (как в main.py), либо без эмодзи.
- На LXC нет `curl` — проверять HTTP через `python3 -c "import urllib.request..."`.

## Секреты

Репозиторий публичный. `config.json` (пароль платформы, токен бота) и `state.json` — в .gitignore и существуют только локально и на LXC. Перед каждым push проверять diff на секреты; токен бота не выводить в чат/логи.
