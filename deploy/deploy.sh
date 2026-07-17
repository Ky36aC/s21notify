#!/usr/bin/env bash
# Деплой на LXC с машины разработчика (компиляция на LXC — никогда, там 512 МБ).
# Использование: ./deploy/deploy.sh [run_id]
#   без аргумента — последний успешный run workflow build на ветке main
set -euo pipefail

HOST=root@s21notify.lan
DIR=/opt/s21notify
ARTIFACT=s21notify
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ $# -ge 1 ]; then
    RUN_ID=$1
else
    RUN_ID=$(gh run list --workflow build.yml --branch main --status success --limit 1 \
        --json databaseId --jq '.[0].databaseId')
fi
echo "==> Скачиваю артефакт из run $RUN_ID"
gh run download "$RUN_ID" -n "$ARTIFACT" -D "$TMP"

echo "==> Раскладываю на $HOST:$DIR"
ssh "$HOST" "mkdir -p $DIR/data $DIR/static"
scp "$TMP/s21-server" "$HOST:$DIR/s21-server.new"
scp -r "$TMP"/static/* "$HOST:$DIR/static/"
scp "$TMP/s21notify.service" "$HOST:/etc/systemd/system/s21notify.service"
ssh "$HOST" "chmod +x $DIR/s21-server.new && mv $DIR/s21-server.new $DIR/s21-server"

if ! ssh "$HOST" "test -f $DIR/.env"; then
    scp "$TMP/env.example" "$HOST:$DIR/env.example"
    echo '!! На LXC нет .env — заполни из env.example (chmod 600) и запусти снова'
    exit 1
fi

echo "==> Рестарт"
ssh "$HOST" "systemctl daemon-reload && systemctl enable --now s21notify && systemctl restart s21notify && sleep 3 && systemctl is-active s21notify"
ssh "$HOST" "python3 -c \"import urllib.request,sys; r=urllib.request.urlopen('http://127.0.0.1:80/healthz', timeout=10); sys.stdout.write(r.read().decode())\""
echo
echo "==> Готово"
