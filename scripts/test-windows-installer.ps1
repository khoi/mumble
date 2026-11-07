# Silently installs, reinstalls, installs over another version with, and
# uninstalls a mumble-setup-*.exe on a disposable Windows machine (CI), checking
# the managed layout that src/updater/windows.rs expects at every step. It
# uses the real %APPDATA%, so never run it on a machine with Mumble installed.
#
#   scripts/test-windows-installer.ps1 -Installer dist\mumble-setup-2.4.0-windows-x64.exe `
#     -Executable dist\mumble-2.4.0-windows-x64.exe -Console dist\mumble-2.4.0-windows-x64.com -Version 2.4.0
#
# -Console (mumble.com) is left out only for a release published before it.
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Installer,
    [Parameter(Mandatory)] [string]$Executable,
    [string]$Console,
    [Parameter(Mandatory)] [string]$Version
)
$ErrorActionPreference = 'Stop'

$support = Join-Path $env:APPDATA 'mumble'
$installed = Join-Path $support "versions\$Version\mumble.exe"
$installedConsole = Join-Path $support "versions\$Version\mumble.com"
$bin = Join-Path $support 'bin'
$pointer = Join-Path $support 'current-version'
$receipt = Join-Path $support 'install.json'
$shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Mumble.lnk'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{6ED48C92-5177-41F4-B8D6-37B7B02CFDF4}_is1'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$logs = Join-Path $env:RUNNER_TEMP 'mumble-installer-logs'
New-Item -ItemType Directory -Force $logs | Out-Null

function Check([bool]$condition, [string]$message) {
    if (-not $condition) { throw "FAIL: $message" }
    Write-Output "ok - $message"
}

function Run-Setup([string]$name) {
    $process = Start-Process $Installer -Wait -PassThru -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=$logs\$name.log"
    )
    $process.ExitCode
}

if (Test-Path $support) { throw "$support already exists; run this only on a clean machine." }
$expected = (Get-FileHash -Algorithm SHA256 $Executable).Hash

Check ((Run-Setup 'install') -eq 0) 'silent install exits 0'
Check (Test-Path $installed) "mumble.exe is at versions\$Version"
Check ((Get-FileHash -Algorithm SHA256 $installed).Hash -eq $expected) 'installed mumble.exe matches the release bytes'
Check ((Get-Content $pointer -Raw).Trim() -eq $Version) "current-version names $Version"
$installReceipt = Get-Content $receipt -Raw | ConvertFrom-Json
Check ($installReceipt.app -eq 'mumble' -and $installReceipt.artifact -eq 'desktop' -and $installReceipt.channel -eq 'stable' -and $installReceipt.method -eq 'direct') 'install.json marks a direct install of mumble desktop stable'
Check ($installReceipt.root -eq $support) 'install.json names the install root'
# Mumble reads the receipt as UTF-8; Setup once wrote the ANSI code page (#1352).
$receiptBytes = [IO.File]::ReadAllBytes($receipt)
Check (-not ($receiptBytes.Length -ge 3 -and $receiptBytes[0] -eq 0xEF -and $receiptBytes[1] -eq 0xBB -and $receiptBytes[2] -eq 0xBF)) 'install.json has no byte order mark'
$strict = New-Object System.Text.UTF8Encoding($false, $true)
Check ($null -ne $strict.GetString($receiptBytes)) 'install.json is valid UTF-8'
$link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
Check ($link.TargetPath -eq $installed) 'Start Menu shortcut targets the managed executable'
Check ($link.Arguments -eq 'app') 'Start Menu shortcut launches the app'
Check ($link.Description -eq 'Mumble managed install') 'Start Menu shortcut carries the managed mark the updater repoints'
$entry = Get-ItemProperty $uninstallKey
Check ($entry.DisplayName -eq 'Mumble') 'Apps & Features lists Mumble'
Check ($entry.DisplayVersion -eq $Version) "Apps & Features shows $Version"
$uninstaller = ($entry.UninstallString -replace '"', '')
Check ($uninstaller -like "$support\uninstall\*") 'the uninstaller lives outside versions\, which updates prune'
Check (Test-Path $uninstaller) 'the uninstaller exists'

