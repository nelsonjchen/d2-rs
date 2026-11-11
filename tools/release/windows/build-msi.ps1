# Builds d2-rs-<VERSION>-windows-<ARCH>.msi from a release archive and checks
# it: MSI properties, an administrative extraction whose files must match the
# archive byte for byte, and (with -InstallTest) a real install that puts
# d2-rs on the system PATH and an uninstall that removes it again.
# Adapted from Go D2's ci/release/windows/build.ps1 (MPL-2.0).
#
# WiX 7 binaries are distributed under the Open Source Maintenance Fee EULA
# (https://github.com/wixtoolset/wix/blob/main/OSMFEULA.txt). The repository
# owner accepted it for d2-rs on 2026-11-11; release.yml sets
# WIX7_EULA_ACCEPTED=true on that basis.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?$')]
    [string] $Version,

    [Parameter(Mandatory = $true)]
    [ValidateSet('amd64', 'arm64')]
    [string] $Arch,

    [Parameter(Mandatory = $true)]
    [string] $ArchivePath,

    [Parameter(Mandatory = $true)]
    [string] $OutputDirectory,

    [switch] $InstallTest
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

function Get-MsiProperty {
    param(
        [Parameter(Mandatory = $true)] $Database,
        [Parameter(Mandatory = $true)] [string] $Name
    )

    $query = "SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = '$Name'"
    $view = $Database.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $Database, @($query))
    $null = $view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null)
    $record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)
    if ($null -eq $record) {
        throw "MSI property $Name is missing"
    }
    return $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, 1)
}

function Get-MachinePath {
    return [Environment]::GetEnvironmentVariable('Path', [EnvironmentVariableTarget]::Machine)
}

if (-not (Test-Path -LiteralPath $ArchivePath -PathType Leaf)) {
    throw "archive does not exist: $ArchivePath"
}
if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
    throw 'wix is not installed or is not on PATH'
}
$wixVersion = (& wix --version | Out-String).Trim()
$wixSemVer = ($wixVersion -split '[+-]', 2)[0]
if ($wixSemVer -cne '7.0.0') {
    throw "WiX version is '$wixVersion', expected 7.0.0"
}
if ($env:WIX7_EULA_ACCEPTED -cne 'true') {
    throw 'WiX 7 requires explicit acceptance of the OSMF EULA; set WIX7_EULA_ACCEPTED=true.'
}

$temp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$workingDirectory = Join-Path $temp "d2-rs-msi-$Version-$Arch"
$archiveDirectory = Join-Path $workingDirectory 'archive'
$wixDirectory = Join-Path $workingDirectory 'wix'
$extractDirectory = Join-Path $workingDirectory 'extracted'
Remove-Item -LiteralPath $workingDirectory -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $archiveDirectory, $wixDirectory, $extractDirectory -Force | Out-Null

