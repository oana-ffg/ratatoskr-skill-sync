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

$child = Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList @('-NoProfile', '-NonInteractive', '-Command', 'Start-Sleep -Seconds 60') -WindowStyle Hidden -PassThru
$caught = $false
try { Wait-Installer $child 100 }
catch { $caught = $true }
Assert-Equal $caught $true
Assert-Equal $child.HasExited $true

$child = Start-Process -FilePath "$env:SystemRoot\System32\cmd.exe" -ArgumentList @('/c', 'exit 17') -WindowStyle Hidden -PassThru
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
Write-Output 'Windows supervisor behavior tests passed'
