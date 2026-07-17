# -*- coding: utf-8 -*-
"""Конфигурация: config.json рядом с проектом, горячая перезагрузка по mtime."""

import json
import re
import threading
from pathlib import Path

BASE_DIR = Path(__file__).resolve().parent.parent
CONFIG_PATH = BASE_DIR / "config.json"
STATE_PATH = BASE_DIR / "state.json"

DEFAULTS = {
    "s21_username": "",
    "s21_password": "",
    "tg_bot_token": "",
    "tg_chat_id": "",
    "poll_interval_sec": 60,
    "remind_minutes": "30, 15, 3",
    "notify_bookings": True,   # новые записи на проверку
    "notify_changes": True,    # отмены и переносы
    "notify_reminders": True,  # напоминания перед проверкой
    "notify_feed": True,       # лента уведомлений платформы
    "notify_deadlines": True,  # дедлайны и экзамены
    "notify_alarm": True,      # будильник, если не подтверждено «я за компом»
    "web_host": "0.0.0.0",
    "web_port": 8021,
}


def parse_remind_minutes(value):
    """«30, 15, 3» (или int из старых конфигов) → отсортированный по убыванию список."""
    if isinstance(value, (int, float)):
        return [max(1, min(720, int(value)))]
    minutes = set()
    for part in re.split(r"[,;\s]+", str(value)):
        if part.isdigit() and 1 <= int(part) <= 720:
            minutes.add(int(part))
    return sorted(minutes, reverse=True) or [30]


class Config:
    """Потокобезопасный доступ к config.json с автоперечиткой при изменении файла."""

    def __init__(self, path=CONFIG_PATH):
        self._path = Path(path)
        self._lock = threading.Lock()
        self._mtime = 0.0
        self._data = dict(DEFAULTS)
        self.reload()

    def reload(self):
        with self._lock:
            try:
                mtime = self._path.stat().st_mtime
            except FileNotFoundError:
                return
            if mtime == self._mtime:
                return
            try:
                raw = json.loads(self._path.read_text(encoding="utf-8"))
            except (json.JSONDecodeError, OSError):
                return
            self._data = {**DEFAULTS, **raw}
            self._mtime = mtime

    def get(self, key):
        self.reload()
        with self._lock:
            return self._data.get(key, DEFAULTS.get(key))

    def snapshot(self):
        self.reload()
        with self._lock:
            return dict(self._data)

    def update(self, **kwargs):
        with self._lock:
            self._data.update(kwargs)
            self._path.write_text(
                json.dumps(self._data, ensure_ascii=False, indent=2),
                encoding="utf-8",
            )
            self._mtime = self._path.stat().st_mtime

    @property
    def configured(self):
        s = self.snapshot()
        return bool(s["s21_username"] and s["s21_password"] and s["tg_bot_token"])


class State:
    """state.json: seen-списки, снапшоты броней, флаги напоминаний."""

    def __init__(self, path=STATE_PATH):
        self._path = Path(path)
        self._lock = threading.Lock()
        try:
            self.data = json.loads(self._path.read_text(encoding="utf-8"))
        except (FileNotFoundError, json.JSONDecodeError):
            self.data = {}
        # ключи из v1, больше не используются
        self.data.pop("access_token", None)
        self.data.pop("context_headers", None)

    def save(self):
        with self._lock:
            self._path.write_text(
                json.dumps(self.data, ensure_ascii=False), encoding="utf-8"
            )

    @property
    def is_cold_start(self):
        return not self._path.exists()
