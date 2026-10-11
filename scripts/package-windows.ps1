param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Version,
    [string]$OutputDirectory = 'dist'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$manifest = Get-Content -Raw (Join-Path $root 'Cargo.toml')
if ($manifest -notmatch '(?m)^version\s*=\s*"([^"\r\n]+)"') { throw 'Package version missing' }
$expected = $Matches[1]
if ($Version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$' -or $Version -cne $expected) { throw 'Version must match Cargo.toml' }
if (!(Test-Path -LiteralPath $Exe -PathType Leaf)) { throw 'Executable missing' }
foreach ($name in @('LICENSE','THIRD_PARTY_NOTICES.txt','docs/windows-quickstart.txt')) {
    if (!(Test-Path -LiteralPath (Join-Path $root $name) -PathType Leaf)) { throw ('Required notice missing: ' + $name) }
}
New-Item -ItemType Directory -Force $OutputDirectory | Out-Null
$output = (Resolve-Path $OutputDirectory).Path
$zip = Join-Path $output 'pikker-windows-x64.zip'
if (Test-Path -LiteralPath $zip) { throw 'Refusing to overwrite existing archive' }
$stage = Join-Path ([IO.Path]::GetTempPath()) ('pikker-package-' + [guid]::NewGuid())
New-Item -ItemType Directory $stage | Out-Null
try {
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $stage 'pikker.exe')
    Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination $stage
    Copy-Item -LiteralPath (Join-Path $root 'THIRD_PARTY_NOTICES.txt') -Destination $stage
    Copy-Item -LiteralPath (Join-Path $root 'docs/windows-quickstart.txt') -Destination (Join-Path $stage 'LEIA-ME.txt')
    $sums = foreach ($name in @('LICENSE','LEIA-ME.txt','THIRD_PARTY_NOTICES.txt','pikker.exe')) {
        (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $stage $name)).Hash.ToLowerInvariant() + '  ' + $name
    }
    [IO.File]::WriteAllLines((Join-Path $stage 'SHA256SUMS.txt'), [string[]]$sums)
    Compress-Archive -Path "$stage/*" -DestinationPath $zip
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText((Join-Path $output 'SHA256SUMS.txt'), "$hash  pikker-windows-x64.zip`n")
    $zip
} finally { Remove-Item -LiteralPath $stage -Recurse -Force }
