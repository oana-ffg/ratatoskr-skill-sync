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

$env:RATATOSKR_TEST_CLI = Join-Path $Root 'ratatoskr.exe'
$env:RATATOSKR_TEST_CONFIG = $ConfigPath
& (Join-Path $Root 'windows_schedule.exe') --nocapture
if ($LASTEXITCODE -ne 0) { throw "Standard-user migration tests failed: $LASTEXITCODE" }
Write-Host 'PASS: real-registration migration, ownership preservation and disabled-task coverage as a standard user'
