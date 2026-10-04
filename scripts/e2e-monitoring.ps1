[CmdletBinding()]
param([Parameter(Mandatory)][string]$RunRoot)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$resolvedRun = (Resolve-Path -LiteralPath $RunRoot).Path
if (-not $resolvedRun.StartsWith((Join-Path $repoRoot 'data\e2e-'), [StringComparison]::OrdinalIgnoreCase)) {
    throw 'RunRoot must be a PingLake project-local e2e directory.'
}
$control = Get-Content -LiteralPath (Join-Path $resolvedRun 'control.json') | ConvertFrom-Json
$session = [Microsoft.PowerShell.Commands.WebRequestSession]::new()
$base = $control.base_url
Invoke-RestMethod -Uri "$base/api/v1/auth/login" -Method Post -Body (@{ password = $control.admin_password } | ConvertTo-Json) -ContentType 'application/json' -WebSession $session | Out-Null

function Hub {
    param([string]$Path, [string]$Method = 'Get', $Body)
    $args = @{ Uri = "$base/api/v1$Path"; Method = $Method; WebSession = $session; TimeoutSec = 10 }
    if ($null -ne $Body) { $args.Body = $Body | ConvertTo-Json -Depth 30; $args.ContentType = 'application/json' }
    $result = Invoke-RestMethod @args
    Write-Output $result
}

