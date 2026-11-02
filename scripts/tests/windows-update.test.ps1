param([string]$Supervisor = (Join-Path $PSScriptRoot '..\..\crates\ratatoskr-core\src\windows_update.ps1'))
$ErrorActionPreference = 'Stop'
. $Supervisor

function Assert-Equal($Actual, $Expected) {
    if ($Actual -ne $Expected) { throw "Expected '$Expected', got '$Actual'" }
}

$events = [Collections.Generic.List[string]]::new()
$result = Invoke-UpdateTransaction -Install { $events.Add('install'); throw 'installer aborted' } -Restore { $events.Add('restore') } -Report { param($message); $events.Add("report:$message") } -Restart { $events.Add('restart') }
Assert-Equal $result 'installer aborted'
Assert-Equal ($events -join ',') 'install,restore,report:installer aborted,restart'

$events.Clear()
$result = Invoke-UpdateTransaction -Install { $events.Add('install') } -Restore { throw 'must not roll back success' } -Report { param($message); $events.Add("report:$message") } -Restart { $events.Add('restart') }
Assert-Equal $result $null
Assert-Equal ($events -join ',') 'install,report:,restart'

$events.Clear()
$caught = $false
try { Invoke-UpdateTransaction -Install {} -Restore {} -Report { throw 'disk unavailable' } -Restart { $events.Add('restart') } }
catch { $caught = $true }
Assert-Equal $caught $true
Assert-Equal ($events -join ',') 'restart'

$events.Clear()
$caught = $false
try { Invoke-UpdateTransaction -Install {} -Restore {} -Report { param($message); $events.Add("report:$message") } -Restart { throw 'cannot start desktop' } }
catch { $caught = $true }
Assert-Equal $caught $true
Assert-Equal ($events -join ',') 'report:,report:restarting sync failed: cannot start desktop'

$events.Clear()
$caught = $false
try { Invoke-UpdateTransaction -Install { Throw-UnstoppedInstaller 'termination denied' } -Restore { $events.Add('unsafe restore') } -Restart { $events.Add('unsafe restart') } -Report { param($message); $events.Add("report:$message") } }
catch { $caught = $true }
Assert-Equal $caught $true
Assert-Equal ($events -join ',') 'report:termination denied'

$child = Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList @('-NoProfile', '-NonInteractive', '-Command', 'Start-Sleep -Seconds 60') -WindowStyle Hidden -PassThru
$caught = $false
try { Wait-Installer $child 100 }
catch { $caught = $true }
Assert-Equal $caught $true
Assert-Equal $child.HasExited $true

$child = Start-UpdateProcess "$env:SystemRoot\System32\cmd.exe" '/c exit 0'
Wait-Installer $child 5000
Assert-Equal $child.ExitCode 0

$child = Start-UpdateProcess "$env:SystemRoot\System32\cmd.exe" '/c exit 17'
$caught = $false
try { Wait-Installer $child 5000 }
catch { $caught = $true; Assert-Equal $_.Exception.Message 'Installer failed with exit code 17' }
Assert-Equal $caught $true

