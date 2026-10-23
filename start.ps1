#Requires -Version 5.1
<#
.SYNOPSIS
    问序（Wenxu）本地开发一键启动脚本。

.DESCRIPTION
    在仓库根目录 .pgdata/ 下维护一个测试专用的 PostgreSQL 18 实例
    （127.0.0.1:55432，用户 questionnaire，数据库 questionnaire_test，trust 认证），
    自动准备 .env、执行数据库迁移并幂等创建默认管理员，然后并行启动前端与后端。

.PARAMETER ResetDb
    停止并删除 .pgdata/，重新初始化一个干净的测试数据库。

.PARAMETER Stop
    停止本脚本管理的测试数据库后退出（与 -ResetDb 同时使用时只执行停止）。

.PARAMETER DbOnly
    仅确保测试数据库启动、完成迁移并初始化默认管理员后退出，不启动前后端服务。

.EXAMPLE
    .\start.ps1

.EXAMPLE
    .\start.ps1 -ResetDb

.EXAMPLE
    .\start.ps1 -Stop

.EXAMPLE
    .\start.ps1 -DbOnly
#>
[CmdletBinding()]
param(
    [switch]$ResetDb,
    [switch]$Stop,
    [switch]$DbOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# 常量：测试数据库与默认管理员
# ---------------------------------------------------------------------------
$Root = if ($PSScriptRoot) { $PSScriptRoot } else { (Get-Location).Path }
$PgData = Join-Path $Root '.pgdata'
$PgLog = Join-Path $PgData 'postgres.log'
$PgHost = '127.0.0.1'
$PgPort = 55432
$PgUser = 'questionnaire'
$PgDatabase = 'questionnaire_test'
$DefaultAdminUsername = 'admin'
$DefaultAdminPassword = 'admin123456'

# ---------------------------------------------------------------------------
# 输出与工具检查
# ---------------------------------------------------------------------------
function Write-Step {
    param([string]$Message)
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Write-Ok {
    param([string]$Message)
    Write-Host "    $Message" -ForegroundColor Green
}

function Write-Note {
    param([string]$Message)
    Write-Host "    $Message" -ForegroundColor Yellow
}

function Assert-Tool {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Hint
    )
    if ($null -eq (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "未找到命令 '$Name'。$Hint"
    }
}

# ---------------------------------------------------------------------------
# PostgreSQL 辅助函数
# ---------------------------------------------------------------------------
function Test-PgReady {
    & pg_isready -h $PgHost -p $PgPort -q 2>$null | Out-Null
    return $LASTEXITCODE -eq 0
}

function Wait-PgReady {
    param(
        [int]$Attempts = 60,
        [int]$DelayMilliseconds = 500
    )
    for ($index = 0; $index -lt $Attempts; $index++) {
        if (Test-PgReady) {
            return $true
        }
        Start-Sleep -Milliseconds $DelayMilliseconds
    }
    return $false
}

function Initialize-EnvFile {
    $envPath = Join-Path $Root '.env'
    if (Test-Path -LiteralPath $envPath) {
        Write-Ok '检测到已有 .env，保持原样。'
        return
    }

    $lines = @(
        'DATABASE_URL=postgres://questionnaire@127.0.0.1:55432/questionnaire_test',
        'LISTEN_ADDR=127.0.0.1:3000',
        'PUBLIC_ORIGIN=http://localhost:5173',
        'COOKIE_SECURE=false',
        'SESSION_TTL_SECONDS=86400',
        'AUTH_REQUESTS_PER_MINUTE=60',
        'SUBMISSIONS_PER_MINUTE=600',
        'RUST_LOG=questionnaire_api=info'
    )
    $content = ($lines -join [Environment]::NewLine) + [Environment]::NewLine
    # 使用无 BOM 的 UTF-8，避免 dotenvy 把 BOM 带进第一个键名。
    [System.IO.File]::WriteAllText($envPath, $content, [System.Text.UTF8Encoding]::new($false))
    Write-Step '已生成测试用 .env（连接串指向 127.0.0.1:55432/questionnaire_test）。'
}

function Stop-TestDatabase {
    if (-not (Test-Path -LiteralPath (Join-Path $PgData 'PG_VERSION'))) {
        Write-Note '尚未初始化 .pgdata，无需停止测试数据库。'
        return
    }

    Assert-Tool 'pg_ctl' '请安装 PostgreSQL 18，并将其 bin 目录加入 PATH。'
    Assert-Tool 'pg_isready' '请安装 PostgreSQL 18，并将其 bin 目录加入 PATH。'

    if (-not (Test-PgReady)) {
        Write-Note '测试数据库当前未运行。'
        return
    }

    Write-Step '停止测试数据库...'
    & pg_ctl -D $PgData -m fast -w stop
    if ($LASTEXITCODE -ne 0) {
        throw "pg_ctl 停止测试数据库失败（退出码 $LASTEXITCODE），请查看 $PgLog。"
    }
    Write-Ok '测试数据库已停止。'
}

function Reset-TestDatabase {
    if (Test-Path -LiteralPath $PgData) {
        Stop-TestDatabase
        Write-Step '删除旧的测试数据目录 .pgdata...'
        Remove-Item -LiteralPath $PgData -Recurse -Force
        Write-Ok '.pgdata 已删除，将重新初始化干净实例。'
    }
    else {
        Write-Note '未发现 .pgdata，直接初始化干净实例。'
    }
}

function Start-TestDatabase {
    $initialized = Test-Path -LiteralPath (Join-Path $PgData 'PG_VERSION')
    if (-not $initialized) {
        Assert-Tool 'initdb' '请安装 PostgreSQL 18，并将其 bin 目录加入 PATH。'
        Write-Step '初始化测试专用 PostgreSQL 实例（.pgdata，trust 认证）...'
        $null = New-Item -ItemType Directory -Force -Path $PgData
        & initdb -D $PgData -U $PgUser -A trust -E UTF8 --no-locale
        if ($LASTEXITCODE -ne 0) {
            throw "initdb 初始化失败（退出码 $LASTEXITCODE）。"
        }
        Write-Ok '测试数据库实例初始化完成。'
    }

    if (Test-PgReady) {
        Write-Ok "测试数据库已在 ${PgHost}:${PgPort} 运行。"
        return
    }

    Write-Step "启动测试数据库（监听 ${PgHost}:${PgPort}）..."
    & pg_ctl -D $PgData -l $PgLog -o "-h $PgHost -p $PgPort" -w start
    $startExitCode = $LASTEXITCODE
    if (-not (Wait-PgReady)) {
        throw "启动测试数据库后 30 秒内未就绪（pg_ctl 退出码 $startExitCode），请查看日志：$PgLog"
    }
    Write-Ok '测试数据库已启动。'
}

function Ensure-TestDatabase {
    Assert-Tool 'psql' '请安装 PostgreSQL 18，并将其 bin 目录加入 PATH。'
    Assert-Tool 'createdb' '请安装 PostgreSQL 18，并将其 bin 目录加入 PATH。'

    $existsSql = "SELECT 1 FROM pg_database WHERE datname = '$PgDatabase';"
    $exists = (& psql -h $PgHost -p $PgPort -U $PgUser -d postgres -tAc $existsSql | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "查询测试数据库列表失败（psql 退出码 $LASTEXITCODE）。"
    }

    if ($exists -ne '1') {
        Write-Step "创建数据库 $PgDatabase..."
        & createdb -h $PgHost -p $PgPort -U $PgUser --owner=$PgUser --encoding=UTF8 --template=template0 --locale=C $PgDatabase
        if ($LASTEXITCODE -ne 0) {
            throw "createdb 创建数据库失败（退出码 $LASTEXITCODE）。"
        }
        Write-Ok "数据库 $PgDatabase 已创建。"
    }
}

function Test-AdminExists {
    $tableSql = "SELECT to_regclass('public.users') IS NOT NULL;"
    $tableExists = (& psql -h $PgHost -p $PgPort -U $PgUser -d $PgDatabase -tAc $tableSql | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "查询测试数据库结构失败（psql 退出码 $LASTEXITCODE）。"
    }
    if ($tableExists -ne 't') {
        return $false
    }

    $adminSql = "SELECT 1 FROM users WHERE username = '$DefaultAdminUsername' AND role = 'admin' LIMIT 1;"
    $adminExists = (& psql -h $PgHost -p $PgPort -U $PgUser -d $PgDatabase -tAc $adminSql | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "查询默认管理员失败（psql 退出码 $LASTEXITCODE）。"
    }
    return ($adminExists -eq '1')
}

function Invoke-DatabaseBootstrap {
    Assert-Tool 'cargo' '请安装 Rust stable 工具链，并将 cargo 加入 PATH。'

    # create-admin 子命令内部也会执行 Migrator::up，首次运行可同时完成建表与管理员创建。
    if (-not (Test-AdminExists)) {
        Write-Step "创建默认管理员 '$DefaultAdminUsername'（首次运行会编译后端并执行迁移）..."
        $createExitCode = 1
        $previousUsername = $env:ADMIN_USERNAME
        $previousPassword = $env:ADMIN_PASSWORD
        try {
            $env:ADMIN_USERNAME = $DefaultAdminUsername
            $env:ADMIN_PASSWORD = $DefaultAdminPassword
            & cargo run --locked -p questionnaire-api -- create-admin
            $createExitCode = $LASTEXITCODE
        }
        finally {
            if ($null -eq $previousUsername) {
                Remove-Item Env:ADMIN_USERNAME -ErrorAction SilentlyContinue
            }
            else {
                $env:ADMIN_USERNAME = $previousUsername
            }
            if ($null -eq $previousPassword) {
                Remove-Item Env:ADMIN_PASSWORD -ErrorAction SilentlyContinue
            }
            else {
                $env:ADMIN_PASSWORD = $previousPassword
            }
        }
        if ($createExitCode -ne 0) {
            throw "创建默认管理员失败（cargo 退出码 $createExitCode）。"
        }
        Write-Ok "默认管理员已创建：$DefaultAdminUsername / $DefaultAdminPassword"
        return
    }

    # 管理员已存在时仍单独执行迁移，保证后续新增的 schema 变更被应用（-DbOnly 同样需要）。
    Write-Ok "默认管理员 '$DefaultAdminUsername' 已存在，跳过创建。"
    Write-Step '执行数据库迁移...'
    & cargo run --locked -p questionnaire-api -- migrate
    if ($LASTEXITCODE -ne 0) {
        throw "数据库迁移失败（cargo 退出码 $LASTEXITCODE）。"
    }
    Write-Ok '数据库迁移完成。'
}

# ---------------------------------------------------------------------------
# 前端进程
# ---------------------------------------------------------------------------
function Resolve-Pnpm {
    foreach ($candidate in @('pnpm.cmd', 'pnpm.exe', 'pnpm')) {
        $command = Get-Command $candidate -ErrorAction SilentlyContinue
        if ($null -ne $command) {
            return $command.Source
        }
    }
    throw "未找到 pnpm。请安装 pnpm（Node.js 22.18+ / 24.12+）并将其加入 PATH。"
}

function Start-Frontend {
    $pnpm = Resolve-Pnpm
    Write-Step '启动前端开发服务器（http://localhost:5173）...'
    $process = Start-Process -FilePath $pnpm `
        -ArgumentList @('--dir', 'frontend', 'run', 'dev') `
        -WorkingDirectory $Root `
        -PassThru `
        -NoNewWindow
    Write-Ok "前端进程已启动（PID $($process.Id)）。"
    return $process
}

function Stop-Frontend {
    param([System.Diagnostics.Process]$Process)
    if ($null -eq $Process) {
        return
    }

    try {
        if (-not $Process.HasExited) {
            # pnpm 会派生出 node/vite 子进程，必须按进程树终止。
            & taskkill.exe /PID $Process.Id /T /F 2>$null | Out-Null
            if (-not $Process.HasExited) {
                Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
            }
            Write-Host '前端开发服务器已停止。' -ForegroundColor Yellow
        }
    }
    catch {
        Write-Warning "停止前端进程失败：$($_.Exception.Message)"
    }
}

# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------
Push-Location -LiteralPath $Root
$frontendProcess = $null
try {
    if ($Stop) {
        Stop-TestDatabase
        return
    }

    if ($ResetDb) {
        Reset-TestDatabase
    }

    Initialize-EnvFile

    Assert-Tool 'pg_ctl' '请安装 PostgreSQL 18，并将其 bin 目录加入 PATH。'
    Assert-Tool 'pg_isready' '请安装 PostgreSQL 18，并将其 bin 目录加入 PATH。'
    Start-TestDatabase
    Ensure-TestDatabase
    Invoke-DatabaseBootstrap

    if ($DbOnly) {
        Write-Ok "仅数据库模式完成：postgres://$PgUser@$PgHost`:$PgPort/$PgDatabase"
        return
    }

    $frontendProcess = Start-Frontend

    Write-Step '启动后端 API（http://127.0.0.1:3000）...'
    Write-Note '按 Ctrl+C 停止后端；脚本会自动关闭前端开发服务器。'
    & cargo run --locked -p questionnaire-api
    $backendExitCode = $LASTEXITCODE
    if ($backendExitCode -ne 0) {
        Write-Warning "后端进程已退出（退出码 $backendExitCode）。"
    }
}
finally {
    Stop-Frontend -Process $frontendProcess
    Pop-Location
}
