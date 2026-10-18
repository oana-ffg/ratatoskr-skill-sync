$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$failures = [System.Collections.Generic.List[string]]::new()
$name = 'rss' + [guid]::NewGuid().ToString('N').Substring(0, 12)
$root = Join-Path $env:PUBLIC ('Ratatoskr-standard-user-' + [guid]::NewGuid().ToString('N'))
$config = Join-Path $root 'config Æ 🦀 & test.toml'
$configArgument = '\\?\' + $config
$hash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($configArgument))).ToLowerInvariant()
$taskName = 'io.github.oana-ffg.ratatoskr.' + $hash.Substring(0, 12)
$user = $null
$process = $null
$rootCreated = $false

function Invoke-Cleanup([string]$Label, [scriptblock]$Action) {
    try { & $Action; Write-Host "PASS: cleanup $Label" }
    catch { $failures.Add("Cleanup $Label failed: $_"); Write-Warning $failures[-1] }
}

try {
    New-Item -ItemType Directory -Path $root | Out-Null
    $rootCreated = $true
    $password = [Convert]::ToBase64String([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)) + 'aA1!'
    $securePassword = ConvertTo-SecureString $password -AsPlainText -Force
    $user = New-LocalUser -Name $name -Password $securePassword -Description 'Disposable Ratatoskr CI standard user'
    $sid = $user.SID.Value
    $users = Get-LocalGroup -SID 'S-1-5-32-545'
    Add-LocalGroupMember -Group $users -Member $user
    $administrators = Get-LocalGroup -SID 'S-1-5-32-544'
    if (@(Get-LocalGroupMember -Group $administrators | Where-Object { $_.SID.Value -eq $sid }).Count -ne 0) {
        throw 'Disposable test user unexpectedly belongs to Administrators'
    }
    & icacls.exe $root /grant "*${sid}:(OI)(CI)M"
    if ($LASTEXITCODE -ne 0) { throw "Grant fixture permissions failed: $LASTEXITCODE" }
    foreach ($binary in @('ratatoskr.exe', 'ratatoskrw.exe')) {
        Copy-Item (Join-Path $PSScriptRoot "../target/debug/$binary") (Join-Path $root $binary)
    }
    $child = Join-Path $root 'standard-user-check.ps1'
    Copy-Item (Join-Path $PSScriptRoot 'windows-standard-user-check.ps1') $child
    Start-Service seclogon
    $credential = [pscredential]::new("$env:COMPUTERNAME\$name", $securePassword)
    $arguments = "-NoProfile -NonInteractive -File `"$child`" -Root `"$root`" -ConfigPath `"$config`" -ExpectedSid $sid -TaskName $taskName"
    $process = Start-Process -FilePath (Get-Command pwsh.exe).Source -Credential $credential `
        -WorkingDirectory $root -ArgumentList $arguments -PassThru `
        -RedirectStandardOutput (Join-Path $root 'stdout.txt') -RedirectStandardError (Join-Path $root 'stderr.txt')
    if (-not $process.WaitForExit(120000)) {
        & taskkill.exe /PID $process.Id /T /F
        if ($LASTEXITCODE -ne 0) { throw "Standard-user test timed out; process cleanup failed: $LASTEXITCODE" }
        throw 'Standard-user test timed out after 120 seconds'
    }
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) { throw "Standard-user test failed with exit code $($process.ExitCode)" }
}
catch { $failures.Add("Standard-user scheduler test failed: $_") }
finally {
    if ($null -ne $process) { Invoke-Cleanup 'child process handles' { $process.Dispose() } }
    foreach ($log in @('stdout.txt', 'stderr.txt')) {
        if (Test-Path (Join-Path $root $log)) { Get-Content (Join-Path $root $log) }
    }
    Invoke-Cleanup 'task' {
        $remaining = @(Get-ScheduledTask | Where-Object { $_.TaskPath -eq '\' -and $_.TaskName -eq $taskName })
        foreach ($task in $remaining) { Unregister-ScheduledTask -InputObject $task -Confirm:$false }
        if (@(Get-ScheduledTask | Where-Object { $_.TaskName -eq $taskName }).Count -ne 0) {
            throw "Task $taskName still exists"
        }
    }
    if ($null -ne $user) {
        Invoke-Cleanup 'user profile' {
            Get-CimInstance Win32_UserProfile -Filter "SID='$($user.SID.Value)'" | Remove-CimInstance
        }
        Invoke-Cleanup 'user' {
            Remove-LocalUser -SID $user.SID
            if (@(Get-LocalUser | Where-Object { $_.SID -eq $user.SID }).Count -ne 0) {
                throw 'Disposable user still exists'
            }
        }
    }
    if ($rootCreated) { Invoke-Cleanup 'fixture' { Remove-Item $root -Recurse -Force } }
}
if ($failures.Count -ne 0) { throw ($failures -join "`n") }
Write-Host 'PASS: standard-user schedule registration/removal; task, account, profile and fixture cleaned up'
