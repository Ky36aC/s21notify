# -*- coding: utf-8 -*-
"""Веб-интерфейс настройки и статуса (Flask, одна страница)."""

import logging

import requests
from flask import Flask, redirect, render_template_string, request

from .config import parse_remind_minutes
from .s21api import test_credentials

log = logging.getLogger("web")

PAGE = """<!doctype html>
<html lang="ru">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>s21notify</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  body { margin:0; font-family:'Segoe UI',system-ui,sans-serif; background:#141118; color:#e8e4ee; }
  .wrap { max-width:760px; margin:0 auto; padding:24px 16px 48px; }
  h1 { font-size:26px; margin:8px 0 2px; } h1 span { color:#a55eff; }
  .sub { color:#8d8699; margin-bottom:20px; }
  .card { background:#1d1926; border:1px solid #2e2839; border-radius:12px; padding:18px 20px; margin-bottom:16px; }
  .card h2 { font-size:15px; margin:0 0 14px; color:#c9b8ff; text-transform:uppercase; letter-spacing:.08em; }
  label { display:block; font-size:13px; color:#8d8699; margin:10px 0 4px; }
  input[type=text],input[type=password],input[type=number] {
    width:100%; padding:9px 12px; border-radius:8px; border:1px solid #3a3348;
    background:#14101c; color:#e8e4ee; font-size:14px; }
  input:focus { outline:none; border-color:#861be3; }
  .row { display:flex; gap:14px; flex-wrap:wrap; } .row > div { flex:1; min-width:180px; }
  .toggles { display:grid; grid-template-columns:repeat(auto-fit,minmax(220px,1fr)); gap:8px; margin-top:6px; }
  .toggles label { display:flex; align-items:center; gap:8px; margin:0; padding:8px 10px;
    background:#14101c; border:1px solid #2e2839; border-radius:8px; color:#e8e4ee; font-size:13.5px; cursor:pointer; }
  .btns { display:flex; gap:10px; flex-wrap:wrap; margin-top:16px; }
  button { padding:9px 18px; border-radius:8px; border:1px solid #3a3348; background:#251f31;
    color:#e8e4ee; font-size:14px; cursor:pointer; }
  button:hover { border-color:#861be3; }
  button.primary { background:#861be3; border-color:#861be3; font-weight:600; }
  button.primary:hover { background:#9a3cf0; }
  .msg { padding:10px 14px; border-radius:8px; margin-bottom:16px; font-size:14px; }
  .msg.ok { background:#15291b; border:1px solid #2e7d32; color:#9be3a8; }
  .msg.err { background:#2d1519; border:1px solid #b3363e; color:#f3a6ab; }
  .kv { display:flex; justify-content:space-between; padding:6px 0; border-bottom:1px solid #241f2e; font-size:14px; }
  .kv:last-child { border:none; }
  .kv .v { color:#c9b8ff; text-align:right; }
  .journal { font:12.5px/1.6 Consolas,monospace; max-height:300px; overflow-y:auto; }
  .journal div { padding:2px 0; border-bottom:1px solid #201b2a; }
  .journal .t { color:#6f6880; margin-right:8px; }
  .journal .error { color:#f3a6ab; } .journal .sent { color:#9be3a8; }
  .hint { font-size:12.5px; color:#6f6880; margin-top:6px; }
  code { background:#14101c; padding:1px 6px; border-radius:5px; }
</style>
</head>
<body><div class="wrap">
  <h1>s21<span>notify</span></h1>
  <div class="sub">Уведомления Школы 21 в Telegram — v{{ version }}</div>

  {% if msg %}<div class="msg {{ 'ok' if ok else 'err' }}">{{ msg }}</div>{% endif %}

  <div class="card">
    <h2>Статус</h2>
    <div class="kv"><span>Telegram-чат</span>
      <span class="v">{{ '✅ привязан (' + cfg.tg_chat_id + ')' if cfg.tg_chat_id else '❌ не привязан — напиши боту /start' }}</span></div>
    <div class="kv"><span>Последний успешный опрос</span>
      <span class="v">{{ last_poll or '—' }}</span></div>
    <div class="kv"><span>Последняя ошибка</span>
      <span class="v">{{ last_error or 'нет' }}</span></div>
  </div>

  <form method="post" action="/save">
  <div class="card">
    <h2>Платформа Школы 21</h2>
    <div class="row">
      <div><label>Логин (email)</label>
        <input type="text" name="s21_username" value="{{ cfg.s21_username }}" placeholder="login@student.21-school.ru"></div>
      <div><label>Пароль {{ '(сохранён — оставь пустым, чтобы не менять)' if cfg.s21_password }}</label>
        <input type="password" name="s21_password" value="" placeholder="{{ '••••••••' if cfg.s21_password else 'пароль от платформы' }}"></div>
    </div>
  </div>

  <div class="card">
    <h2>Telegram</h2>
    <label>Токен бота (создай бота у <a href="https://t.me/BotFather" style="color:#a55eff">@BotFather</a>)</label>
    <input type="text" name="tg_bot_token" value="{{ cfg.tg_bot_token }}" placeholder="123456789:AAE...">
    <div class="hint">После сохранения напиши своему боту <code>/start</code> — чат привяжется сам.</div>
  </div>

  <div class="card">
    <h2>Что присылать</h2>
    <div class="toggles">
      <label><input type="checkbox" name="notify_bookings" {{ 'checked' if cfg.notify_bookings }}> 🔔 Новые записи на проверку</label>
      <label><input type="checkbox" name="notify_changes" {{ 'checked' if cfg.notify_changes }}> ❌ Отмены и переносы</label>
      <label><input type="checkbox" name="notify_reminders" {{ 'checked' if cfg.notify_reminders }}> ⏰ Напоминания перед проверкой</label>
      <label><input type="checkbox" name="notify_feed" {{ 'checked' if cfg.notify_feed }}> 🏫 Лента уведомлений платформы</label>
      <label><input type="checkbox" name="notify_deadlines" {{ 'checked' if cfg.notify_deadlines }}> 📅 Дедлайны и экзамены</label>
      <label><input type="checkbox" name="notify_alarm" {{ 'checked' if cfg.notify_alarm }}> 🚨 Будильник без «я за компом»</label>
    </div>
    <div class="row" style="margin-top:14px">
      <div><label>Интервал опроса, сек</label>
        <input type="number" name="poll_interval_sec" min="30" max="3600" value="{{ cfg.poll_interval_sec }}"></div>
      <div><label>Напоминать за N минут (через запятую)</label>
        <input type="text" name="remind_minutes" value="{{ cfg.remind_minutes }}" placeholder="30, 15, 3"></div>
    </div>
    <div class="hint">Последнее напоминание приходит с кнопкой «✅ Я за компом». Если её не нажать,
      за 45 секунд до проверки бот включит будильник — сообщения каждые 10 секунд до старта.</div>
  </div>

  <div class="btns">
    <button class="primary" type="submit">💾 Сохранить</button>
    <button type="submit" formaction="/test-platform">Проверить платформу</button>
    <button type="submit" formaction="/test-bot">Проверить бота</button>
    {% if cfg.tg_chat_id %}<button type="submit" formaction="/reset-chat">Отвязать чат</button>{% endif %}
  </div>
  </form>

  <div class="card" style="margin-top:16px">
    <h2>Журнал</h2>
    <div class="journal">
      {% for t, kind, text in journal %}<div><span class="t">{{ t }}</span><span class="{{ kind }}">{{ text }}</span></div>
      {% else %}<div>пока пусто</div>{% endfor %}
    </div>
  </div>
</div></body></html>"""


