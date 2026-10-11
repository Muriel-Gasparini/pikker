param([Parameter(Mandatory)][string]$Archive)
$ErrorActionPreference = 'Stop'
if (!$IsWindows) { throw 'Executable smoke test requires Windows' }
& "$PSScriptRoot/test-package-windows.ps1" -Archive $Archive
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('pikker smoke ' + [guid]::NewGuid())
$oldData = $env:PIKKER_DATA_DIR
$oldLock = $env:PIKKER_LCU_LOCKFILE
$process = $null
$firewallRule = $null
try {
    New-Item -ItemType Directory $tmp | Out-Null
    Expand-Archive -LiteralPath $Archive -DestinationPath $tmp
    $env:PIKKER_DATA_DIR = Join-Path $tmp 'isolated-data'
    $env:PIKKER_LCU_LOCKFILE = Join-Path $tmp 'absent-lockfile'
    # Hosted CI is ephemeral; block only this extracted EXE's outbound access.
    # Smoke tests must not consume real meta-provider quota.
    $firewallRule = New-NetFirewallRule -DisplayName ('Pikker smoke ' + [guid]::NewGuid()) -Direction Outbound -Action Block -Program (Join-Path $tmp 'pikker.exe')
    $process = Start-Process -FilePath (Join-Path $tmp 'pikker.exe') -WorkingDirectory $tmp -PassThru
    $window = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 500
        $process.Refresh()
        if ($process.HasExited) { throw ('EXE exited before opening its window: ' + $process.ExitCode) }
        if ($process.MainWindowHandle -ne 0) { $window = $true; break }
    }
    if (!$window) { throw 'No native window observed within 15 seconds' }
    if (!$process.CloseMainWindow()) { throw 'Cannot request normal window shutdown' }
    if (!$process.WaitForExit(20000)) { throw 'Normal shutdown exceeded 20 seconds' }
    if ($process.ExitCode -ne 0) { throw ('EXE shutdown failed: ' + $process.ExitCode) }
    'Windows executable opened and closed normally (no live LoL validation)'
} finally {
    if ($process) {
        $process.Refresh()
        if (!$process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
        $process.Dispose()
    }
    $env:PIKKER_DATA_DIR = $oldData
    $env:PIKKER_LCU_LOCKFILE = $oldLock
    if ($firewallRule) { $firewallRule | Remove-NetFirewallRule }
    if (Test-Path -LiteralPath $tmp) { Remove-Item -LiteralPath $tmp -Recurse -Force }
}