$archiveEntries = @(& tar -tzf $ArchivePath)
if ($LASTEXITCODE -ne 0 -or $archiveEntries.Count -eq 0) {
    throw 'unable to list release archive'
}
$archiveRoot = "d2-rs-$Version/"
foreach ($entry in $archiveEntries) {
    $normalizedEntry = $entry.Replace('\', '/')
    if ($normalizedEntry.StartsWith('/', [System.StringComparison]::Ordinal) -or
        $normalizedEntry -match '^[A-Za-z]:' -or
        $normalizedEntry -match '(^|/)\.\.(/|$)') {
        throw "release archive contains an unsafe path: $entry"
    }
    if (-not $normalizedEntry.StartsWith($archiveRoot, [System.StringComparison]::Ordinal)) {
        throw "release archive contains a path outside ${archiveRoot}: $entry"
    }
}

& tar -xzf $ArchivePath -C $archiveDirectory
if ($LASTEXITCODE -ne 0) {
    throw 'unable to extract release archive'
}

$releaseDirectory = Join-Path $archiveDirectory "d2-rs-$Version"
$payload = [ordered]@{
    'd2-rs.exe' = Join-Path $releaseDirectory 'bin/d2-rs.exe'
    'LICENSE' = Join-Path $releaseDirectory 'LICENSE'
    'THIRD_PARTY_NOTICES.md' = Join-Path $releaseDirectory 'THIRD_PARTY_NOTICES.md'
}
foreach ($name in $payload.Keys) {
    if (-not (Test-Path -LiteralPath $payload[$name] -PathType Leaf)) {
        throw "release archive does not contain $name"
    }
    Copy-Item -LiteralPath $payload[$name] -Destination (Join-Path $wixDirectory $name)
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'd2-rs.wxs') -Destination $wixDirectory

$msiVersion = $Version.Substring(1).Split('-')[0]
$wixArch = if ($Arch -eq 'amd64') { 'x64' } else { 'arm64' }
Push-Location $wixDirectory
try {
    & wix build -acceptEula wix7 -arch $wixArch -d "D2RsVersion=$msiVersion" -o d2-rs.msi ./d2-rs.wxs
    if ($LASTEXITCODE -ne 0) {
        throw "wix exited with status $LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

$msiPath = Join-Path $wixDirectory 'd2-rs.msi'
if (-not (Test-Path -LiteralPath $msiPath -PathType Leaf)) {
    throw 'wix did not produce d2-rs.msi'
}

$installer = New-Object -ComObject WindowsInstaller.Installer
$database = $installer.GetType().InvokeMember(
    'OpenDatabase', 'InvokeMethod', $null, $installer,
    @((Resolve-Path -LiteralPath $msiPath).Path, 0))
$expectedProperties = @{
    ProductName = 'd2-rs'
    Manufacturer = 'd2-rs'
    UpgradeCode = '{E07615D4-6C1F-4C0F-B2F4-7ED59CAA42C1}'
    ProductVersion = $msiVersion
}
foreach ($property in $expectedProperties.Keys) {
    $actual = Get-MsiProperty -Database $database -Name $property
    if ($actual -cne $expectedProperties[$property]) {
        throw "MSI $property is '$actual', expected '$($expectedProperties[$property])'"
    }
}
[void][Runtime.InteropServices.Marshal]::ReleaseComObject($database)

function Assert-Payload {
    param([Parameter(Mandatory = $true)] [string] $Directory)
    foreach ($name in $payload.Keys) {
        $found = @(Get-ChildItem -LiteralPath $Directory -Filter $name -File -Recurse)
        if ($found.Count -ne 1) {
            throw "expected exactly one $name under $Directory, found $($found.Count)"
        }
        $expected = (Get-FileHash -LiteralPath $payload[$name] -Algorithm SHA256).Hash
        $actual = (Get-FileHash -LiteralPath $found[0].FullName -Algorithm SHA256).Hash
        if ($expected -cne $actual) {
            throw "installed $name does not match the release archive"
        }
    }
    $exe = @(Get-ChildItem -LiteralPath $Directory -Filter 'd2-rs.exe' -File -Recurse)[0].FullName
    $versionOutput = (& $exe --version 2>&1 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $versionOutput -cne $Version) {
        throw "installed d2-rs.exe reported an unexpected version: $versionOutput"
    }
}

$extract = Start-Process -FilePath msiexec.exe -Wait -PassThru `
    -ArgumentList "/a `"$msiPath`" /qn TARGETDIR=`"$extractDirectory`""
if ($extract.ExitCode -notin @(0, 3010)) {
    throw "administrative MSI extraction exited with status $($extract.ExitCode)"
}
Assert-Payload -Directory $extractDirectory

if ($InstallTest) {
    $installDirectory = Join-Path $env:ProgramFiles 'd2-rs'
    $logPath = Join-Path $workingDirectory 'install.log'
    $install = Start-Process -FilePath msiexec.exe -Wait -PassThru `
        -ArgumentList "/i `"$msiPath`" /qn /norestart /l*v `"$logPath`""
    if ($install.ExitCode -notin @(0, 3010)) {
        throw "MSI install exited with status $($install.ExitCode); log: $logPath"
    }
    Assert-Payload -Directory $installDirectory
    $pathEntries = (Get-MachinePath) -split ';' | ForEach-Object { $_.TrimEnd('\') }
    if ($pathEntries -notcontains $installDirectory.TrimEnd('\')) {
        throw "install did not add $installDirectory to the system PATH"
    }

    $uninstall = Start-Process -FilePath msiexec.exe -Wait -PassThru `
        -ArgumentList "/x `"$msiPath`" /qn /norestart"
    if ($uninstall.ExitCode -notin @(0, 3010)) {
        throw "MSI uninstall exited with status $($uninstall.ExitCode)"
    }
    if (Test-Path -LiteralPath (Join-Path $installDirectory 'd2-rs.exe')) {
        throw 'uninstall left d2-rs.exe behind'
    }
    $pathEntries = (Get-MachinePath) -split ';' | ForEach-Object { $_.TrimEnd('\') }
    if ($pathEntries -contains $installDirectory.TrimEnd('\')) {
        throw "uninstall left $installDirectory on the system PATH"
    }
}

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$outputPath = Join-Path $OutputDirectory "d2-rs-$Version-windows-$Arch.msi"
Copy-Item -LiteralPath $msiPath -Destination $outputPath -Force
$outputHash = (Get-FileHash -LiteralPath $outputPath -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Host "Built $outputPath"
Write-Host "SHA-256: $outputHash"
