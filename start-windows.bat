@echo off
rem s21notify: первый запуск создаст окружение, дальше просто стартует в фоне.
cd /d "%~dp0"

if not exist ".venv\Scripts\python.exe" (
    echo == Создаю окружение и ставлю зависимости, это одноразово...
    py -3 -m venv .venv || python -m venv .venv
    ".venv\Scripts\python.exe" -m pip install -q -r requirements.txt
)

echo == Запускаю s21notify в фоне и открываю настройки...
start "" ".venv\Scripts\pythonw.exe" run.py
timeout /t 2 /nobreak >nul
start "" http://localhost:8021
