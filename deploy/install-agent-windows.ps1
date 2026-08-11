[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Binary,

    [Parameter(Mandatory = $true)]
    [string]$HubUrl,

    [Security.SecureString]$EnrollmentToken,

    [string]$EnrollmentTokenFile,

    [string]$Sha256,

    [string]$Name = $env:COMPUTERNAME,

    [switch]$AllowInsecureHttp
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

function Assert-RestrictedAcl {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [hashtable]$RequiredRights
    )

    $acl = Get-Acl -LiteralPath $Path
    if (-not $acl.AreAccessRulesProtected) {
        throw "ACL inheritance remains enabled on $Path."
    }

    $rules = $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])
    foreach ($entry in $RequiredRights.GetEnumerator()) {
        $sid = [Security.Principal.SecurityIdentifier]::new([string]$entry.Key)
        $required = [Security.AccessControl.FileSystemRights]$entry.Value
        $allowed = @($rules | Where-Object {
            $_.AccessControlType -eq [Security.AccessControl.AccessControlType]::Allow -and
            $_.IdentityReference.Value -eq $sid.Value -and
            (($_.FileSystemRights -band $required) -eq $required)
        })
        if ($allowed.Count -eq 0) {
            throw "ACL on $Path does not grant the required access to $($sid.Value)."
        }
    }

    $allowedSids = @($RequiredRights.Keys | ForEach-Object { [string]$_ })
    $unexpected = @($rules | Where-Object {
        $_.AccessControlType -eq [Security.AccessControl.AccessControlType]::Allow -and
        $_.IdentityReference.Value -notin $allowedSids
    })
    if ($unexpected.Count -ne 0) {
        throw "ACL on $Path contains unexpected allow rules."
    }
}

function Set-ExactRestrictedAcl {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [Security.AccessControl.FileSystemRights]$LocalServiceRights,

        [switch]$Directory
    )

    $acl = Get-Acl -LiteralPath $Path
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($rule in @($acl.Access)) {
        [void]$acl.RemoveAccessRuleAll($rule)
    }

    $inheritance = [Security.AccessControl.InheritanceFlags]::None
    if ($Directory) {
        $inheritance = [Security.AccessControl.InheritanceFlags]::ContainerInherit -bor
            [Security.AccessControl.InheritanceFlags]::ObjectInherit
    }
    $propagation = [Security.AccessControl.PropagationFlags]::None
    $allow = [Security.AccessControl.AccessControlType]::Allow
    foreach ($entry in @(
        @{ Sid = 'S-1-5-18'; Rights = [Security.AccessControl.FileSystemRights]::FullControl },
        @{ Sid = 'S-1-5-32-544'; Rights = [Security.AccessControl.FileSystemRights]::FullControl },
        @{ Sid = 'S-1-5-19'; Rights = $LocalServiceRights }
    )) {
        $sid = [Security.Principal.SecurityIdentifier]::new($entry.Sid)
        $rule = [Security.AccessControl.FileSystemAccessRule]::new(
            $sid,
            $entry.Rights,
            $inheritance,
            $propagation,
            $allow
        )
        [void]$acl.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $Path -AclObject $acl
}

function Set-RestrictedDirectoryAcl {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$LocalServiceRights
    )

    $rights = if ($LocalServiceRights -eq 'M') {
        [Security.AccessControl.FileSystemRights]::Modify
    } else {
        [Security.AccessControl.FileSystemRights]::ReadAndExecute
    }
    Set-ExactRestrictedAcl -Path $Path -LocalServiceRights $rights -Directory
    Assert-RestrictedAcl -Path $Path -RequiredRights @{
        'S-1-5-18' = [Security.AccessControl.FileSystemRights]::FullControl
        'S-1-5-32-544' = [Security.AccessControl.FileSystemRights]::FullControl
        'S-1-5-19' = $rights
    }
}

function Set-RestrictedConfigAcl {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    Set-ExactRestrictedAcl -Path $Path -LocalServiceRights ([Security.AccessControl.FileSystemRights]::ReadAndExecute)
    Assert-RestrictedAcl -Path $Path -RequiredRights @{
        'S-1-5-18' = [Security.AccessControl.FileSystemRights]::FullControl
        'S-1-5-32-544' = [Security.AccessControl.FileSystemRights]::FullControl
        'S-1-5-19' = [Security.AccessControl.FileSystemRights]::ReadAndExecute
    }
}

function Assert-Sha256 {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [string]$ExpectedSha256
    )

    if ([string]::IsNullOrWhiteSpace($ExpectedSha256)) {
        return
    }
    if ($ExpectedSha256 -notmatch '^[A-Fa-f0-9]{64}$') {
        throw 'Sha256 must be exactly 64 hexadecimal characters.'
    }

    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    if (-not [string]::Equals($actual, $ExpectedSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Binary SHA-256 does not match -Sha256.'
    }
}

