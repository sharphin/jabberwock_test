# Keep strict error handling local to this function; do not poison the interactive VS Code terminal.
function Initialize-JabberwockDev {
    $ErrorActionPreference = "Stop"

    # Workspace-local MSVC headers + Windows SDK for Clang; no machine-wide changes.
    $programFilesX86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
    $vs = Join-Path $programFilesX86 "Microsoft Visual Studio\18\BuildTools"
    $vcRoot = Join-Path $vs "VC\Tools\MSVC"
    $kits = Join-Path $programFilesX86 "Windows Kits\10"

    if (!(Get-Command clang -ErrorAction SilentlyContinue)) { throw "clang.exe not found. Install LLVM or add it to PATH." }
    if (!(Test-Path $vcRoot)) { throw "MSVC tools directory not found: $vcRoot" }

    # cl.exe is optional here: Jabberwock uses Clang. Select a toolset with the CRT headers.
    $toolset = Get-ChildItem -LiteralPath $vcRoot -Directory |
        Where-Object { Test-Path (Join-Path $_.FullName "include\vcruntime.h") } |
        Sort-Object { try { [version]$_.Name } catch { [version]"0.0" } } -Descending |
        Select-Object -First 1
    if (!$toolset) {
        throw "vcruntime.h is missing from all MSVC toolsets under $vcRoot. In Visual Studio Installer, install/repair the MSVC x64/x86 build tools component."
    }
    $vc = $toolset.FullName
    $vcInclude = Join-Path $vc "include"

    $sdkIncludeRoot = Join-Path $kits "Include"
    if (!(Test-Path $sdkIncludeRoot)) { throw "Windows SDK headers not found: $sdkIncludeRoot. Install a Windows 10/11 SDK in Visual Studio Installer." }
    $sdk = Get-ChildItem -LiteralPath $sdkIncludeRoot -Directory |
        Where-Object { Test-Path (Join-Path $_.FullName "ucrt\math.h") } |
        Sort-Object { try { [version]$_.Name } catch { [version]"0.0" } } -Descending |
        Select-Object -First 1
    if (!$sdk) { throw "Windows SDK UCRT math.h not found under $sdkIncludeRoot. Install/repair a Windows 10/11 SDK." }

    $sdkVersion = $sdk.Name
    $sdkInc = $sdk.FullName
    $sdkLib = Join-Path (Join-Path $kits "Lib") $sdkVersion
    $env:VCToolsInstallDir = "$vc\"
    $env:VCToolsVersion = $toolset.Name
    $env:VSINSTALLDIR = "$vs\"
    $env:WindowsSdkDir = "$kits\"
    $env:WindowsSDKVersion = "$sdkVersion\"

    $includePaths = @(
        $vcInclude
        (Join-Path $sdkInc "ucrt")
        (Join-Path $sdkInc "shared")
        (Join-Path $sdkInc "um")
        (Join-Path $sdkInc "winrt")
        (Join-Path $sdkInc "cppwinrt")
    ) | Where-Object { Test-Path $_ }
    $oldIncludes = @($env:INCLUDE -split ";" | Where-Object { $_ })
    $env:INCLUDE = (($includePaths + $oldIncludes | Select-Object -Unique) -join ";")

    $libPaths = @(
        (Join-Path $vc "lib\x64")
        (Join-Path $sdkLib "ucrt\x64")
        (Join-Path $sdkLib "um\x64")
    ) | Where-Object { Test-Path $_ }
    $oldLibs = @($env:LIB -split ";" | Where-Object { $_ })
    $env:LIB = (($libPaths + $oldLibs | Select-Object -Unique) -join ";")

    # Verify Clang can see the Windows CRT headers before announcing readiness.
    $probe = Join-Path $env:TEMP "jabberwock-header-check-$PID.c"
    try {
        Set-Content -LiteralPath $probe -Encoding ASCII -Value @(
            "#include <math.h>"
            "int main(void) { return 0; }"
        )
        & clang -x c -fsyntax-only $probe
        if ($LASTEXITCODE -ne 0) { throw "Clang could not compile a source file including math.h; see the diagnostic above." }
    }
    finally { Remove-Item -LiteralPath $probe -Force -ErrorAction SilentlyContinue }

    Set-Location (Split-Path -Parent $PSScriptRoot)
    Write-Host "Jabberwock Dev Environment: READY" -ForegroundColor Green
    Write-Host "MSVC headers: $($toolset.Name) | Windows SDK: $sdkVersion | Compiler: Clang"
}

Initialize-JabberwockDev
