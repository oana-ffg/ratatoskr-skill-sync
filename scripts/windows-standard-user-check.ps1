param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$ConfigPath,
    [Parameter(Mandatory)][string]$ExpectedSid,
    [Parameter(Mandatory)][string]$TaskName
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if ($identity.User.Value -ne $ExpectedSid) { throw 'Child process is not the disposable user' }
if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) -or
    'S-1-5-32-544' -in @($identity.Groups | ForEach-Object Value)) {
    throw 'Child process has an administrator SID; this must be a genuine standard user'
}
Write-Host "PASS: genuine standard-user token, SID $ExpectedSid; no Administrators SID"

$source = Join-Path $Root 'source'
$state = Join-Path $Root 'state'
New-Item -ItemType Directory $source | Out-Null
$settings = "source = $(ConvertTo-Json $source -Compress)`nmachine = 'standard-user-ci'`nstate_dir = $(ConvertTo-Json $state -Compress)`n"
[IO.File]::WriteAllText($ConfigPath, $settings, [Text.UTF8Encoding]::new($false))
$configArgument = '\\?\' + $ConfigPath
$helper = [IO.Path]::GetFullPath((Join-Path $env:SystemRoot 'System32/where.exe'))
$helperArgument = '\\?\' + $helper
$cli = Join-Path $Root 'ratatoskr.exe'
function Invoke-Ratatoskr([string[]]$Arguments) {
    & $cli --config $configArgument --json @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Ratatoskr $($Arguments[0..1] -join ' ') failed: $LASTEXITCODE" }
}

[xml]$preview = Invoke-Ratatoskr -Arguments @('schedule', 'plan', '--helper', $helperArgument, '--seconds', '60') | Out-String
if ($preview.Task.Triggers.LogonTrigger.UserId -ne $ExpectedSid -or
    $preview.Task.Principals.Principal.UserId -ne $ExpectedSid) {
    throw 'Preview does not bind trigger and principal to the token SID'
}
if ($preview.Task.Actions.Exec.Command -ne $helper -or
    $preview.Task.Actions.Exec.Arguments -ne "--config `"$ConfigPath`" sync") {
    throw 'Preview does not preserve the expected Unicode paths'
}
# where.exe only searches for files; a trigger can never execute Ratatoskr or
# touch any real user configuration. Both CLI inputs deliberately use \\?\ paths.
Invoke-Ratatoskr -Arguments @('schedule', 'install', '--helper', $helperArgument, '--seconds', '60')
$recordPath = Join-Path $state 'standard-user-ci/installed-schedule.json'
$record = Get-Content -Raw $recordPath | ConvertFrom-Json
if ($record.label -ne $TaskName) { throw 'Unexpected task name; cleanup ownership would be ambiguous' }
[xml]$written = Get-Content -Raw $record.path
if ($written.Task.Principals.Principal.id -ne 'Author' -or
    $written.Task.Principals.Principal.LogonType -ne 'InteractiveToken' -or
    $written.Task.Principals.Principal.RunLevel -ne 'LeastPrivilege' -or
    $written.Task.Actions.Context -ne 'Author') {
    throw 'Written XML does not contain the explicit Author principal and action context'
}
if ($written.Task.Actions.Exec.Command -ne $helper -or
    $written.Task.Actions.Exec.Arguments -ne "--config `"$ConfigPath`" sync") {
    throw 'Written XML does not contain the expected simplified absolute paths'
}

$scheduler = New-Object -ComObject Schedule.Service
$scheduler.Connect()
$folder = $scheduler.GetFolder('\')
$task = $folder.GetTask($TaskName)
[xml]$registered = $task.Xml
$definition = $task.Definition
$registeredPrincipal = $registered.Task.Principals.Principal
Write-Host "Registered COM principal: UserId=$($definition.Principal.UserId); Id=$($definition.Principal.Id); LogonType=$($definition.Principal.LogonType); RunLevel=$($definition.Principal.RunLevel); Context=$($definition.Actions.Context)"
Write-Host "Registered XML: $($registered.OuterXml)"
function Resolve-Sid([string]$Value) {
    if ($Value.StartsWith('S-1-')) { return $Value }
    return [Security.Principal.NTAccount]::new($Value).Translate([Security.Principal.SecurityIdentifier]).Value
}
if ((Resolve-Sid $registeredPrincipal.UserId) -ne $ExpectedSid -or
    (Resolve-Sid $registered.Task.Triggers.LogonTrigger.UserId) -ne $ExpectedSid) {
    throw 'Registered task is not bound to the standard user'
}
# Query effective settings through COM rather than relying on the export's
# serialization of optional/default XML elements. TASK_LOGON_INTERACTIVE_TOKEN=3;
# TASK_RUNLEVEL_LUA=0. The written XML above must still set both explicitly.
if ($definition.Principal.LogonType -ne 3 -or
    $definition.Principal.RunLevel -ne 0 -or
    $definition.Principal.Id -ne 'Author' -or
    $definition.Actions.Context -ne 'Author') {
    throw 'Registered task does not use the explicit interactive least-privilege principal'
}
if ($registered.Task.Actions.Exec.Command -ne $helper -or
    $registered.Task.Actions.Exec.Arguments -ne "--config `"$ConfigPath`" sync") {
    throw 'Registered action paths differ from the simplified fixture paths'
}
Write-Host "PASS: real CLI registered $TaskName with this user's SID on trigger and InteractiveToken/LeastPrivilege principal"
Write-Host 'PASS: canonical helper/config inputs became plain absolute action paths'
Invoke-Ratatoskr -Arguments @('schedule', 'remove')
if (@($folder.GetTasks(1) | Where-Object Name -eq $TaskName).Count -ne 0 -or
    (Test-Path $recordPath) -or (Test-Path $record.path)) {
    throw 'CLI removal left the task or owned schedule files behind'
}
Write-Host 'PASS: real CLI removed the owned task and schedule files as the same standard user'
