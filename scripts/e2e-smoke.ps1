[CmdletBinding()]
param(
    [int]$Port = 18090,
    [switch]$LeaveRunning
)

$ErrorActionPreference = 'Stop'

function New-Secret {
    $bytes = New-Object byte[] 32
    $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $rng.GetBytes($bytes)
    } finally {
        $rng.Dispose()
    }
    [Convert]::ToBase64String($bytes)
}

function Wait-ForHub {
    param([string]$BaseUrl)

    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        try {
            $health = Invoke-RestMethod -Uri "$BaseUrl/api/healthz" -TimeoutSec 2
            if ($health.status -eq 'ok') {
                return
            }
        } catch {
            Start-Sleep -Milliseconds 250
        }
    } while ([DateTime]::UtcNow -lt $deadline)

    throw 'PingLake Hub did not become ready within 15 seconds.'
}

function Stop-IfRunning {
    param([int]$Id)

    $process = Get-Process -Id $Id -ErrorAction SilentlyContinue
    if ($process) {
        Stop-Process -Id $Id -Force -ErrorAction Stop
    }
}

function Invoke-HubJson {
    param(
        [Parameter(Mandatory = $true)]
        [string]$BaseUrl,

        [Parameter(Mandatory = $true)]
        [string]$CookieJar,

        [Parameter(Mandatory = $true)]
        [ValidateSet('GET', 'POST', 'PUT')]
        [string]$Method,

        [Parameter(Mandatory = $true)]
        [string]$Path,

        [string]$BodyPath
    )

    $arguments = @(
        '--silent',
        '--show-error',
        '--fail-with-body',
        '--max-time', '10',
        '--noproxy', '*',
        '--cookie', $CookieJar,
        '--cookie-jar', $CookieJar,
        '--request', $Method
    )
    if ($BodyPath) {
        $arguments += @('--header', 'Content-Type: application/json', '--data-binary', "@$BodyPath")
    }
    $arguments += "$BaseUrl$Path"
    $response = & curl.exe @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "Hub request $Method $Path failed with curl exit code $LASTEXITCODE."
    }
    if ([string]::IsNullOrWhiteSpace($response)) {
        return $null
    }
    $parsed = $response | ConvertFrom-Json
    Write-Output $parsed
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$runRoot = Join-Path $repoRoot ("data\e2e-" + [Guid]::NewGuid().ToString('N'))
$hubBinary = Join-Path $repoRoot 'target\debug\pinglake-hub.exe'
$agentBinary = Join-Path $repoRoot 'target\debug\pinglake-agent.exe'
$baseUrl = "http://127.0.0.1:$Port"
$adminPassword = New-Secret
$enrollmentToken = New-Secret
$hubProcess = $null
$agentProcesses = @()

if (-not (Test-Path -LiteralPath $hubBinary -PathType Leaf)) {
    throw "Hub binary is missing: $hubBinary"
}
if (-not (Test-Path -LiteralPath $agentBinary -PathType Leaf)) {
    throw "Agent binary is missing: $agentBinary"
}

New-Item -ItemType Directory -Force -Path $runRoot | Out-Null
$savedEnvironment = @{}
foreach ($name in @(
    'PINGLAKE_BIND',
    'PINGLAKE_DATABASE',
    'PINGLAKE_ADMIN_PASSWORD',
    'PINGLAKE_ENROLLMENT_TOKEN',
    'PINGLAKE_COOKIE_SECURE',
    'RUST_LOG'
)) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

