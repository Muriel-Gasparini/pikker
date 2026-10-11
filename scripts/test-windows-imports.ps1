$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/check-windows-imports.ps1"
$sample = "Image has the following dependencies:`n    KERNEL32.dll`n    USER32.dll`n    DWRITE.dll`n    api-ms-win-core-synch-l1-2-0.dll`nSummary"
$imports = @(Get-ImportedLibraries $sample)
if ($imports.Count -ne 4) { throw 'Dumpbin dependency parser lost libraries' }
Assert-SystemImports $imports
Assert-SystemImports @('KERNEL32.dll', 'comctl32.dll')
Assert-SystemImports @('KERNEL32.dll', 'COMCTL32.DLL')
$reviewMessage = ''
try { Assert-SystemImports @('KERNEL32.dll', 'first-private.dll', 'second-private.dll') } catch { $reviewMessage = $_.Exception.Message }
if ($reviewMessage -notmatch 'first-private\.dll' -or $reviewMessage -notmatch 'second-private\.dll') {
    throw 'Import review must report every non-system dependency, not only the first'
}
Assert-SystemImports @('KERNEL32.dll', 'combase.dll')
Assert-SystemImports @('KERNEL32.dll', 'COMBASE.DLL')
foreach ($libraries in @(@('KERNEL32.dll','VCRUNTIME140.dll'), @('KERNEL32.dll','libunwind.dll'), @('unknown.dll'), @())) {
    $rejected = $false
    try { Assert-SystemImports $libraries } catch { $rejected = $true }
    if (!$rejected) { throw 'Missing or non-system imports accepted' }
}
'Windows import validation tests passed'
