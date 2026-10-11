param([string]$Exe)
$ErrorActionPreference = 'Stop'
function Get-ImportedLibraries([string]$Dump) {
    foreach ($line in ($Dump -split '\r?\n')) {
        if ($line -match '^\s+([A-Za-z0-9_.-]+\.dll)\s*$') { $Matches[1] }
    }
}
function Assert-SystemImports([string[]]$Libraries) {
    if (!$Libraries -or $Libraries.Count -eq 0) { throw 'No DLL dependencies detected' }
    $system = @('advapi32.dll','bcrypt.dll','bcryptprimitives.dll','combase.dll','comctl32.dll','comdlg32.dll','crypt32.dll',
        'd3d11.dll','dbghelp.dll','dwmapi.dll','dwrite.dll','dxgi.dll','gdi32.dll','imm32.dll','iphlpapi.dll',
        'kernel32.dll','kernelbase.dll','msvcrt.dll','ntdll.dll','ole32.dll','oleaut32.dll',
        'opengl32.dll','propsys.dll','rpcrt4.dll','secur32.dll','setupapi.dll','shell32.dll',
        'shlwapi.dll','ucrtbase.dll','user32.dll','userenv.dll','uxtheme.dll','version.dll','winmm.dll','ws2_32.dll')
    $review = @(foreach ($library in $Libraries) {
        $name = $library.ToLowerInvariant()
        if ($name -notmatch '^api-ms-win-[a-z0-9-]+\.dll$' -and $name -notmatch '^ext-ms-win-[a-z0-9-]+\.dll$' -and $name -notin $system) {
            $library
        }
    })
    if ($review.Count -gt 0) {
        throw ('Non-system DLLs require explicit packaging/license review: ' + ($review -join ', '))
    }
}
if ($Exe) {
    if (!$IsWindows) { throw 'PE inspection requires Windows build tools' }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $dumpbin = @(& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -find 'VC/Tools/MSVC/**/bin/Hostx64/x64/dumpbin.exe')[0]
    if (!$dumpbin -or !(Test-Path -LiteralPath $dumpbin)) { throw 'dumpbin unavailable' }
    $headers = & $dumpbin /HEADERS $Exe
    if ($LASTEXITCODE -ne 0 -or ($headers -join "`n") -notmatch '8664 machine' -or ($headers -join "`n") -notmatch '2 subsystem') {
        throw 'Expected AMD64 Windows GUI executable'
    }
    $dependencies = & $dumpbin /DEPENDENTS $Exe
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect executable imports' }
    $libraries = @(Get-ImportedLibraries ($dependencies -join "`n"))
    $libraries
    Assert-SystemImports $libraries
}