function Replace-FileAtomically {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Candidate,

        [Parameter(Mandatory = $true)]
        [string]$Destination,

        [Parameter(Mandatory = $true)]
        [string]$Backup
    )

    if (Test-Path -LiteralPath $Destination) {
        [IO.File]::Replace($Candidate, $Destination, $Backup, $true)
        return $true
    }

    [IO.File]::Move($Candidate, $Destination)
    return $false
}

function Restore-ReplacedFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Destination,

        [Parameter(Mandatory = $true)]
        [string]$Backup,

        [bool]$OriginalExisted
    )

    if ($OriginalExisted) {
        if (Test-Path -LiteralPath $Destination) {
            [IO.File]::Replace($Backup, $Destination, $null, $true)
        } else {
            [IO.File]::Move($Backup, $Destination)
        }
    } elseif (Test-Path -LiteralPath $Destination) {
        Remove-Item -LiteralPath $Destination -Force
    }
}

function Get-ScStartMode {
    param([string]$StartMode)

    switch ($StartMode) {
        'Auto' { 'auto' }
        'Disabled' { 'disabled' }
        default { 'demand' }
    }
}

$currentIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($currentIdentity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this installer from an elevated PowerShell window.'
}

try {
    $hubUri = [Uri]$HubUrl
} catch {
    throw 'HubUrl must be a valid HTTP(S) URL.'
}
if ($hubUri.Scheme -notin @('http', 'https')) {
    throw 'HubUrl must use HTTP or HTTPS.'
}
$ipAddress = $null
$isLoopback = $hubUri.Host -eq 'localhost' -or
    ([Net.IPAddress]::TryParse($hubUri.DnsSafeHost, [ref]$ipAddress) -and [Net.IPAddress]::IsLoopback($ipAddress))
if ($hubUri.Scheme -eq 'http' -and -not $isLoopback -and -not $AllowInsecureHttp) {
    throw 'Non-loopback HubUrl must use HTTPS unless -AllowInsecureHttp is explicitly supplied.'
}

if ($EnrollmentTokenFile) {
    $tokenPlain = [IO.File]::ReadAllText((Resolve-Path -LiteralPath $EnrollmentTokenFile)).Trim()
} elseif ($env:PINGLAKE_ENROLLMENT_TOKEN) {
    $tokenPlain = $env:PINGLAKE_ENROLLMENT_TOKEN
} else {
    if (-not $EnrollmentToken) {
        $EnrollmentToken = Read-Host 'Enrollment token' -AsSecureString
    }
    $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($EnrollmentToken)
    try {
        $tokenPlain = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
    } finally {
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
    }
}
if ([string]::IsNullOrWhiteSpace($tokenPlain)) {
    throw 'Enrollment token must not be empty.'
}

$installDir = Join-Path $env:ProgramFiles 'PingLake'
$stateDir = Join-Path $env:ProgramData 'PingLake'
$binaryPath = Join-Path $installDir 'pinglake-agent.exe'
$configPath = Join-Path $stateDir 'agent.json'
$serviceName = 'PingLakeAgent'
$transactionId = [Guid]::NewGuid().ToString('N')
$binaryCandidate = Join-Path $installDir ".pinglake-agent.$transactionId.candidate.exe"
$configCandidate = Join-Path $stateDir ".agent.$transactionId.candidate.json"
$binaryBackup = Join-Path $installDir ".pinglake-agent.$transactionId.backup.exe"
$configBackup = Join-Path $stateDir ".agent.$transactionId.backup.json"
$binaryReplaced = $false
$configReplaced = $false
$binaryOriginalExisted = $false
$configOriginalExisted = $false
$serviceCreated = $false
$serviceUpdated = $false
$serviceStarted = $false