def _form_config(config, form):
    """Собирает настройки из формы поверх сохранённых.

    Пустое значение логина/пароля/токена означает «не менять» — так случайный
    POST с неполной формой не затирает рабочие настройки."""
    updates = {
        "poll_interval_sec": max(30, int(form.get("poll_interval_sec") or 60)),
        "remind_minutes": ", ".join(
            map(str, parse_remind_minutes(form.get("remind_minutes") or ""))),
        "notify_bookings": "notify_bookings" in form,
        "notify_changes": "notify_changes" in form,
        "notify_reminders": "notify_reminders" in form,
        "notify_feed": "notify_feed" in form,
        "notify_deadlines": "notify_deadlines" in form,
        "notify_alarm": "notify_alarm" in form,
    }
    for key in ("s21_username", "s21_password", "tg_bot_token"):
        value = form.get(key, "").strip()
        if value:
            updates[key] = value
    return updates


def create_app(config, journal, api, version):
    app = Flask(__name__)

    def render(msg=None, ok=True):
        return render_template_string(
            PAGE,
            cfg=type("C", (), config.snapshot()),
            journal=journal.snapshot(),
            last_poll=journal.last_ok_poll.strftime("%d.%m %H:%M:%S") if journal.last_ok_poll else None,
            last_error=journal.last_error,
            msg=msg, ok=ok, version=version,
        )

    @app.get("/")
    def index():
        return render()

    @app.post("/save")
    def save():
        config.update(**_form_config(config, request.form))
        api.invalidate()
        return render("Сохранено. Настройки подхватятся при следующем опросе.", True)

    @app.post("/test-platform")
    def test_platform():
        updates = _form_config(config, request.form)
        cfg = {**config.snapshot(), **updates}
        if not cfg["s21_username"] or not cfg["s21_password"]:
            return render("Заполни логин и пароль платформы", False)
        ok, message = test_credentials(cfg["s21_username"], cfg["s21_password"])
        if ok:
            config.update(**updates)
            api.invalidate()
            message += " (настройки сохранены)"
        return render(message, ok)

    @app.post("/test-bot")
    def test_bot():
        updates = _form_config(config, request.form)
        token = updates.get("tg_bot_token") or config.get("tg_bot_token")
        if not token:
            return render("Вставь токен бота", False)
        try:
            r = requests.get(f"https://api.telegram.org/bot{token}/getMe", timeout=15).json()
        except requests.RequestException as e:
            return render(f"Сетевая ошибка: {e}", False)
        if r.get("ok"):
            config.update(**{**updates, "tg_bot_token": token})
            name = r["result"].get("username", "?")
            return render(f"Бот @{name} на связи (настройки сохранены). Теперь напиши ему /start", True)
        return render(f"Telegram отверг токен: {r.get('description', r)}", False)

    @app.post("/reset-chat")
    def reset_chat():
        config.update(tg_chat_id="")
        return render("Чат отвязан. Напиши боту /start, чтобы привязать заново.", True)

    return app
