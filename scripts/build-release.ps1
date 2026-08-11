[CmdletBinding()]
param(
    [string]$OutputDirectory
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

function Copy-RequiredFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Source,

        [Parameter(Mandatory = $true)]
        [string]$DestinationDirectory
    )

    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        throw "Required release artifact is missing: $Source"
    }
    Copy-Item -LiteralPath $Source -Destination $DestinationDirectory -Force
}

function Write-Sha256Manifest {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Directory
    )

    $manifestPath = Join-Path $Directory 'SHA256SUMS.txt'
    $prefix = "$Directory$([IO.Path]::DirectorySeparatorChar)"
    $lines = Get-ChildItem -LiteralPath $Directory -File -Recurse |
        Where-Object { $_.FullName -ne $manifestPath } |
        Sort-Object FullName |
        ForEach-Object {
            $relative = $_.FullName.Substring($prefix.Length).Replace('\', '/')
            $hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            "$hash  $relative"
        }
    [IO.File]::WriteAllLines($manifestPath, [string[]]$lines, [Text.UTF8Encoding]::new($false))
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot 'release'
}
$output = [IO.Path]::GetFullPath($OutputDirectory)
$outputParent = Split-Path -Parent $output
$transactionId = [Guid]::NewGuid().ToString('N')
$staging = Join-Path $outputParent ".pinglake-release-$transactionId"
$backup = Join-Path $outputParent ".pinglake-release-previous-$transactionId"
# Keep compiler intermediates outside the OneDrive-backed repository. File-provider
# activity can otherwise interrupt parallel rustc output without a useful diagnostic.
$cargoTarget = Join-Path ([IO.Path]::GetTempPath()) "pinglake-cargo-$transactionId"
$hadCargoTarget = Test-Path Env:CARGO_TARGET_DIR
$previousCargoTarget = $env:CARGO_TARGET_DIR
$previousMoved = $false
$published = $false

New-Item -ItemType Directory -Force -Path $outputParent, $staging, $cargoTarget | Out-Null

try {
    Push-Location (Join-Path $repoRoot 'web')
    try {
        Invoke-Native -FilePath 'npm' -Arguments @('ci') -Description 'Installing web dependencies'
        Invoke-Native -FilePath 'npm' -Arguments @('run', 'build') -Description 'Building web assets'
    } finally {
        Pop-Location
    }

    Push-Location $repoRoot
    try {
        $env:CARGO_TARGET_DIR = $cargoTarget
        Invoke-Native -FilePath 'cargo' -Arguments @('test', '--workspace', '--locked') -Description 'Running workspace tests'
        Invoke-Native -FilePath 'cargo' -Arguments @('build', '--release', '--locked', '-p', 'pinglake-hub', '-p', 'pinglake-agent') -Description 'Building Windows release binaries'
    } finally {
        Pop-Location
    }

    Copy-RequiredFile -Source (Join-Path $cargoTarget 'release\pinglake-hub.exe') -DestinationDirectory $staging
    Copy-RequiredFile -Source (Join-Path $cargoTarget 'release\pinglake-agent.exe') -DestinationDirectory $staging

    & (Join-Path $repoRoot 'scripts\build-linux-agent.ps1') -OutputDirectory $staging

    foreach ($relativePath in @(
        'deploy\install-agent-windows.ps1',
        'deploy\uninstall-agent-windows.ps1',
        'deploy\install-agent-linux.sh',
        'deploy\uninstall-agent-linux.sh',
        'deploy\pinglake-agent.service',
        'README.md',
        'SECURITY.md',
        'LICENSE'
    )) {
        Copy-RequiredFile -Source (Join-Path $repoRoot $relativePath) -DestinationDirectory $staging
    }
    Write-Sha256Manifest -Directory $staging

    if (Test-Path -LiteralPath $output) {
        Move-Item -LiteralPath $output -Destination $backup -ErrorAction Stop
        $previousMoved = $true
    }
    Move-Item -LiteralPath $staging -Destination $output -ErrorAction Stop
    $published = $true
    $staging = $null

    if ($previousMoved) {
        Remove-Item -LiteralPath $backup -Recurse -Force -ErrorAction SilentlyContinue
    }
} catch {
    $failure = $_
    if (-not $published -and $previousMoved -and -not (Test-Path -LiteralPath $output) -and (Test-Path -LiteralPath $backup)) {
        Move-Item -LiteralPath $backup -Destination $output -ErrorAction SilentlyContinue
    }
    throw $failure
} finally {
    if ($hadCargoTarget) {
        $env:CARGO_TARGET_DIR = $previousCargoTarget
    } else {
        Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    }
    if ($staging -and (Test-Path -LiteralPath $staging)) {
        Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
    }
    if (Test-Path -LiteralPath $cargoTarget) {
        Remove-Item -LiteralPath $cargoTarget -Recurse -Force -ErrorAction SilentlyContinue
    }
    if ($published -and (Test-Path -LiteralPath $backup)) {
        Remove-Item -LiteralPath $backup -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Write-Host "Release files written to $output"
