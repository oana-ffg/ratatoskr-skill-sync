param([string]$PlanPath)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Write-UpdateResult($Plan, [string]$ErrorMessage) {
    $path = Join-Path $Plan.data 'windows-install-result.json'
    $temporary = "$path.$([guid]::NewGuid().ToString('N')).tmp"
    $result = @{ version = $Plan.version; finished_at = [datetime]::UtcNow.ToString('o'); error = $null }
    if ($ErrorMessage) { $result.error = "Desktop update: $ErrorMessage" }
    [IO.File]::WriteAllText($temporary, ($result | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    if ([IO.File]::Exists($path)) { [IO.File]::Replace($temporary, $path, "$path.previous-$([guid]::NewGuid().ToString('N'))") }
    else { [IO.File]::Move($temporary, $path) }
}

function Wait-Installer($Child, [int]$TimeoutMilliseconds = 120000) {
    if (-not $Child.WaitForExit($TimeoutMilliseconds)) {
        # Kill the owned process tree before restoring any files. Never leave an
        # installer capable of overwriting the recovered, running installation.
        $termination = Start-Process -FilePath "$env:SystemRoot\System32\taskkill.exe" -ArgumentList @('/PID', $Child.Id, '/T', '/F') -WindowStyle Hidden -PassThru
        if (-not $termination.WaitForExit(10000)) { throw 'Installer timed out; process-tree termination also timed out' }
        if (-not $Child.WaitForExit(10000)) { throw 'Installer timed out; could not stop its process tree' }
        if ($termination.ExitCode -ne 0) { throw "Installer timed out; process-tree termination failed ($($termination.ExitCode))" }
        throw "Installer timed out after $TimeoutMilliseconds milliseconds and was stopped"
    }
    if ($Child.ExitCode -ne 0) { throw "Installer failed with exit code $($Child.ExitCode)" }
}

function Invoke-UpdateTransaction([scriptblock]$Install, [scriptblock]$Restore, [scriptblock]$Restart, [scriptblock]$Report) {
    $failure = $null
    try { & $Install }
    catch {
        $failure = $_.Exception.Message
        try { & $Restore }
        catch { $failure += "; restoring the previous executables failed: $($_.Exception.Message)" }
    }
    # A failed ledger write must not prevent the restart. The caller's outer
    # catch also provides a native alert when reporting itself fails.
    $reportFailure = $null
    try { & $Report $failure } catch { $reportFailure = $_.Exception.Message }
    try { & $Restart }
    catch {
        $failure = (@($failure, "restarting sync failed: $($_.Exception.Message)") | Where-Object { $_ }) -join '; '
        & $Report $failure
        throw $failure
    }
    if ($reportFailure) { throw "Saving the update outcome failed: $reportFailure; $failure" }
    return $failure
}

function Restore-Executables($Plan, [string]$Root) {
    $installation = Split-Path -Parent $Plan.desktop
    $failed = Join-Path $Root 'failed-install'
    New-Item -ItemType Directory -Path $failed -Force | Out-Null
    foreach ($name in @('ratatoskr-desktop.exe', 'ratatoskr.exe')) {
        $target = Join-Path $installation $name
        if (Test-Path -LiteralPath $target) { Move-Item -LiteralPath $target -Destination (Join-Path $failed $name) }
        Copy-Item -LiteralPath (Join-Path $Root "backup\$name") -Destination $target
    }
}

function Start-SyncTray($Plan) {
    $child = Start-Process -FilePath $Plan.desktop -ArgumentList '--background' -WindowStyle Hidden -PassThru
    if ($child.WaitForExit(3000)) { throw "Desktop exited during recovery ($($child.ExitCode))" }
}

function Show-RecoveryFailure([string]$Message) {
    # This process survives the tray: it can notify even if the tray cannot start.
    [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType=WindowsRuntime] > $null
    [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType=WindowsRuntime] > $null
    $xml = New-Object Windows.Data.Xml.Dom.XmlDocument
    $xml.LoadXml('<toast><visual><binding template="ToastText02"><text id="1">Ratatoskr update needs attention</text><text id="2"></text></binding></visual></toast>')
    $xml.GetElementsByTagName('text').Item(1).InnerText = $Message
    [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Ratatoskr Skill Sync').Show([Windows.UI.Notifications.ToastNotification]::new($xml))
}

function Start-SupervisedUpdate([string]$Path) {
    $root = Split-Path -Parent $Path
    $plan = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    $installer = Join-Path $root 'installer.exe'
    if ((Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash -ne $plan.sha256) { throw 'Staged installer hash mismatch' }
    $parentProcess = Get-Process -Id $plan.parent
    # Open the handle before acknowledging readiness; PID reuse cannot redirect
    # this wait to a different process after the original desktop exits.
    $null = $parentProcess.Handle
    [IO.File]::WriteAllText((Join-Path $root 'ready'), 'ready')
    if (-not $parentProcess.WaitForExit(30000)) { throw 'Desktop did not exit; installation was not started' }
    $failure = Invoke-UpdateTransaction -Install {
        # Silent mode cannot sit indefinitely on an NSIS "Installation Aborted"
        # page. No /R: only this supervisor owns restart, including on failure.
        $directoryArgument = '/D=' + (Split-Path -Parent $plan.desktop)
        $child = Start-Process -FilePath $installer -ArgumentList @('/S', '/UPDATE', $directoryArgument) -WindowStyle Hidden -PassThru
        Wait-Installer $child
        $actual = [Diagnostics.FileVersionInfo]::GetVersionInfo($plan.desktop).ProductVersion
        if ($actual -ne $plan.version) { throw "Installer exited successfully but installed $actual instead of $($plan.version)" }
        $helper = Join-Path (Split-Path -Parent $plan.desktop) 'ratatoskr.exe'
        $versionOutput = Join-Path $root 'helper-version.stdout'
        $probe = Start-Process -FilePath $helper -ArgumentList '--version' -WindowStyle Hidden -PassThru -RedirectStandardOutput $versionOutput -RedirectStandardError (Join-Path $root 'helper-version.stderr')
        Wait-Installer $probe 10000
        $helperVersion = (Get-Content -LiteralPath $versionOutput -Raw).Trim()
        if ($helperVersion -ne "ratatoskr $($plan.version)") { throw "Bundled helper version mismatch: $helperVersion" }
    } -Restore { Restore-Executables $plan $root } -Restart { Start-SyncTray $plan } -Report {
        param($failure)
        Write-UpdateResult $plan $failure
    }
    if ($failure) { [Console]::Error.WriteLine($failure); exit 1 }
}

if ($MyInvocation.InvocationName -ne '.') {
    try { Start-SupervisedUpdate $PlanPath }
    catch {
        $failure = $_.Exception.Message
        # The recovered tray reports ordinary install failures from the ledger.
        # If recovery/reporting failed, alert from this independent process too.
        try {
            $plan = Get-Content -LiteralPath $PlanPath -Raw | ConvertFrom-Json
            Write-UpdateResult $plan $failure
            Show-RecoveryFailure "$failure. Update evidence: $(Split-Path -Parent $PlanPath)"
        }
        catch { [Console]::Error.WriteLine("Update failure: $failure; fallback reporting failed: $($_.Exception.Message)") }
        [Console]::Error.WriteLine($failure)
        exit 1
    }
}
