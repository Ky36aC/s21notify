# -*- coding: utf-8 -*-
"""Точка входа: watcher + telegram-бот + веб-интерфейс в одном процессе."""

import logging
import sys

from . import __version__
from .bot import Bot
from .config import Config, State
from .s21api import S21Api
from .watcher import Alarm, Journal, Watcher
from .web import create_app


def main():
    for stream in (sys.stdout, sys.stderr):
        if stream and hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace")
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(name)-8s %(levelname)-7s %(message)s",
        datefmt="%d.%m %H:%M:%S",
    )
    logging.getLogger("werkzeug").setLevel(logging.WARNING)
    log = logging.getLogger("main")

    config = Config()
    state = State()
    journal = Journal()
    api = S21Api(config)

    bot = Bot(config, api, journal, state)
    watcher = Watcher(config, state, api, journal, send_fn=bot.send_to_user)
    alarm = Alarm(config, state, journal, send_fn=bot.send_to_user)
    bot.watcher = watcher

    watcher.start()
    bot.start()
    alarm.start()

    host, port = config.get("web_host"), int(config.get("web_port"))
    log.info("s21notify v%s — веб-интерфейс на http://%s:%s", __version__, host, port)
    if not config.configured:
        log.info("настройки не заполнены — открой веб-интерфейс и заполни форму")

    app = create_app(config, journal, api, __version__)
    app.run(host=host, port=port, threaded=True)


if __name__ == "__main__":
    main()
