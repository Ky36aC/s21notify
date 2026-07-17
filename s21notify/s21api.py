# -*- coding: utf-8 -*-
"""Клиент платформы: логин через форму Keycloak + GraphQL-запросы.

Механика логина повторяет официальный веб-клиент (client_id=school21,
grant authorization_code); подсмотрена в https://github.com/s21toolkit/s21auth (MIT).
"""

import base64
import html
import json
import logging
import re
import threading
import time
import uuid

import requests

log = logging.getLogger("s21api")

KC_BASE = "https://auth.21-school.ru/auth/realms/EduPowerKeycloak/protocol/openid-connect"
REDIRECT_URI = "https://platform.21-school.ru/"
REST_API = "https://platform.21-school.ru/services/rest"
GRAPHQL_API = "https://platform.21-school.ru/services/graphql"


class AuthError(Exception):
    pass


def _token_valid(token, margin=60):
    try:
        payload = json.loads(base64.urlsafe_b64decode(token.split(".")[1] + "=="))
        return payload["exp"] - time.time() > margin
    except Exception:
        return False


def kc_login(session, username, password):
    """Возвращает access_token или бросает AuthError."""
    auth_url = (
        f"{KC_BASE}/auth?client_id=school21&response_mode=fragment"
        f"&response_type=code&scope=openid"
        f"&state={uuid.uuid4()}&nonce={uuid.uuid4()}&redirect_uri={REDIRECT_URI}"
    )
    r = session.get(auth_url, timeout=30)
    r.raise_for_status()

    m = re.search(r'window\.loginAction\s*=\s*"(https://[^"]+)"', r.text) \
        or re.search(r'action="(https://[^"]+)"', r.text)
    if not m:
        raise AuthError("Не нашёл форму логина Keycloak (изменилась разметка платформы?)")
    action_url = html.unescape(m.group(1))

    r = session.post(
        action_url,
        data={"username": username, "password": password},
        allow_redirects=False,
        timeout=30,
    )
    location = r.headers.get("Location", "")
    hops = 0
    while "code=" not in location:
        if r.status_code != 302 or not location or hops >= 5:
            raise AuthError("Неверный логин или пароль платформы")
        r = session.get(location, allow_redirects=False, timeout=30)
        location = r.headers.get("Location", "")
        hops += 1

    code = re.search(r"code=([^&#]+)", location).group(1)

    r = session.post(
        f"{KC_BASE}/token",
        data={
            "grant_type": "authorization_code",
            "client_id": "school21",
            "code": code,
            "redirect_uri": REDIRECT_URI,
        },
        timeout=30,
    )
    tok = r.json()
    if "access_token" not in tok:
        raise AuthError(f"Token endpoint вернул ошибку: {tok}")
    return tok["access_token"]


def test_credentials(username, password):
    """Проверка логина/пароля для веб-интерфейса. Возвращает (ok, сообщение)."""
    try:
        kc_login(requests.Session(), username, password)
        return True, "Вход на платформу выполнен успешно"
    except AuthError as e:
        return False, str(e)
    except requests.RequestException as e:
        return False, f"Сетевая ошибка: {e}"


class S21Api:
    """Потокобезопасный клиент: сам логинится и обновляет токен по истечении."""

    def __init__(self, config):
        self._config = config
        self._lock = threading.Lock()
        self._session = requests.Session()
        self._token = None
        self._ctx_headers = None

    def _headers(self):
        with self._lock:
            if not self._token or not _token_valid(self._token):
                cfg = self._config.snapshot()
                log.info("получаю новый токен платформы...")
                self._token = kc_login(
                    self._session, cfg["s21_username"], cfg["s21_password"]
                )
                self._ctx_headers = None
            headers = {"Authorization": f"Bearer {self._token}"}
            if not self._ctx_headers:
                r = self._session.get(
                    f"{REST_API}/edu-context/context-info", headers=headers, timeout=30
                )
                r.raise_for_status()
                self._ctx_headers = r.json()["data"]["contextHeaders"]
            headers.update(self._ctx_headers)
            return headers

    def invalidate(self):
        with self._lock:
            self._token = None
            self._ctx_headers = None

    def gql(self, op_name, query, variables=None):
        r = self._session.post(
            GRAPHQL_API,
            headers=self._headers(),
            json={"operationName": op_name, "query": query, "variables": variables or {}},
            timeout=30,
        )
        if r.status_code in (401, 403):
            self.invalidate()
        if not r.ok:
            reason = r.headers.get("x-bad-request", r.text[:500])
            raise RuntimeError(f"GraphQL HTTP {r.status_code} [{op_name}]: {reason}")
        data = r.json()
        if data.get("errors"):
            raise RuntimeError(f"GraphQL error [{op_name}]: {data['errors']}")
        return data["data"]
