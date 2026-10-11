param([switch]$SelfTest, [string]$Archive)
$ErrorActionPreference = 'Stop'
function Assert-PikkerPackage([string]$Path) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead((Resolve-Path $Path))
    try {
        $want = @('LICENSE','LEIA-ME.txt','SHA256SUMS.txt','THIRD_PARTY_NOTICES.txt','pikker.exe')
        $names = @($zip.Entries | ForEach-Object FullName)
        $actualNames = ($names | Sort-Object) -join "`n"
        $expectedNames = ($want | Sort-Object) -join "`n"
        if ($actualNames -cne $expectedNames) { throw 'Unexpected or missing package entry' }
        $reader = [IO.StreamReader]::new($zip.GetEntry('SHA256SUMS.txt').Open())
        try { $sums = $reader.ReadToEnd() } finally { $reader.Dispose() }
        $seen = @{}
        foreach ($line in ($sums -split '\r?\n' | Where-Object { $_ })) {
            if ($line -notmatch '^([a-f0-9]{64})  (LICENSE|LEIA-ME\.txt|THIRD_PARTY_NOTICES\.txt|pikker\.exe)$') { throw 'Invalid checksum entry' }
            $hash = $Matches[1]; $name = $Matches[2]
            if ($seen.ContainsKey($name)) { throw 'Duplicate checksum entry' }
            $seen[$name] = $true
            $stream = $zip.GetEntry($name).Open()
            $sha = [Security.Cryptography.SHA256]::Create()
            try { $actual = [Convert]::ToHexString($sha.ComputeHash($stream)).ToLowerInvariant() }
            finally { $sha.Dispose(); $stream.Dispose() }
            if ($actual -cne $hash) { throw 'Package checksum mismatch' }
        }
        if ($seen.Count -ne 4) { throw 'Missing checksums' }
    } finally { $zip.Dispose() }
}
if ($SelfTest) {
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('pikker-package-test-' + [guid]::NewGuid())
    New-Item -ItemType Directory $tmp | Out-Null
    try {
        $exe = Join-Path $tmp 'input.exe'; [IO.File]::WriteAllBytes($exe, [byte[]](77,90,1,2))
        $good = Join-Path $tmp 'good'
        $manifest = Get-Content -Raw (Join-Path (Split-Path $PSScriptRoot -Parent) 'Cargo.toml')
        if ($manifest -notmatch '(?m)^version\s*=\s*"([^"\r\n]+)"') { throw 'Test manifest version missing' }
        $currentVersion = $Matches[1]
        & "$PSScriptRoot/package-windows.ps1" -Exe $exe -Version $currentVersion -OutputDirectory $good
        Assert-PikkerPackage (Join-Path $good 'pikker-windows-x64.zip')
        foreach ($version in @(($currentVersion + '-different'),'../main',($currentVersion + ';echo BAD'))) {
            $rejected = $false
            try { & "$PSScriptRoot/package-windows.ps1" -Exe $exe -Version $version -OutputDirectory (Join-Path $tmp ([guid]::NewGuid())) }
            catch { $rejected = $true }
            if (!$rejected) { throw 'Invalid version was accepted' }
        }
        $bad = Join-Path $tmp 'bad'; Expand-Archive (Join-Path $good 'pikker-windows-x64.zip') $bad
        foreach ($mutation in @('extra','missing','changed','dll')) {
            $copy = Join-Path $tmp $mutation; Copy-Item $bad $copy -Recurse
            switch ($mutation) {
                'extra' { [IO.File]::WriteAllText((Join-Path $copy 'profile.json'),'{}') }
                'missing' { Remove-Item (Join-Path $copy 'THIRD_PARTY_NOTICES.txt') }
                'changed' { [IO.File]::WriteAllText((Join-Path $copy 'LICENSE'),'tampered') }
                'dll' { [IO.File]::WriteAllText((Join-Path $copy 'unknown.dll'),'unapproved') }
            }
            $archive = Join-Path $tmp ($mutation + '.zip'); Compress-Archive "$copy/*" $archive
            $rejected = $false
            try { Assert-PikkerPackage $archive } catch { $rejected = $true }
            if (!$rejected) { throw ('Invalid archive accepted: ' + $mutation) }
        }
        'Package self-tests passed'
    } finally { Remove-Item -LiteralPath $tmp -Recurse -Force }
} elseif ($Archive) { Assert-PikkerPackage $Archive; 'Package verified' }
else { throw 'Specify -SelfTest or -Archive' }