try {
    $env:PINGLAKE_BIND = "127.0.0.1:$Port"
    $env:PINGLAKE_DATABASE = Join-Path $runRoot 'pinglake.db'
    $env:PINGLAKE_ADMIN_PASSWORD = $adminPassword
    $env:PINGLAKE_ENROLLMENT_TOKEN = $enrollmentToken
    $env:PINGLAKE_COOKIE_SECURE = 'false'
    $env:RUST_LOG = 'warn'
    $hubProcess = Start-Process -FilePath $hubBinary -WorkingDirectory $repoRoot -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $runRoot 'hub.out.log') `
        -RedirectStandardError (Join-Path $runRoot 'hub.err.log')
    Wait-ForHub -BaseUrl $baseUrl

    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }

    for ($index = 1; $index -le 7; $index++) {
        $stateDirectory = Join-Path $runRoot "agent-state-$index"
        $configPath = Join-Path $runRoot "agent-$index.json"
        New-Item -ItemType Directory -Force -Path $stateDirectory | Out-Null
        $config = [ordered]@{
            hub_url = $baseUrl
            enrollment_token = $enrollmentToken
            name = "e2e-node-$index"
            interval_secs = 5
            state_dir = $stateDirectory
            insecure_skip_verify = $false
            allow_insecure_http = $false
        }
        [IO.File]::WriteAllText(
            $configPath,
            ($config | ConvertTo-Json),
            [Text.UTF8Encoding]::new($false)
        )
        $agentProcesses += Start-Process -FilePath $agentBinary -WorkingDirectory $repoRoot -WindowStyle Hidden -PassThru `
            -ArgumentList @('--config', ('"{0}"' -f $configPath)) `
            -RedirectStandardOutput (Join-Path $runRoot "agent-$index.out.log") `
            -RedirectStandardError (Join-Path $runRoot "agent-$index.err.log")
    }

    $control = [ordered]@{
        base_url = $baseUrl
        run_root = $runRoot
        admin_password = $adminPassword
        enrollment_token = $enrollmentToken
        hub_pid = $hubProcess.Id
        agent_pids = @($agentProcesses | ForEach-Object Id)
    }
    [IO.File]::WriteAllText(
        (Join-Path $runRoot 'control.json'),
        ($control | ConvertTo-Json),
        [Text.UTF8Encoding]::new($false)
    )

    Start-Sleep -Seconds 12
    $cookieJar = Join-Path $runRoot 'cookies.txt'
    $loginBodyPath = Join-Path $runRoot 'login.json'
    [IO.File]::WriteAllText($loginBodyPath, (@{ password = $adminPassword } | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    $login = Invoke-HubJson -BaseUrl $baseUrl -CookieJar $cookieJar -Method POST -Path '/api/v1/auth/login' -BodyPath $loginBodyPath
    if (-not $login.authenticated) {
        throw 'Hub login response did not authenticate the test session.'
    }
    $nodes = @(Invoke-HubJson -BaseUrl $baseUrl -CookieJar $cookieJar -Method GET -Path '/api/v1/nodes')
    $summary = Invoke-HubJson -BaseUrl $baseUrl -CookieJar $cookieJar -Method GET -Path '/api/v1/summary'
    if ($nodes.Count -ne 7 -or @($nodes | Where-Object online).Count -ne 7) {
        throw "Expected seven online nodes, got $($nodes.Count) nodes and $(@($nodes | Where-Object online).Count) online."
    }
    $history = @(Invoke-HubJson -BaseUrl $baseUrl -CookieJar $cookieJar -Method GET -Path "/api/v1/nodes/$($nodes[0].id)/history?minutes=60")
    if ($history.Count -eq 0) {
        throw 'Expected persisted history for the first agent.'
    }

    if ($LeaveRunning) {
        [pscustomobject]@{
            status = 'running'
            base_url = $baseUrl
            run_root = $runRoot
            registered_nodes = $nodes.Count
            online_nodes = $summary.online_nodes
            history_points = $history.Count
        } | ConvertTo-Json -Compress
        $hubProcess = $null
        $agentProcesses = @()
        return
    }

    $settings = [ordered]@{
        cpu_percent = 85
        memory_percent = 90
        disk_percent = 85
        temperature_celsius = 85
        offline_after_seconds = 5
        sustained_for_seconds = 0
        webhook_enabled = $false
        webhook_url = ''
    }
    $settingsBodyPath = Join-Path $runRoot 'settings.json'
    [IO.File]::WriteAllText($settingsBodyPath, ($settings | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    Invoke-HubJson -BaseUrl $baseUrl -CookieJar $cookieJar -Method PUT -Path '/api/v1/settings' -BodyPath $settingsBodyPath | Out-Null
    Stop-IfRunning -Id $agentProcesses[0].Id
    $offlineDeadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        Start-Sleep -Seconds 1
        $offlineNodes = @(Invoke-HubJson -BaseUrl $baseUrl -CookieJar $cookieJar -Method GET -Path '/api/v1/nodes' | Where-Object { -not $_.online })
        $alerts = @(Invoke-HubJson -BaseUrl $baseUrl -CookieJar $cookieJar -Method GET -Path '/api/v1/alerts')
        if ($offlineNodes.Count -ge 1 -and @($alerts | Where-Object { $_.kind -eq 'offline' -and $_.active }).Count -ge 1) {
            break
        }
    } while ([DateTime]::UtcNow -lt $offlineDeadline)
    if ($offlineNodes.Count -lt 1 -or @($alerts | Where-Object { $_.kind -eq 'offline' -and $_.active }).Count -lt 1) {
        throw 'Expected an active offline alert after stopping one agent.'
    }

    [pscustomobject]@{
        status = 'passed'
        run_root = $runRoot
        registered_nodes = $nodes.Count
        online_nodes_before_stop = $summary.online_nodes
        history_points = $history.Count
        offline_nodes_after_stop = $offlineNodes.Count
        active_offline_alerts = @($alerts | Where-Object { $_.kind -eq 'offline' -and $_.active }).Count
        login_authenticated = $login.authenticated
    } | ConvertTo-Json -Compress
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }
    if (-not $LeaveRunning) {
        foreach ($agentProcess in $agentProcesses) {
            Stop-IfRunning -Id $agentProcess.Id
        }
        if ($hubProcess) {
            Stop-IfRunning -Id $hubProcess.Id
        }
    }
}
