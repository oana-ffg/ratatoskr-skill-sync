param([string]$PlanPath)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Start-UpdateProcess([string]$Executable, [string]$Arguments, [bool]$CaptureOutput = $false) {
    # Windows PowerShell's Start-Process -PassThru can lose a fast child's exit
    # code. Starting through Process retains the handle from creation to wait.
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Executable
    $info.Arguments = $Arguments
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $CaptureOutput
    $info.RedirectStandardError = $CaptureOutput
    $child = [Diagnostics.Process]::new()
    $child.StartInfo = $info
    if (-not $child.Start()) { throw "Could not start $Executable" }
    return $child
}

function Get-UpdateHash([string]$Path) {
    # Do not depend on an inherited PSModulePath exposing Get-FileHash, e.g.
    # when Windows PowerShell is launched by PowerShell 7 or a packaged app.
    $stream = [IO.File]::OpenRead($Path)
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($algorithm.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
    finally { $algorithm.Dispose(); $stream.Dispose() }
}

function Write-UpdateResult($Plan, [string]$ErrorMessage) {
    $path = Join-Path $Plan.data 'windows-install-result.json'
    $temporary = "$path.$([guid]::NewGuid().ToString('N')).tmp"
    $result = @{ version = $Plan.version; finished_at = [datetime]::UtcNow.ToString('o'); error = $null }
    if ($ErrorMessage) { $result.error = "Desktop update: $ErrorMessage" }
    [IO.File]::WriteAllText($temporary, ($result | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    if ([IO.File]::Exists($path)) { [IO.File]::Replace($temporary, $path, "$path.previous-$([guid]::NewGuid().ToString('N'))") }
    else { [IO.File]::Move($temporary, $path) }
}

function Throw-UnstoppedInstaller([string]$Message) {
    $problem = [InvalidOperationException]::new($Message)
    $problem.Data['InstallerStillRunning'] = $true
    throw $problem
}

function Wait-Installer($Child, [int]$TimeoutMilliseconds = 120000) {
    if (-not $Child.WaitForExit($TimeoutMilliseconds)) {
        # Kill the owned process tree before restoring any files. Never leave an
        # installer capable of overwriting the recovered, running installation.
        try { $termination = Start-UpdateProcess "$env:SystemRoot\System32\taskkill.exe" "/PID $($Child.Id) /T /F" }
        catch { Throw-UnstoppedInstaller "Installer timed out; could not start process-tree termination: $($_.Exception.Message)" }
        if (-not $termination.WaitForExit(10000)) { Throw-UnstoppedInstaller 'Installer timed out; process-tree termination also timed out' }
        if (-not $Child.WaitForExit(10000)) { Throw-UnstoppedInstaller 'Installer timed out; could not stop its process tree' }
        if ($termination.ExitCode -ne 0) { Throw-UnstoppedInstaller "Installer timed out; process-tree termination failed ($($termination.ExitCode))" }
        throw "Installer timed out after $TimeoutMilliseconds milliseconds and was stopped"
    }
    if ($Child.ExitCode -ne 0) { throw "Installer failed with exit code $($Child.ExitCode)" }
}

function Invoke-UpdateTransaction([scriptblock]$Install, [scriptblock]$Restore, [scriptblock]$Restart, [scriptblock]$Report) {
    $failure = $null
    $unsafeToRestore = $false
    try { & $Install }
    catch {
        $failure = $_.Exception.Message
        $unsafeToRestore = $_.Exception.Data['InstallerStillRunning'] -eq $true
        if (-not $unsafeToRestore) {
            try { & $Restore }
            catch { $failure += "; restoring the previous executables failed: $($_.Exception.Message)" }
        }
    }
    # A failed ledger write must not prevent the restart. The caller's outer
    # catch also provides a native alert when reporting itself fails.
    $reportFailure = $null
    try { & $Report $failure } catch { $reportFailure = $_.Exception.Message }
    if ($unsafeToRestore) { throw "$failure; recovery deferred because the installer could still be writing files. $reportFailure" }
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
    $child = Start-UpdateProcess $Plan.desktop '--background'
    if ($child.WaitForExit(3000)) { throw "Desktop exited during recovery ($($child.ExitCode))" }
}

function Show-RecoveryFailure([string]$AppId, [string]$Message) {
    # This process survives the tray: it can notify even if the tray cannot start.
    [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType=WindowsRuntime] > $null
    [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType=WindowsRuntime] > $null
    $xml = New-Object Windows.Data.Xml.Dom.XmlDocument
    $xml.LoadXml('<toast><visual><binding template="ToastText02"><text id="1">Ratatoskr update needs attention</text><text id="2"></text></binding></visual></toast>')
    $xml.GetElementsByTagName('text').Item(1).InnerText = $Message
    [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($AppId).Show([Windows.UI.Notifications.ToastNotification]::new($xml))
}

function Report-RecoveryFailure($Plan, [string]$Failure) {
    # A full/unwritable disk must not prevent the independent notification.
    try { Write-UpdateResult $Plan $Failure }
    catch { $Failure += "; saving update diagnostics failed: $($_.Exception.Message)" }
    try { Show-RecoveryFailure $Plan.app_id $Failure }
    catch {
        $Failure += "; notification delivery failed: $($_.Exception.Message)"
        try { Write-UpdateResult $Plan $Failure }
        catch { [Console]::Error.WriteLine("Cannot retain fallback delivery failure: $($_.Exception.Message)") }
    }
    [Console]::Error.WriteLine($Failure)
}

function Start-SupervisedUpdate([string]$Path) {
    $root = Split-Path -Parent $Path
    $plan = [IO.File]::ReadAllText($Path) | ConvertFrom-Json
    $installer = Join-Path $root 'installer.exe'
    if ((Get-UpdateHash $installer) -ne $plan.sha256) { throw 'Staged installer hash mismatch' }
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
        $child = Start-UpdateProcess $installer "/S /UPDATE $directoryArgument"
        Wait-Installer $child
        $actual = [Diagnostics.FileVersionInfo]::GetVersionInfo($plan.desktop).ProductVersion
        if ($actual -ne $plan.version) { throw "Installer exited successfully but installed $actual instead of $($plan.version)" }
        $helper = Join-Path (Split-Path -Parent $plan.desktop) 'ratatoskr.exe'
        $versionOutput = Join-Path $root 'helper-version.stdout'
        $probe = Start-UpdateProcess $helper '--version' $true
        Wait-Installer $probe 10000
        $helperVersion = $probe.StandardOutput.ReadToEnd().Trim()
        [IO.File]::WriteAllText($versionOutput, $helperVersion)
        [IO.File]::WriteAllText((Join-Path $root 'helper-version.stderr'), $probe.StandardError.ReadToEnd())
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
            $plan = [IO.File]::ReadAllText($PlanPath) | ConvertFrom-Json
            Report-RecoveryFailure $plan "$failure. Update evidence: $(Split-Path -Parent $PlanPath)"
        }
        catch { [Console]::Error.WriteLine("Update failure: $failure; fallback reporting failed: $($_.Exception.Message)") }
        [Console]::Error.WriteLine($failure)
        exit 1
    }
}