function User-Path { [Environment]::GetEnvironmentVariable('Path', 'User') }
function Path-Has([string]$folder) {
    @((User-Path) -split ';' | Where-Object { $_.TrimEnd('\') -eq $folder }).Count -gt 0
}

# Starts without a Vulkan loader too (#1240), which some runners lack.
if ($Console) {
    $expectedConsole = (Get-FileHash -Algorithm SHA256 $Console).Hash
    Check (Test-Path $installedConsole) "mumble.com is beside mumble.exe at versions\$Version"
    Check ((Get-FileHash -Algorithm SHA256 $installedConsole).Hash -eq $expectedConsole) 'installed mumble.com matches the release bytes'
    Check ((Get-FileHash -Algorithm SHA256 (Join-Path $bin 'mumble.com')).Hash -eq $expectedConsole) 'bin\mumble.com matches the release bytes'
    Check (Path-Has $bin) 'bin is on the user''s PATH'
    # mumble.com is a console program, so PowerShell waits for it, prints or
    # captures its output, and sets $LASTEXITCODE, as for any other command.
    $reported = (& $installedConsole --version) -split '\s+' | Select-Object -Last 1
    Check ($reported -eq $Version) "installed mumble.com runs mumble.exe, which reports $Version"
    # A new terminal: the user's PATH, where `mumble` resolves to bin\mumble.com.
    $env:Path = "$(User-Path);$([Environment]::GetEnvironmentVariable('Path', 'Machine'))"
    $command = Get-Command mumble -CommandType Application | Select-Object -First 1
    Check ($command.Source -eq (Join-Path $bin 'mumble.com')) "``mumble`` resolves to bin\mumble.com ($($command.Source))"
    $started = Get-Date
    $status = mumble status --json
    $code = $LASTEXITCODE
    Check ($code -eq 3) "``mumble status --json`` waits and exits 3, Mumble isn't running ($code)"
    Check (($status | Out-String) -match 'not-running') "``mumble status --json`` printed its reply: $status"
    Check ($null -ne (($status | Out-String) | ConvertFrom-Json)) '``mumble status --json`` printed JSON'
    Write-Output "  (mumble status --json took $([int]((Get-Date) - $started).TotalMilliseconds) ms)"
} else {
    # A release before mumble.com: its mumble.exe was a console program.
    $out = Join-Path $logs 'version.txt'
    Start-Process $installed -ArgumentList '--version' -Wait -NoNewWindow -RedirectStandardOutput $out
    $reported = (Get-Content $out -Raw).Trim() -split '\s+' | Select-Object -Last 1
    Check ($reported -eq $Version) "installed mumble.exe reports $Version"
}

Check ((Run-Setup 'reinstall') -eq 0) 'reinstalling the same version exits 0'
Check ((Get-FileHash -Algorithm SHA256 $installed).Hash -eq $expected) 'reinstall keeps the release bytes'
if ($Console) {
    Check (@((User-Path) -split ';' | Where-Object { $_.TrimEnd('\') -eq $bin }).Count -eq 1) 'reinstalling adds bin to PATH once'
}

# Versions are never compared: the installer installs its own over any other.
Set-Content -Path $pointer -Value '99.0.0' -NoNewline
Check ((Run-Setup 'over-another') -eq 0) 'installing over another version exits 0'
Check ((Get-Content $pointer -Raw).Trim() -eq $Version) 'current-version names this installer''s version'

# Simulate what a self-update and Launch at login leave behind.
$updated = Join-Path $support 'versions\99.0.0\mumble.exe'
New-Item -ItemType Directory -Force (Split-Path $updated) | Out-Null
Copy-Item $installed $updated
if ($Console) {
    Copy-Item $installedConsole (Join-Path (Split-Path $updated) 'mumble.com')
    Copy-Item (Join-Path $bin 'mumble.com') (Join-Path $bin 'mumble.com.old')
}
New-Item -ItemType Directory -Force (Join-Path $support 'updates') | Out-Null
if (-not (Test-Path $runKey)) { New-Item -Path $runKey | Out-Null }
Set-ItemProperty -Path $runKey -Name 'Mumble' -Value "`"$updated`" app --hidden"
Set-Content -Path (Join-Path $support 'settings.json') -Value '{}'

$process = Start-Process $uninstaller -Wait -PassThru -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=$logs\uninstall.log"
Check ($process.ExitCode -eq 0) 'silent uninstall exits 0'
# The uninstaller finishes from a temporary copy of itself.
$deadline = (Get-Date).AddSeconds(60)
while ((Test-Path $uninstallKey) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
Check (-not (Test-Path $uninstallKey)) 'Apps & Features entry is removed'
Check (-not (Test-Path (Join-Path $support 'versions'))) 'every version, including self-updated ones, is removed'
Check (-not (Test-Path (Join-Path $support 'updates'))) 'staged updates are removed'
Check (-not (Test-Path $pointer)) 'current-version is removed'
Check (-not (Test-Path $receipt)) 'install.json is removed'
Check (-not (Test-Path $shortcut)) 'Start Menu shortcut is removed'
Check ($null -eq (Get-ItemProperty -Path $runKey -Name 'Mumble' -ErrorAction SilentlyContinue)) 'Launch at login is removed'
Check (Test-Path (Join-Path $support 'settings.json')) 'settings are kept'
if ($Console) {
    Check (-not (Test-Path $bin)) 'bin is removed'
    Check (-not (Path-Has $bin)) 'bin is off the user''s PATH'
}
Remove-Item $support -Recurse -Force
Write-Output "Installer $([IO.Path]::GetFileName($Installer)) passed."
# `mumble status` left $LASTEXITCODE at 3, which would become this script's code.
exit 0