$nodes = @(Hub '/nodes')
$node = $nodes | Where-Object display_name -EQ 'e2e-node-1' | Select-Object -First 1
if (-not $node) { throw 'The smoke run did not register e2e-node-1.' }
$configPath = Join-Path $resolvedRun 'agent-1.json'
$agentConfig = Get-Content -LiteralPath $configPath | ConvertFrom-Json
$agentConfig | Add-Member allow_loopback_probe_targets $true -Force
$agentConfig | Add-Member latency_bind '127.0.0.1:18091' -Force
$agentConfig | Add-Member dashboard_origin 'https://monitor.example.com' -Force
$occupied = Get-NetTCPConnection -LocalPort 18091 -State Listen -ErrorAction SilentlyContinue
if ($occupied -and $occupied.OwningProcess -ne $control.agent_pids[0]) { throw 'Test measurement port 18091 is occupied.' }
if ($control.agent_pids.Count) { Stop-Process -Id $control.agent_pids[0] -ErrorAction SilentlyContinue }
[IO.File]::WriteAllText($configPath, ($agentConfig | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
$agent = Start-Process -FilePath (Join-Path $repoRoot 'target\debug\pinglake-agent.exe') -WindowStyle Hidden -PassThru -WorkingDirectory $repoRoot -ArgumentList @('--config', ('"{0}"' -f $configPath)) -RedirectStandardOutput (Join-Path $resolvedRun 'monitoring-agent.out.log') -RedirectStandardError (Join-Path $resolvedRun 'monitoring-agent.err.log')
$control.agent_pids[0] = $agent.Id
[IO.File]::WriteAllText((Join-Path $resolvedRun 'control.json'), ($control | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))

$unused = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$unused.Start()
$closedPort = $unused.LocalEndpoint.Port
$unused.Stop()
$hubPort = ([uri]$base).Port
function Probe {
    param([string]$Name, [string]$Kind, [string]$Target, $Port)
    @{ id = [Guid]::NewGuid().ToString(); name = $Name; kind = $Kind; target = $Target; port = $Port; enabled = $true; interval_secs = 10; timeout_ms = 500; expected_status = $null; response_contains = $null }
}
$checks = @(
    (Probe 'Local ICMP' 'icmp' '127.0.0.1' $null),
    (Probe 'Local TCP' 'tcp' '127.0.0.1' $hubPort),
    (Probe 'Local HTTP' 'http' "$base/api/healthz" $null),
    (Probe 'Closed TCP' 'tcp' '127.0.0.1' $closedPort),
    (Probe 'Metadata policy' 'http' 'http://169.254.169.254/' $null)
)
$dns = Probe 'Local DNS' 'dns' 'localhost' $null
$dns.dns = @{ record_type = 'A'; expected_value = '127.0.0.1' }
$checks += $dns
$checks[2].expected_status = 200
$checks[2].response_contains = 'ok'
$currentConfig = Hub "/nodes/$($node.id)/monitoring"
$configuration = @{ revision = $currentConfig.revision; browser_latency_url = $null; services = @(
    @{ id = [Guid]::NewGuid().ToString(); name = 'EventLog'; enabled = $true; expected_state = 'running' },
    @{ id = [Guid]::NewGuid().ToString(); name = 'PingLakeMissingE2EService'; enabled = $true; expected_state = 'running' }
); probes = $checks; process_checks = @(
    @{ id = [Guid]::NewGuid().ToString(); name = 'PingLake Agent'; process_name = 'pinglake-agent.exe'; expected_count = $null; enabled = $true; expected_state = 'running'; interval_secs = 10; timeout_ms = 500 }
); local_port_checks = @(
    @{ id = [Guid]::NewGuid().ToString(); name = 'Latency endpoint'; address_scope = @{ scope = 'loopback' }; address_family = 'any'; protocol = 'tcp'; port = 18091; enabled = $true; interval_secs = 10; timeout_ms = 500 }
) }
$saved = Hub "/nodes/$($node.id)/monitoring" 'Put' $configuration
$deadline = [DateTime]::UtcNow.AddSeconds(55)
do {
    Start-Sleep -Seconds 1
    $current = @(Hub '/nodes') | Where-Object id -EQ $node.id | Select-Object -First 1
    $data = $current.latest.monitoring
    $results = @($data.probes | Group-Object target_id | ForEach-Object { $_.Group | Sort-Object completed_at | Select-Object -Last 1 })
    $services = @($data.services)
    if ($data.agent.applied_config_revision -eq $saved.revision -and $results.Count -eq 6 -and $services.Count -eq 2 -and @($data.process_checks).Count -eq 1 -and @($data.local_port_checks).Count -eq 1) { break }
} while ([DateTime]::UtcNow -lt $deadline)
if ($results.Count -ne 6 -or $services.Count -ne 2 -or @($data.process_checks).Count -ne 1 -or @($data.local_port_checks).Count -ne 1) { throw 'Monitoring configuration was not applied with complete results.' }
foreach ($probe in $checks) {
    $result = $results | Where-Object target_id -EQ $probe.id
    $expected = if ($probe.name -eq 'Closed TCP') { 'failure' } elseif ($probe.name -eq 'Metadata policy') { 'policy_denied' } else { 'success' }
    if ($result.status -ne $expected -and -not ($probe.name -eq 'Closed TCP' -and $result.status -eq 'timeout')) { throw "Probe '$($probe.name)' expected $expected, got $($result.status)." }
}
if (-not (@($data.process_checks) | Where-Object { $_.process_name -eq 'pinglake-agent.exe' -and $_.healthy })) { throw 'Process check did not observe the running Agent.' }
if (-not (@($data.local_port_checks) | Where-Object { $_.status -in @('unsupported', 'ok') })) { throw 'Local port check did not return an explicit platform result.' }
if (-not ($services | Where-Object { $_.name -eq 'EventLog' -and $_.healthy })) { throw 'SCM service check did not observe EventLog.' }
if (-not ($services | Where-Object { $_.name -eq 'PingLakeMissingE2EService' -and $_.state -eq 'not_found' -and -not $_.healthy })) { throw 'Missing service was not identified.' }
$history = @(Hub "/nodes/$($node.id)/monitoring/history?minutes=60&section=cpu")
if (-not $history.Count -or -not $history[-1].received_at -or -not $history[-1].collected_at) { throw 'Detailed history did not preserve two timestamps.' }
$stats = @(Hub "/nodes/$($node.id)/probes/statistics?minutes=60" | Where-Object { $checks.id -contains $_.target_id })
if (@($stats | Where-Object { $_.successful -gt 0 -and $null -ne $_.p95_ms }).Count -lt 3) { throw 'Successful probe percentile statistics are missing.' }
if (-not ($stats | Where-Object { $_.name -eq 'Metadata policy' -and $null -eq $_.success_rate_percent -and $_.unknown -gt 0 })) { throw 'Policy-denied samples must be unknown, not successful.' }
$dnsStats = @(Hub "/nodes/$($node.id)/checks/statistics?minutes=60&kind=dns")
if (-not ($dnsStats | Where-Object { $_.subject_id -eq $dns.id -and $_.attempts -gt 0 })) { throw 'Canonical DNS check statistics are missing.' }

$group = Hub '/groups' 'Post' @{ name = 'Monitoring E2E disposable group' }
Hub "/nodes/$($node.id)/group" 'Put' @{ group_id = $group.id } | Out-Null
Hub "/groups/$($group.id)" 'Delete' | Out-Null
$after = @(Hub '/nodes') | Where-Object id -EQ $node.id | Select-Object -First 1
if ($after.group_id -or -not $after.latest) { throw 'Group deletion failed to preserve ungrouped node metrics.' }

$endpoint = Invoke-WebRequest -Uri 'http://127.0.0.1:18091/pinglake/latency' -Headers @{ Origin = 'https://monitor.example.com' } -UseBasicParsing
if ($endpoint.StatusCode -ne 204 -or $endpoint.Headers['Cache-Control'] -notmatch 'no-store') { throw 'Measurement endpoint returned incorrect status or cache policy.' }
try {
    Invoke-WebRequest -Uri 'http://127.0.0.1:18091/pinglake/latency' -Headers @{ Origin = 'https://other.example.com' } -ErrorAction Stop | Out-Null
    throw 'Measurement endpoint accepted a foreign origin.'
} catch {
    $status = 0
    if ($_.Exception.Response) {
        $status = [int]$_.Exception.Response.StatusCode
    }
    if ($status -ne 403) { throw }
}

@{ status = 'passed'; nodes = $nodes.Count; applied_revision = $saved.revision; probes = $results.Count; services = $services.Count; detailed_history_points = $history.Count; group_delete_preserved_node = $true; endpoint_origin_and_cache_checked = $true; browser_https_success_verified = $false } | ConvertTo-Json -Compress
