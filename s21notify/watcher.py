# -*- coding: utf-8 -*-
"""Цикл опроса платформы: диффинг состояния и генерация уведомлений."""

import collections
import datetime as dt
import html
import logging
import re
import threading
import time

from . import queries
from .config import parse_remind_minutes
from .s21api import AuthError

log = logging.getLogger("watcher")

DEADLINE_WINDOW_DAYS = 30
DEADLINE_REMIND_HOURS = 24

# будильник: если «я за компом» не подтверждено, начинаем долбить
# за ALARM_BEFORE_SEC до старта, повторяя каждые ALARM_REPEAT_SEC
ALARM_BEFORE_SEC = 45
ALARM_REPEAT_SEC = 10

# эти типы уведомлений ленты дублируют собственные сообщения watcher'а
# (CALENDAR — «кто-то записался на проверку», DASHBOARD — «проверка скоро начнётся»)
SKIP_FEED_TYPES = {"CALENDAR", "DASHBOARD"}


def ack_markup(bid):
    """Кнопка подтверждения «я за компом» для последнего напоминания и будильника."""
    return {"inline_keyboard": [[{"text": "✅ Я за компом", "callback_data": f"ack:{bid}"}]]}


# ---------------------------------------------------------------- утилиты
def parse_ts(iso):
    return dt.datetime.fromisoformat(iso.replace("Z", "+00:00"))


def fmt_time(iso):
    try:
        return parse_ts(iso).astimezone().strftime("%d.%m %H:%M")
    except Exception:
        return str(iso)


def strip_html(s):
    return re.sub(r"<[^>]+>", "", s or "").strip()


def esc(s):
    return html.escape(str(s))


class Journal:
    """Журнал последних событий для веб-интерфейса."""

    def __init__(self, maxlen=100):
        self._items = collections.deque(maxlen=maxlen)
        self._lock = threading.Lock()
        self.last_ok_poll = None
        self.last_error = None

    def add(self, text, kind="info"):
        with self._lock:
            self._items.append(
                (dt.datetime.now().strftime("%d.%m %H:%M:%S"), kind, text)
            )
        (log.info if kind != "error" else log.error)(text)

    def snapshot(self):
        with self._lock:
            return list(self._items)[::-1]


# дедлайны у платформы обычно в статусе SHIFTED (перенесённый), а не OPEN;
# сервер отвечает на этот запрос долго — нужен увеличенный таймаут
DEADLINE_STATUSES = ["OPEN", "SHIFTED", "OVERDUE"]


def fetch_deadlines(api):
    """Список дедлайнов: [{id, ts, title}], отсортирован по близости."""
    data = api.gql(queries.DEADLINES_OP, queries.DEADLINES_QUERY, {
        "deadlineStatuses": DEADLINE_STATUSES,
        "page": {"offset": 0, "limit": 50},
        "deadlinesFrom": None,
        "deadlinesTo": None,
        "sorting": None,
    }, timeout=90)
    out = []
    for it in (data.get("student") or {}).get("getDeadlines") or []:
        d = it.get("deadline") or {}
        names = [
            (g.get("project") or {}).get("goalName", "?")
            for g in ((it.get("deadlineGoal") or {}).get("goalProjects") or [])
        ]
        title = " / ".join(n for n in names if n) \
            or strip_html(d.get("description")) or "дедлайн"
        out.append({
            "id": str(d.get("deadlineId")),
            "ts": d.get("deadlineTs", ""),
            "title": title,
        })
    out.sort(key=lambda x: x["ts"])
    return out