try {
    New-Item -ItemType Directory -Force -Path $installDir, $stateDir | Out-Null
    Set-RestrictedDirectoryAcl -Path $stateDir -LocalServiceRights 'M'
    Set-RestrictedDirectoryAcl -Path $installDir -LocalServiceRights 'RX'

    if ([Uri]::IsWellFormedUriString($Binary, [UriKind]::Absolute)) {
        $binaryUri = [Uri]$Binary
        if ($binaryUri.Scheme -ne 'https') {
            throw 'Binary download URL must use HTTPS.'
        }
        if ([string]::IsNullOrWhiteSpace($Sha256)) {
            throw 'Remote binary URLs require -Sha256.'
        }
        Invoke-WebRequest -Uri $binaryUri -OutFile $binaryCandidate -UseBasicParsing -MaximumRedirection 5
    } else {
        Copy-Item -LiteralPath $Binary -Destination $binaryCandidate -Force
    }
    Assert-Sha256 -Path $binaryCandidate -ExpectedSha256 $Sha256
    Invoke-Native -FilePath $binaryCandidate -Arguments @('--version') -Description 'Validating PingLake Agent binary'

    $config = [ordered]@{
        hub_url = $HubUrl
        enrollment_token = $tokenPlain
        name = $Name
        interval_secs = 5
        state_dir = $stateDir
        insecure_skip_verify = $false
        allow_insecure_http = [bool]$AllowInsecureHttp
    }
    $json = $config | ConvertTo-Json
    [IO.File]::WriteAllText($configCandidate, $json, [Text.UTF8Encoding]::new($false))
    Set-RestrictedConfigAcl -Path $configCandidate
    $tokenPlain = $null

    $existingService = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
    $existingServiceCim = if ($existingService) {
        Get-CimInstance Win32_Service -Filter "Name='$serviceName'"
    }
    $wasRunning = $existingService -and $existingService.Status -ne 'Stopped'

    if ($wasRunning) {
        Stop-Service -Name $serviceName -ErrorAction Stop
    }

    $binaryOriginalExisted = Replace-FileAtomically -Candidate $binaryCandidate -Destination $binaryPath -Backup $binaryBackup
    $binaryReplaced = $true
    $configOriginalExisted = Replace-FileAtomically -Candidate $configCandidate -Destination $configPath -Backup $configBackup
    $configReplaced = $true
    Set-RestrictedConfigAcl -Path $configPath

    $serviceCommand = '"{0}" --service --config "{1}"' -f $binaryPath, $configPath
    if ($existingService) {
        $serviceRegistryPath = Join-Path 'HKLM:\SYSTEM\CurrentControlSet\Services' $serviceName
        Set-ItemProperty -LiteralPath $serviceRegistryPath -Name ImagePath -Value $serviceCommand
        Invoke-Native -FilePath 'sc.exe' -Arguments @(
            'config', $serviceName, 'start=', 'auto', 'obj=', 'NT AUTHORITY\LocalService'
        ) -Description 'Updating PingLake Agent service'
        $serviceUpdated = $true
    } else {
        New-Service -Name $serviceName -BinaryPathName $serviceCommand -StartupType Automatic -DisplayName 'PingLake Agent' | Out-Null
        $serviceCreated = $true
        Invoke-Native -FilePath 'sc.exe' -Arguments @(
            'config', $serviceName, 'obj=', 'NT AUTHORITY\LocalService'
        ) -Description 'Setting PingLake Agent service account'
    }
    Invoke-Native -FilePath 'sc.exe' -Arguments @('description', $serviceName, 'Read-only host metrics agent for PingLake.') -Description 'Setting PingLake Agent service description'
    Invoke-Native -FilePath 'sc.exe' -Arguments @('failure', $serviceName, 'reset=', '86400', 'actions=', 'restart/5000/restart/15000/restart/60000') -Description 'Configuring PingLake Agent service recovery'
    Start-Service -Name $serviceName -ErrorAction Stop
    $serviceStarted = $true
    if ((Get-Service -Name $serviceName).Status -ne 'Running') {
        throw 'PingLake Agent service did not reach the Running state.'
    }

    Remove-Item -LiteralPath $binaryBackup, $configBackup -Force -ErrorAction SilentlyContinue
    Write-Host 'PingLake Agent installed and running. Rotate the Hub enrollment token after all nodes enroll.'
} catch {
    $failure = $_
    $rollbackErrors = [Collections.Generic.List[string]]::new()
    try {
        if ($serviceStarted -or $wasRunning) {
            Stop-Service -Name $serviceName -Force -ErrorAction SilentlyContinue
        }
    } catch {
        $rollbackErrors.Add("stopping service: $($_.Exception.Message)")
    }
    try {
        if ($serviceCreated) {
            Invoke-Native -FilePath 'sc.exe' -Arguments @('delete', $serviceName) -Description 'Removing failed PingLake Agent service'
        } elseif ($serviceUpdated -and $existingServiceCim) {
            Invoke-Native -FilePath 'sc.exe' -Arguments @(
                'config',
                $serviceName,
                'binPath=', $existingServiceCim.PathName,
                'start=', (Get-ScStartMode $existingServiceCim.StartMode),
                'obj=', $existingServiceCim.StartName
            ) -Description 'Restoring PingLake Agent service configuration'
        }
    } catch {
        $rollbackErrors.Add("restoring service configuration: $($_.Exception.Message)")
    }
    try {
        if ($configReplaced) {
            Restore-ReplacedFile -Destination $configPath -Backup $configBackup -OriginalExisted $configOriginalExisted
        }
    } catch {
        $rollbackErrors.Add("restoring configuration: $($_.Exception.Message)")
    }
    try {
        if ($binaryReplaced) {
            Restore-ReplacedFile -Destination $binaryPath -Backup $binaryBackup -OriginalExisted $binaryOriginalExisted
        }
    } catch {
        $rollbackErrors.Add("restoring binary: $($_.Exception.Message)")
    }
    try {
        if ($wasRunning) {
            Start-Service -Name $serviceName -ErrorAction SilentlyContinue
        }
    } catch {
        $rollbackErrors.Add("restarting previous service: $($_.Exception.Message)")
    }
    if ($rollbackErrors.Count -gt 0) {
        Write-Error "PingLake Agent rollback was incomplete: $($rollbackErrors -join '; ')"
    }
    throw $failure
} finally {
    $tokenPlain = $null
    Remove-Item -LiteralPath $binaryCandidate, $configCandidate, $binaryBackup, $configBackup -Force -ErrorAction SilentlyContinue
}
