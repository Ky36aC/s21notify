#!/usr/bin/env sh
# Одноразовая настройка доски-роадмапа в GitHub Projects из issue репозитория.
#
# gh нужен scope 'project' (у обычного токена его нет) — выдать однократно:
#   gh auth refresh -s project
# затем:
#   sh scripts/setup-github-project.sh
#
# Скрипт создаёт проект (Projects v2) у пользователя и добавляет туда все
# открытые issue. Дальше в вебе переключи вид на Board/Roadmap и расставь статусы.
set -e

OWNER="Ky36aC"
REPO="Ky36aC/s21notify"
TITLE="s21notify — дорожная карта"

echo "==> Создаю проект «$TITLE»"
NUM=$(gh project create --owner "$OWNER" --title "$TITLE" --format json --jq '.number')
echo "    проект #$NUM"

# по умолчанию проект приватный и не виден в репо — делаем публичным и линкуем,
# иначе «пропадает»: дорожная карта должна быть видна в репозитории и другим
echo "==> Публичный + привязка к репозиторию"
gh project edit "$NUM" --owner "$OWNER" --visibility PUBLIC
gh project link "$NUM" --owner "$OWNER" --repo "$REPO"

echo "==> Добавляю открытые issue репозитория"
gh issue list --repo "$REPO" --state open --json url --jq '.[].url' | while read -r URL; do
    gh project item-add "$NUM" --owner "$OWNER" --url "$URL"
    echo "    + $URL"
done

echo "==> Готово. Открываю проект в браузере — переключи вид на Board/Roadmap."
gh project view "$NUM" --owner "$OWNER" --web
