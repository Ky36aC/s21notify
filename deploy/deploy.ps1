# Деплой на LXC с машины разработчика (Windows/PowerShell).
# Компиляция на LXC — никогда, там 512 МБ; на сервер едет готовый артефакт из CI.
# Использование:
#   $env:DEPLOY_HOST = "root@адрес-твоего-сервера"   # обязательно
#   $env:DEPLOY_DIR  = "/opt/s21notify"              # необязательно (дефолт ниже)
#   ./deploy/deploy.ps1            # последний успешный build ветки main
#   ./deploy/deploy.ps1 <run_id>   # конкретный запуск CI
#
# Требуется: gh (авторизован), ssh/scp (OpenSSH-клиент Windows).
[CmdletBinding()]
param([string]$RunId)

$ErrorActionPreference = 'Stop'

$HostTarget = $env:DEPLOY_HOST
if (-not $HostTarget) {
    throw 'Задай DEPLOY_HOST, напр.: $env:DEPLOY_HOST = "root@my-server"'
}
$Dir = if ($env:DEPLOY_DIR) { $env:DEPLOY_DIR } else { '/opt/s21notify' }
$Artifact = 's21notify'

# Общие опции ssh/scp: без интерактива (иначе скрипт зависает).
#   -n / BatchMode      — ssh не читает stdin и не спрашивает пароль/подтверждения
#   StrictHostKeyChecking=accept-new — первый ключ хоста принимаем без вопроса
#   ConnectTimeout      — падать быстро, а не висеть на недоступном хосте
$Opts = @('-o', 'BatchMode=yes', '-o', 'ConnectTimeout=15', '-o', 'StrictHostKeyChecking=accept-new')

# ssh с -n (не наследует stdin — главная причина «зависаний» в скриптах)
function Invoke-Ssh([string]$Cmd) {
    ssh -n @Opts $HostTarget $Cmd
}

$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("s21deploy_" + [System.Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
    if (-not $RunId) {
        $RunId = (gh run list --workflow build.yml --branch main --status success --limit 1 `
            --json databaseId --jq '.[0].databaseId').Trim()
        if (-not $RunId) { throw 'Не нашёл успешный build на ветке main' }
    }
    Write-Host "==> Скачиваю артефакт из run $RunId"
    gh run download $RunId -n $Artifact -D $Tmp
    if ($LASTEXITCODE -ne 0) { throw "gh run download завершился с кодом $LASTEXITCODE" }

    Write-Host "==> Раскладываю на ${HostTarget}:$Dir"
    Invoke-Ssh "mkdir -p $Dir/data $Dir/static"
    if ($LASTEXITCODE -ne 0) { throw "ssh mkdir не прошёл (код $LASTEXITCODE) — проверь доступ по ключу к $HostTarget" }
    scp @Opts (Join-Path $Tmp 's21-server') "${HostTarget}:$Dir/s21-server.new"
    if ($LASTEXITCODE -ne 0) { throw "scp бинаря не прошёл (код $LASTEXITCODE)" }
    # содержимое static/ поэлементно (Windows-scp не разворачивает glob static/*)
    Get-ChildItem (Join-Path $Tmp 'static') | ForEach-Object {
        scp -r @Opts $_.FullName "${HostTarget}:$Dir/static/"
        if ($LASTEXITCODE -ne 0) { throw "scp static/$($_.Name) не прошёл (код $LASTEXITCODE)" }
    }
    scp @Opts (Join-Path $Tmp 's21notify.service') "${HostTarget}:/etc/systemd/system/s21notify.service"
    if ($LASTEXITCODE -ne 0) { throw "scp юнита не прошёл (код $LASTEXITCODE)" }
    Invoke-Ssh "chmod +x $Dir/s21-server.new && mv -f $Dir/s21-server.new $Dir/s21-server"
    if ($LASTEXITCODE -ne 0) { throw "активация бинаря не прошла (код $LASTEXITCODE)" }

    # проверяем наличие .env на сервере (ssh test даёт код возврата)
    Invoke-Ssh "test -f $Dir/.env"
    if ($LASTEXITCODE -ne 0) {
        scp @Opts (Join-Path $Tmp 'env.example') "${HostTarget}:$Dir/env.example"
        Write-Host '!! На LXC нет .env — заполни из env.example (chmod 600) и запусти снова'
        exit 1
    }

    Write-Host '==> Рестарт'
    Invoke-Ssh "systemctl daemon-reload && systemctl enable --now s21notify && systemctl restart s21notify && sleep 3 && systemctl is-active s21notify"
    if ($LASTEXITCODE -ne 0) { throw "сервис не поднялся после рестарта (код $LASTEXITCODE) — смотри journalctl -u s21notify" }
    Invoke-Ssh "python3 -c `"import urllib.request,sys; r=urllib.request.urlopen('http://127.0.0.1:80/healthz', timeout=10); sys.stdout.write(r.read().decode())`""
    Write-Host ''
    Write-Host '==> Готово'
}
finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