$root = Join-Path ([IO.Path]::GetTempPath()) ('ratatoskr-supervisor-test-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $root 'backup'), (Join-Path $root 'installed') | Out-Null
$plan = [pscustomobject]@{ desktop = Join-Path $root 'installed\ratatoskr-desktop.exe'; data = $root; version = '0.2.19' }
foreach ($name in @('ratatoskr-desktop.exe', 'ratatoskr.exe')) {
    [IO.File]::WriteAllText((Join-Path $root "backup\$name"), 'previous executable')
    [IO.File]::WriteAllText((Join-Path $root "installed\$name"), 'partial installation')
}
Restore-Executables $plan $root
foreach ($name in @('ratatoskr-desktop.exe', 'ratatoskr.exe')) {
    Assert-Equal ([IO.File]::ReadAllText((Join-Path $root "installed\$name"))) 'previous executable'
    Assert-Equal ([IO.File]::ReadAllText((Join-Path $root "failed-install\$name"))) 'partial installation'
}
Write-UpdateResult $plan 'installer aborted'
$result = Get-Content (Join-Path $root 'windows-install-result.json') -Raw | ConvertFrom-Json
Assert-Equal $result.error 'Desktop update: installer aborted'
Write-UpdateResult $plan $null
$result = Get-Content (Join-Path $root 'windows-install-result.json') -Raw | ConvertFrom-Json
Assert-Equal $result.error $null

# Exercise the complete supervisor entry point with a real failing executable.
# Only tray launch is replaced; all paths, backups and state are disposable.
$transaction = Join-Path $root ('entry point-' + [char]0xE6 + [char]0x96EA)
New-Item -ItemType Directory -Path $transaction, (Join-Path $transaction 'backup') | Out-Null
Copy-Item "$env:SystemRoot\System32\where.exe" (Join-Path $transaction 'installer.exe')
foreach ($name in @('ratatoskr-desktop.exe', 'ratatoskr.exe')) {
    [IO.File]::WriteAllText((Join-Path $transaction "backup\$name"), 'working backup')
    [IO.File]::WriteAllText((Join-Path $transaction $name), 'installed executable')
}
$parent = Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList @('-NoProfile', '-NonInteractive', '-Command', 'Start-Sleep -Seconds 60') -WindowStyle Hidden -PassThru
$plan = @{ parent = $parent.Id; data = $transaction; desktop = Join-Path $transaction 'ratatoskr-desktop.exe'; version = '0.2.19'; sha256 = Get-UpdateHash (Join-Path $transaction 'installer.exe') }
[IO.File]::WriteAllText((Join-Path $transaction 'plan.json'), ($plan | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
$driver = Join-Path $transaction 'driver.ps1'
@'
param($Supervisor, $UpdatePlan)
. $Supervisor
function Start-SyncTray($Plan) { [IO.File]::WriteAllText((Join-Path $Plan.data 'restarted'), 'yes') }
Start-SupervisedUpdate $UpdatePlan
'@ | Set-Content $driver
$supervisorProcess = Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList @('-NoProfile', '-NonInteractive', '-File', "`"$driver`"", "`"$Supervisor`"", "`"$(Join-Path $transaction 'plan.json')`"") -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $transaction 'stderr.log')
$null = $supervisorProcess.Handle
$readyDeadline = [datetime]::UtcNow.AddSeconds(10)
while (-not (Test-Path -LiteralPath (Join-Path $transaction 'ready'))) {
    if ($supervisorProcess.HasExited -or [datetime]::UtcNow -ge $readyDeadline) { $parent.Kill(); throw 'Supervisor did not acknowledge the predecessor handle' }
    Start-Sleep -Milliseconds 25
}
Start-Sleep -Milliseconds 100
Assert-Equal (Test-Path -LiteralPath (Join-Path $transaction 'windows-install-result.json')) $false
$parent.Kill()
$parent.WaitForExit()
if (-not $supervisorProcess.WaitForExit(10000)) { $supervisorProcess.Kill(); throw 'Supervisor entry-point test timed out' }
Assert-Equal $supervisorProcess.ExitCode 1
Assert-Equal $parent.HasExited $true
Assert-Equal ([IO.File]::ReadAllText((Join-Path $transaction 'restarted'))) 'yes'
Assert-Equal ([IO.File]::ReadAllText((Join-Path $transaction 'ratatoskr-desktop.exe'))) 'working backup'
$outcome = Get-Content (Join-Path $transaction 'windows-install-result.json') -Raw | ConvertFrom-Json
if ($outcome.error -notmatch '^Desktop update: Installer failed with exit code [1-9][0-9]*$') { throw "Unexpected supervisor outcome: $($outcome.error)" }

$originalReport = ${function:Write-UpdateResult}
$originalToast = ${function:Show-RecoveryFailure}
try {
    $events.Clear()
    function Write-UpdateResult { throw 'disk full' }
    function Show-RecoveryFailure($AppId, $Message) { $events.Add("$AppId|$Message") }
    Report-RecoveryFailure ([pscustomobject]@{ app_id = 'test-app' }) 'restart failed'
    Assert-Equal ($events -join ',') 'test-app|restart failed; saving update diagnostics failed: disk full'
} finally {
    Set-Item Function:Write-UpdateResult $originalReport
    Set-Item Function:Show-RecoveryFailure $originalToast
}
Write-Output 'Windows supervisor behavior tests passed'
