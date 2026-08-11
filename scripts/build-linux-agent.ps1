[CmdletBinding()]
param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\release')
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

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$output = [IO.Path]::GetFullPath($OutputDirectory)
$outputParent = Split-Path -Parent $output
$transactionId = [Guid]::NewGuid().ToString('N')
$staging = Join-Path $outputParent ".pinglake-linux-agent-$transactionId"
$artifactName = 'pinglake-agent-linux-amd64'
$destination = Join-Path $output $artifactName
$backup = Join-Path $output ".$artifactName.$transactionId.backup"

New-Item -ItemType Directory -Force -Path $outputParent, $staging | Out-Null

try {
    Push-Location $repoRoot
    try {
        Invoke-Native -FilePath 'docker' -Arguments @(
            'build',
            '--file', 'deploy/Dockerfile.agent-linux-amd64',
            '--target', 'export',
            '--output', "type=local,dest=$staging",
            '.'
        ) -Description 'Building Linux PingLake Agent'
    } finally {
        Pop-Location
    }

    $candidate = Join-Path $staging $artifactName
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw "Docker build completed without $artifactName."
    }

    New-Item -ItemType Directory -Force -Path $output | Out-Null
    if (Test-Path -LiteralPath $destination) {
        [IO.File]::Replace($candidate, $destination, $backup, $true)
        Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
    } else {
        [IO.File]::Move($candidate, $destination)
    }

    $hash = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText(
        "$destination.sha256",
        "$hash  $artifactName`n",
        [Text.UTF8Encoding]::new($false)
    )
} finally {
    Remove-Item -LiteralPath $staging, $backup -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "Linux Agent written to $destination"
