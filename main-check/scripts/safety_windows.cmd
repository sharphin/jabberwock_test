@echo off
setlocal EnableExtensions DisableDelayedExpansion
pushd "%~dp0.." || exit /b 2

rem An explicit override wins; otherwise locate an installed C++ toolchain.
if defined JW_VSDEVCMD goto vs_found
set "JW_VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%JW_VSWHERE%" goto vs_fallback
for /f "usebackq delims=" %%I in (`"%JW_VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "JW_VSDEVCMD=%%I\Common7\Tools\VsDevCmd.bat"
if defined JW_VSDEVCMD goto vs_found
:vs_fallback
set "JW_VSDEVCMD=%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat"
:vs_found
if not exist "%JW_VSDEVCMD%" (
  echo ERROR: VsDevCmd.bat was not found. See docs/windows-setup.md.
  goto failed
)
call "%JW_VSDEVCMD%" -arch=x64 -host_arch=x64
if errorlevel 1 goto failed
if /i not "%VSCMD_ARG_TGT_ARCH%"=="x64" goto failed
if /i not "%VSCMD_ARG_HOST_ARCH%"=="x64" goto failed

set "PATH=%USERPROFILE%\.elan\bin;%USERPROFILE%\.cargo\bin;%ProgramFiles%\LLVM\bin;%PATH%"
if not defined JW_CLANG set "JW_CLANG=%ProgramFiles%\LLVM\bin\clang.exe"
"%JW_CLANG%" --version
if errorlevel 1 goto failed

rem Avoid the Windows Store python.exe alias. JW_PYTHON is an optional full path.
if defined JW_PYTHON goto explicit_python
py -3 -c "import sys; print(sys.executable); sys.exit(0 if sys.version_info >= (3,10) else 1)"
if errorlevel 1 (
  echo ERROR: Python 3.10+ via py -3 is required. Or set JW_PYTHON to the real python.exe path.
  goto failed
)
goto preflight
:explicit_python
"%JW_PYTHON%" -c "import sys; print(sys.executable); sys.exit(0 if sys.version_info >= (3,10) else 1)"
if errorlevel 1 goto failed

:preflight
if not exist target mkdir target
echo Checking Windows x64 headers and linking before the safety suite...
>target/windows-toolchain-probe.log echo Windows x64 setup and link diagnostics
>>target/windows-toolchain-probe.log set JW_CLANG
>>target/windows-toolchain-probe.log set VSCMD_ARG_
>>target/windows-toolchain-probe.log set INCLUDE
>>target/windows-toolchain-probe.log set LIB
"%JW_CLANG%" -v -O0 scripts/windows_toolchain_probe.c -flto -fuse-ld=lld -lshell32 -o target/windows-toolchain-probe.exe >>target/windows-toolchain-probe.log 2>&1
if errorlevel 1 (
  type target\windows-toolchain-probe.log
  goto failed
)
echo Basic x64 toolchain check passed. Sanitizer coverage is checked by the suite.
if defined JW_PYTHON goto run_explicit
py -3 scripts/safety_windows.py %*
goto finished
:run_explicit
"%JW_PYTHON%" scripts/safety_windows.py %*
:finished
set "JW_RESULT=%ERRORLEVEL%"
popd
exit /b %JW_RESULT%
:failed
echo ERROR: Setup or preflight failed. The safety suite was not started.
popd
exit /b 2
