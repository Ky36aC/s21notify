#!/bin/sh
# Установка s21notify как systemd-сервиса (Debian/Ubuntu, запускать от root).
# Использование: ./install.sh  (из папки проекта, например /opt/s21notify)
set -e

DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

echo "== s21notify: установка в $DIR"

if ! command -v python3 >/dev/null; then
    echo "Нужен python3: apt install -y python3 python3-venv" >&2
    exit 1
fi

python3 -m venv .venv 2>/dev/null || {
    echo "Ставлю python3-venv..."
    apt-get update -qq && apt-get install -y -qq python3-venv
    python3 -m venv .venv
}
.venv/bin/pip install -q -r requirements.txt

sed "s|/opt/s21notify|$DIR|g" s21notify.service > /etc/systemd/system/s21notify.service
systemctl daemon-reload
systemctl enable --now s21notify

IP=$(hostname -I 2>/dev/null | awk '{print $1}')
echo ""
echo "== Готово! Сервис запущен."
echo "   Настройка:  http://${IP:-localhost}:8021"
echo "   Статус:     systemctl status s21notify"
echo "   Логи:       journalctl -u s21notify -f"
