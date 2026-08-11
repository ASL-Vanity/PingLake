[CmdletBinding()]
param(
    [switch]$RemoveState
)

$ErrorActionPreference = 'Stop'

function Invoke-Native {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FilePath,

        [Parameter(Mandatory = $true)]
        [string[]]$Arguments,

        [Parameter(Mandatory = $true)]
        [string]$Description
    )

    & $FilePath @Arguments | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "$Description failed with exit code $LASTEXITCODE."
    }
}
$currentIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($currentIdentity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this uninstaller from an elevated PowerShell window.'
}

$serviceName = 'PingLakeAgent'
$service = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
if ($service) {
    if ($service.Status -ne 'Stopped') {
        Stop-Service -Name $serviceName -Force
    }
    Invoke-Native -FilePath 'sc.exe' -Arguments @('delete', $serviceName) -Description 'Deleting PingLake Agent service'
}

$installDir = Join-Path $env:ProgramFiles 'PingLake'
$stateDir = Join-Path $env:ProgramData 'PingLake'
if (Test-Path -LiteralPath $installDir) {
    Remove-Item -LiteralPath $installDir -Recurse -Force
}

if ($RemoveState) {
    if (Test-Path -LiteralPath $stateDir) {
        Remove-Item -LiteralPath $stateDir -Recurse -Force
    }
    Write-Host 'PingLake Agent and state removed.'
} else {
    Write-Host "PingLake Agent removed. State remains in $stateDir."
}