def days_left(ts, now):
    try:
        return max(0, int((parse_ts(ts) - now).total_seconds() // 86400))
    except Exception:
        return None


def booking_info(b, me):
    """Компактный словарь по брони — хранится в state для отмен/переносов."""
    task = (b.get("task") or {})
    verifier = ((b.get("verifierUser") or {}).get("login")) or "?"
    verifiable = (((b.get("verifiableStudent") or {}).get("user")) or {}).get("login") \
        or ((b.get("team") or {}).get("teamName")) or "?"
    return {
        "start": (b.get("eventSlot") or {}).get("start", ""),
        "task": task.get("goalName") or task.get("title") or "?",
        "verifier": verifier,
        "verifiable": verifiable,
        "online": bool(b.get("isOnline")),
        "status": b.get("bookingStatus", ""),
    }


def fmt_booking_line(info, me):
    if info["verifier"] == me:
        role, who = "🔍 Ты проверяешь", info["verifiable"]
    else:
        role, who = "📝 Тебя проверяет", info["verifier"]
    online = " (онлайн)" if info["online"] else ""
    return (
        f"{role} <b>{esc(who)}</b>{online}\n"
        f"📦 {esc(info['task'])}\n"
        f"🕐 {fmt_time(info['start'])}"
    )


class Watcher(threading.Thread):
    """Фоновый поток: опрашивает платформу и шлёт события через send_fn."""

    def __init__(self, config, state, api, journal, send_fn):
        super().__init__(daemon=True, name="watcher")
        self.config = config
        self.state = state
        self.api = api
        self.journal = journal
        self.send = send_fn
        self._wake = threading.Event()
        self._auth_fail_until = 0

    def poll_now(self):
        self._wake.set()

    def run(self):
        cold = self.state.is_cold_start
        while True:
            cfg = self.config.snapshot()
            if cfg["s21_username"] and cfg["s21_password"]:
                now = dt.datetime.now(dt.timezone.utc)
                if now.timestamp() >= self._auth_fail_until:
                    try:
                        self.cycle(cfg, now, cold)
                        cold = False
                        self.journal.last_ok_poll = dt.datetime.now()
                        self.journal.last_error = None
                    except AuthError as e:
                        self.journal.last_error = str(e)
                        self.journal.add(f"ошибка входа: {e} (пауза 5 минут)", "error")
                        self._auth_fail_until = now.timestamp() + 300
                    except Exception as e:
                        self.journal.last_error = str(e)
                        self.journal.add(f"ошибка опроса: {e}", "error")
            self._wake.wait(timeout=max(15, int(cfg["poll_interval_sec"])))
            self._wake.clear()

    # ------------------------------------------------------------ один опрос
    def cycle(self, cfg, now, cold):
        me = cfg["s21_username"].split("@")[0]
        st = self.state.data

        self._check_bookings(cfg, now, cold, me, st)
        if cfg["notify_feed"]:
            self._check_feed(st)
        if cfg["notify_deadlines"]:
            self._check_deadlines(now, cold, st)
            self._check_exams(now, cold, st)

        self.state.save()

    def _notify(self, text, reply_markup=None):
        self.send(text, reply_markup)
        self.journal.add(strip_html(text).split("\n")[0], "sent")

    # ------------------------------------------------------------ проверки
    def _check_bookings(self, cfg, now, cold, me, st):
        to = (now + dt.timedelta(days=14)).isoformat()
        data = self.api.gql(queries.BOOKINGS_OP, queries.BOOKINGS_QUERY,
                            {"to": to, "limit": 50})
        bookings = (data.get("student") or {}).get("getMyUpcomingBookings") or []
        current = {str(b["id"]): booking_info(b, me) for b in bookings}

        # миграция со старого формата state (v1 хранил только список id)
        prev = st.get("bookings")
        if prev is None:
            legacy_seen = set(st.get("seen_bookings", []))
            prev = {bid: info for bid, info in current.items() if bid in legacy_seen}
            if not legacy_seen and not cold:
                prev = dict(current)

        thresholds = parse_remind_minutes(cfg["remind_minutes"])
        reminded = st.get("reminded_bookings", {})
        if isinstance(reminded, list):  # v2.0 хранил просто список id
            reminded = {bid: list(thresholds) for bid in reminded}

        for bid, info in current.items():
            old = prev.get(bid)
            if old is None:
                if cfg["notify_bookings"]:
                    self._notify("🔔 <b>Новая запись на проверку</b>\n"
                                 + fmt_booking_line(info, me))
            elif old["start"] != info["start"]:
                if cfg["notify_changes"]:
                    self._notify(
                        "🔁 <b>Проверка перенесена</b>\n"
                        + fmt_booking_line(info, me)
                        + f"\n(было {fmt_time(old['start'])})"
                    )
                reminded.pop(bid, None)

        for bid, old in prev.items():
            if bid in current:
                continue
            try:
                still_future = parse_ts(old["start"]) > now
            except Exception:
                still_future = False
            if still_future and cfg["notify_changes"]:
                self._notify("❌ <b>Проверка отменена</b>\n" + fmt_booking_line(old, me))
            reminded.pop(bid, None)

        if cfg["notify_reminders"]:
            for bid, info in current.items():
                try:
                    start = parse_ts(info["start"])
                except Exception:
                    continue
                left = start - now
                if left <= dt.timedelta(0):
                    continue
                fired = reminded.setdefault(bid, [])
                due = [t for t in thresholds
                       if t not in fired and left <= dt.timedelta(minutes=t)]
                if not due:
                    continue
                # пересекли сразу несколько порогов (например, запись за 10 мин
                # до старта) — шлём одно сообщение, помечаем все пороги
                minutes = max(1, int(left.total_seconds() // 60))
                text = (f"⏰ <b>Проверка через {minutes} мин</b>\n"
                        + fmt_booking_line(info, me))
                markup = None
                if min(due) == thresholds[-1] and cfg["notify_alarm"] \
                        and bid not in st.get("acked_bookings", []):
                    text += "\n\nНажми кнопку, иначе перед стартом включу будильник 🚨"
                    markup = ack_markup(bid)
                self._notify(text, markup)
                fired.extend(due)

        st["bookings"] = current
        st["reminded_bookings"] = {b: f for b, f in reminded.items() if b in current}
        st["acked_bookings"] = [b for b in set(st.get("acked_bookings", [])) if b in current]
        st.pop("seen_bookings", None)

    def _check_feed(self, st):
        data = self.api.gql(queries.NOTIFICATIONS_OP, queries.NOTIFICATIONS_QUERY,
                            {"paging": {"offset": 0, "limit": 20}})
        notifs = ((data.get("s21Notification") or {}).get("getS21Notifications") or {}) \
            .get("notifications") or []
        seen = set(st.get("seen_notifications", []))
        first_run = "seen_notifications" not in st
        for n in notifs:
            if str(n["id"]) in seen or first_run:
                continue
            if n.get("relatedObjectType") in SKIP_FEED_TYPES:
                continue  # дубль нашего же уведомления о записи/напоминания
            msg = strip_html(n.get("message"))
            self._notify(f"🏫 <b>Школа 21</b> · {esc(n.get('groupName', ''))}\n"
                         f"{esc(msg)}\n🕐 {fmt_time(n.get('time', ''))}")
        st["seen_notifications"] = list(
            {str(n["id"]) for n in notifs} | set(list(seen)[-500:])
        )

    def _check_deadlines(self, now, cold, st):
        deadlines = fetch_deadlines(self.api)
        prev = st.get("deadlines", {})
        known = "deadlines" in st
        current, reminded = {}, set(st.get("reminded_deadlines", []))

        for item in deadlines:
            did, ts, goals = item["id"], item["ts"], item["title"]
            current[did] = {"ts": ts, "title": goals}

            if known and did not in prev:
                self._notify(f"📅 <b>Новый дедлайн</b>\n{esc(goals)}\n🕐 {fmt_time(ts)}")
            elif known and prev.get(did, {}).get("ts") != ts:
                self._notify(f"📅 <b>Дедлайн перенесён</b>\n{esc(goals)}\n"
                             f"🕐 {fmt_time(ts)} (было {fmt_time(prev[did]['ts'])})")
                reminded.discard(did)

            try:
                left = parse_ts(ts) - now
                if did not in reminded and dt.timedelta(0) < left <= dt.timedelta(hours=DEADLINE_REMIND_HOURS):
                    hours = max(1, int(left.total_seconds() // 3600))
                    self._notify(f"⚠️ <b>Дедлайн через ~{hours} ч</b>\n"
                                 f"{esc(goals)}\n🕐 {fmt_time(ts)}")
                    reminded.add(did)
            except Exception:
                pass

        st["deadlines"] = current
        st["reminded_deadlines"] = [d for d in reminded if d in current]

    def _check_exams(self, now, cold, st):
        data = self.api.gql(queries.EXAMS_OP, queries.EXAMS_QUERY, {
            "from": now.isoformat(),
            "to": (now + dt.timedelta(days=DEADLINE_WINDOW_DAYS)).isoformat(),
        })
        exams = (data.get("student") or {}).get("getExams") or []
        prev = st.get("exams", {})
        known = "exams" in st
        current, reminded = {}, set(st.get("reminded_exams", []))

        for e in exams:
            eid = str(e.get("examId"))
            begin = e.get("beginDate", "")
            name = e.get("name") or e.get("goalName") or "экзамен"
            current[eid] = {"ts": begin, "title": name}
            if known and eid not in prev:
                self._notify(f"🎓 <b>Новый экзамен</b>\n{esc(name)}\n🕐 {fmt_time(begin)}")
            try:
                left = parse_ts(begin) - now
                if eid not in reminded and dt.timedelta(0) < left <= dt.timedelta(hours=DEADLINE_REMIND_HOURS):
                    self._notify(f"🎓 <b>Экзамен уже завтра</b>\n{esc(name)}\n🕐 {fmt_time(begin)}")
                    reminded.add(eid)
            except Exception:
                pass

        st["exams"] = current
        st["reminded_exams"] = [e for e in reminded if e in current]


class Alarm(threading.Thread):
    """Будильник: проверка вот-вот начнётся, а «я за компом» не нажато — долбим.

    Отдельный поток, потому что цикл watcher'а ходит раз в poll_interval (60 с),
    а тут нужна точность в секунды. В API не ходит — читает последний снапшот
    броней из state.
    """

    def __init__(self, config, state, journal, send_fn):
        super().__init__(daemon=True, name="alarm")
        self.config = config
        self.state = state
        self.journal = journal
        self.send = send_fn
        # осторожно с именами: threading.Thread занимает _started, _handle и др.
        self._last_sent = {}   # bid -> time.monotonic() последнего сообщения
        self._alarmed = set()  # брони, по которым будильник уже звенел (для журнала)

    def run(self):
        while True:
            time.sleep(5)
            try:
                self._tick()
            except Exception as e:
                log.warning("будильник: %s", e)

    def _tick(self):
        cfg = self.config.snapshot()
        if not (cfg["notify_alarm"] and cfg["notify_reminders"] and cfg["tg_chat_id"]):
            return
        now = dt.datetime.now(dt.timezone.utc)
        me = cfg["s21_username"].split("@")[0]
        st = self.state.data
        acked = set(st.get("acked_bookings", []))

        for bid, info in dict(st.get("bookings", {})).items():
            if bid in acked:
                continue
            try:
                left = (parse_ts(info["start"]) - now).total_seconds()
            except Exception:
                continue
            if not (0 < left <= ALARM_BEFORE_SEC):
                continue
            if time.monotonic() - self._last_sent.get(bid, 0) < ALARM_REPEAT_SEC:
                continue
            self._last_sent[bid] = time.monotonic()
            if bid not in self._alarmed:
                self._alarmed.add(bid)
                self.journal.add("🚨 будильник: подтверждение не нажато, начинаю звонить")
            self.send(
                f"🚨🚨🚨 <b>ПРОВЕРКА ЧЕРЕЗ {int(left)} СЕК!</b>\n"
                + fmt_booking_line(info, me),
                ack_markup(bid),
            )
