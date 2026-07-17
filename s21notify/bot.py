# -*- coding: utf-8 -*-
"""Telegram-бот: отправка уведомлений + команды (long polling, без сторонних библиотек)."""

import datetime as dt
import logging
import threading
import time

import requests

from . import queries
from .watcher import esc, fmt_booking_line, fmt_time, strip_html, booking_info

log = logging.getLogger("bot")

HELP_TEXT = (
    "<b>Команды:</b>\n"
    "/reviews — ближайшие проверки\n"
    "/agenda — события на 7 дней\n"
    "/deadlines — активные дедлайны\n"
    "/status — уровень, XP, коины\n"
    "/check — проверить прямо сейчас\n"
    "/help — эта справка"
)


class Bot(threading.Thread):
    def __init__(self, config, api, journal, watcher=None):
        super().__init__(daemon=True, name="bot")
        self.config = config
        self.api = api
        self.journal = journal
        self.watcher = watcher
        self._offset = 0

    # ------------------------------------------------------------ отправка
    def _api_call(self, method, payload):
        token = self.config.get("tg_bot_token")
        if not token:
            return None
        try:
            r = requests.post(
                f"https://api.telegram.org/bot{token}/{method}",
                json=payload, timeout=65,
            )
            data = r.json()
            if not data.get("ok"):
                log.warning("telegram %s: %s", method, data)
            return data
        except requests.RequestException as e:
            log.warning("telegram %s failed: %s", method, e)
            return None

    def send_to_user(self, text):
        chat_id = self.config.get("tg_chat_id")
        if not chat_id:
            self.journal.add("сообщение не отправлено: chat_id не привязан (напиши боту /start)", "error")
            return
        self._api_call("sendMessage", {
            "chat_id": chat_id, "text": text,
            "parse_mode": "HTML", "disable_web_page_preview": True,
        })

    # ------------------------------------------------------------ приём
    def run(self):
        while True:
            token = self.config.get("tg_bot_token")
            if not token:
                time.sleep(5)
                continue
            try:
                r = requests.get(
                    f"https://api.telegram.org/bot{token}/getUpdates",
                    params={"offset": self._offset, "timeout": 50,
                            "allowed_updates": '["message"]'},
                    timeout=65,
                )
                data = r.json()
            except (requests.RequestException, ValueError) as e:
                log.warning("getUpdates: %s", e)
                time.sleep(10)
                continue
            if not data.get("ok"):
                log.warning("getUpdates: %s", data)
                time.sleep(10)
                continue
            for upd in data.get("result", []):
                self._offset = upd["update_id"] + 1
                try:
                    self._handle(upd.get("message") or {})
                except Exception as e:
                    log.exception("ошибка обработки команды: %s", e)

    def _handle(self, msg):
        chat_id = str((msg.get("chat") or {}).get("id", ""))
        text = (msg.get("text") or "").strip().lower()
        if not chat_id or not text:
            return

        bound = str(self.config.get("tg_chat_id") or "")

        if text.startswith("/start"):
            if not bound:
                self.config.update(tg_chat_id=chat_id)
                self.journal.add(f"привязан chat_id {chat_id}")
                self._reply(chat_id,
                            "✅ <b>Готово, уведомления привязаны к этому чату.</b>\n\n" + HELP_TEXT)
            elif bound == chat_id:
                self._reply(chat_id, "Уже привязано к этому чату 👌\n\n" + HELP_TEXT)
            # чужой чат — молча игнорируем
            return

        if bound != chat_id:
            return  # команды принимаем только от владельца

        cmd = text.split()[0].split("@")[0]
        handler = {
            "/reviews": self._cmd_reviews,
            "/agenda": self._cmd_agenda,
            "/deadlines": self._cmd_deadlines,
            "/status": self._cmd_status,
            "/check": self._cmd_check,
            "/help": lambda: HELP_TEXT,
        }.get(cmd)
        if not handler:
            return
        try:
            self._reply(chat_id, handler())
        except Exception as e:
            self._reply(chat_id, f"⚠️ Не получилось: {esc(e)}")

    def _reply(self, chat_id, text):
        self._api_call("sendMessage", {
            "chat_id": chat_id, "text": text,
            "parse_mode": "HTML", "disable_web_page_preview": True,
        })

    # ------------------------------------------------------------ команды
    def _me(self):
        return self.config.get("s21_username").split("@")[0]

    def _cmd_reviews(self):
        now = dt.datetime.now(dt.timezone.utc)
        data = self.api.gql(queries.BOOKINGS_OP, queries.BOOKINGS_QUERY,
                            {"to": (now + dt.timedelta(days=14)).isoformat(), "limit": 50})
        bookings = (data.get("student") or {}).get("getMyUpcomingBookings") or []
        if not bookings:
            return "Записей на проверку нет 🙌"
        me = self._me()
        parts = [fmt_booking_line(booking_info(b, me), me) for b in bookings]
        return "<b>Ближайшие проверки:</b>\n\n" + "\n\n".join(parts)

    def _cmd_agenda(self):
        now = dt.datetime.now(dt.timezone.utc)
        data = self.api.gql(queries.AGENDA_OP, queries.AGENDA_QUERY, {
            "from": now.isoformat(),
            "to": (now + dt.timedelta(days=7)).isoformat(),
            "limit": 30,
        })
        events = (data.get("calendarEventS21") or {}).get("getMyAgendaEvents") or []
        if not events:
            return "На ближайшую неделю событий нет 🙌"
        lines = [
            f"• {fmt_time(e.get('start', ''))} — {esc(e.get('label') or strip_html(e.get('description')) or e.get('agendaEventType', '?'))}"
            for e in events
        ]
        return "<b>События на 7 дней:</b>\n" + "\n".join(lines)

    def _cmd_deadlines(self):
        now = dt.datetime.now(dt.timezone.utc)
        data = self.api.gql(queries.DEADLINES_OP, queries.DEADLINES_QUERY, {
            "deadlineStatuses": ["OPEN"],
            "page": {"offset": 0, "limit": 50},
            "deadlinesFrom": now.isoformat(),
            "deadlinesTo": (now + dt.timedelta(days=60)).isoformat(),
            "sorting": None,
        })
        items = (data.get("student") or {}).get("getDeadlines") or []
        if not items:
            return "Открытых дедлайнов нет 🙌"
        lines = []
        for it in items:
            d = it.get("deadline") or {}
            goals = ", ".join(
                (g.get("project") or {}).get("goalName", "?")
                for g in ((it.get("deadlineGoal") or {}).get("goalProjects") or [])
            ) or strip_html(d.get("description")) or "дедлайн"
            lines.append(f"• {fmt_time(d.get('deadlineTs', ''))} — {esc(goals)}")
        return "<b>Дедлайны:</b>\n" + "\n".join(lines)

    def _cmd_status(self):
        data = self.api.gql(queries.EXPERIENCE_OP, queries.EXPERIENCE_QUERY, {})
        xp = (data.get("student") or {}).get("getExperience") or {}
        level = ((xp.get("level") or {}).get("range") or {}).get("levelCode", "?")
        return (
            f"<b>{esc(self._me())}</b>\n"
            f"🏅 Уровень: {level}\n"
            f"🍪 Печеньки: {xp.get('cookiesCount', '?')}\n"
            f"⭐ Code-review points: {xp.get('codeReviewPoints', '?')}\n"
            f"🪙 Коины: {xp.get('coinsCount', '?')}"
        )

    def _cmd_check(self):
        if self.watcher:
            self.watcher.poll_now()
        return "🔄 Проверяю прямо сейчас — если есть что-то новое, пришлю отдельным сообщением."
