$ErrorActionPreference = 'Stop'
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('pikker-next-version-' + [guid]::NewGuid())
$root = Split-Path $PSScriptRoot -Parent
try {
    New-Item -ItemType Directory (Join-Path $tmp 'scripts'), (Join-Path $tmp 'docs') | Out-Null
    foreach ($script in @('package-windows.ps1','test-package-windows.ps1')) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $script) -Destination (Join-Path $tmp 'scripts')
    }
    foreach ($notice in @('LICENSE','THIRD_PARTY_NOTICES.txt')) {
        Copy-Item -LiteralPath (Join-Path $root $notice) -Destination $tmp
    }
    Copy-Item -LiteralPath (Join-Path $root 'docs/windows-quickstart.txt') -Destination (Join-Path $tmp 'docs')
    [IO.File]::WriteAllText((Join-Path $tmp 'Cargo.toml'), "[package]`nversion = `"0.1.0-beta.2`"`n")
    & (Get-Process -Id $PID).Path -NoProfile -File (Join-Path $tmp 'scripts/test-package-windows.ps1') -SelfTest
    if ($LASTEXITCODE -ne 0) { throw 'Packaging self-tests failed after normal version bump' }
    'Packaging supports version updates'
} finally {
    if (Test-Path -LiteralPath $tmp) { Remove-Item -LiteralPath $tmp -Recurse -Force }
}
