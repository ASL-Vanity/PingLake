[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ControlPath,
    [Parameter(Mandatory)][string]$Password
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$resolvedControl = (Resolve-Path -LiteralPath $ControlPath).Path
if (-not $resolvedControl.StartsWith((Join-Path $repoRoot 'data\e2e-'), [StringComparison]::OrdinalIgnoreCase) -or (Split-Path -Leaf $resolvedControl) -ne 'control.json') {
    throw 'Only a project-local e2e control file can be used.'
}
if ([string]::IsNullOrWhiteSpace($Password)) { throw 'Password must not be empty.' }
$control = Get-Content -LiteralPath $resolvedControl | ConvertFrom-Json
$uri = [Uri]$control.base_url
if ($uri.Scheme -ne 'http' -or $uri.Host -ne '127.0.0.1') { throw 'Only the loopback HTTP test Hub can be changed.' }
$binary = Join-Path $repoRoot 'target\debug\pinglake-hub.exe'
$existing = Get-Process -Id $control.hub_pid -ErrorAction SilentlyContinue
if ($existing -and $existing.Path -ne $binary) { throw 'Recorded PID is not the project test Hub.' }
$listener = Get-NetTCPConnection -LocalPort $uri.Port -State Listen -ErrorAction SilentlyContinue
if ($listener -and (!$existing -or $listener.LocalAddress -ne '127.0.0.1' -or $listener.OwningProcess -ne $existing.Id)) { throw 'Listener is not the recorded loopback test Hub.' }
$validationSocket = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$validationSocket.Start()
$validationPort = $validationSocket.LocalEndpoint.Port
$validationSocket.Stop()
$previousPassword = $control.admin_password
$candidate = $null
$replacement = $null
$originalStopped = $false

function Start-TestHub {
    param([int]$Port, [string]$LoginPassword, [string]$LogName)
    $environment = @{
        PINGLAKE_BIND = "127.0.0.1:$Port"
        PINGLAKE_DATABASE = Join-Path $control.run_root 'pinglake.db'
        PINGLAKE_ADMIN_PASSWORD = $LoginPassword
        PINGLAKE_ALLOW_WEAK_ADMIN_PASSWORD = 'true'
        PINGLAKE_ENROLLMENT_TOKEN = $control.enrollment_token
        PINGLAKE_COOKIE_SECURE = 'false'
        RUST_LOG = 'warn'
    }
    Start-Process -FilePath $binary -WorkingDirectory $repoRoot -WindowStyle Hidden -PassThru -Environment $environment `
        -RedirectStandardOutput (Join-Path $control.run_root "$LogName.out.log") `
        -RedirectStandardError (Join-Path $control.run_root "$LogName.err.log")
}

function Verify-Login {
    param([int]$Port, [string]$LoginPassword)
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        try {
            $health = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/healthz" -TimeoutSec 2
            if ($health.status -eq 'ok') { break }
        } catch {}
        Start-Sleep -Milliseconds 200
    } while ([DateTime]::UtcNow -lt $deadline)
    $session = [Microsoft.PowerShell.Commands.WebRequestSession]::new()
    $login = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/v1/auth/login" -Method Post -TimeoutSec 5 -WebSession $session `
        -ContentType 'application/json' -Body (@{ password = $LoginPassword } | ConvertTo-Json)
    if (-not $login.authenticated) { throw 'Test Hub did not accept the requested password.' }
    Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/v1/summary" -TimeoutSec 5 -WebSession $session
}

try {
    $candidate = Start-TestHub -Port $validationPort -LoginPassword $Password -LogName 'password-validation'
    Verify-Login -Port $validationPort -LoginPassword $Password | Out-Null
    Stop-Process -Id $candidate.Id -ErrorAction Stop
    $candidate.WaitForExit(5000) | Out-Null
    $candidate = $null
    if ($existing) { Stop-Process -Id $existing.Id -ErrorAction Stop; $existing.WaitForExit(5000) | Out-Null; $originalStopped = $true }
    $replacement = Start-TestHub -Port $uri.Port -LoginPassword $Password -LogName 'hub-password-updated'
    $summary = Verify-Login -Port $uri.Port -LoginPassword $Password
    $control.admin_password = $Password
    $control.hub_pid = $replacement.Id
    [IO.File]::WriteAllText($resolvedControl, ($control | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
    [pscustomobject]@{ status = 'passed'; base_url = $control.base_url; login_verified = $true; total_nodes = $summary.total_nodes; hub_pid = $replacement.Id } | ConvertTo-Json -Compress
} catch {
    if ($candidate) { Stop-Process -Id $candidate.Id -ErrorAction SilentlyContinue }
    if ($originalStopped) {
        if ($replacement) { Stop-Process -Id $replacement.Id -ErrorAction SilentlyContinue }
        $rollback = Start-TestHub -Port $uri.Port -LoginPassword $previousPassword -LogName 'hub-password-rollback'
        Verify-Login -Port $uri.Port -LoginPassword $previousPassword | Out-Null
        $control.hub_pid = $rollback.Id
        $control.admin_password = $previousPassword
        [IO.File]::WriteAllText($resolvedControl, ($control | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
    }
    throw
}
