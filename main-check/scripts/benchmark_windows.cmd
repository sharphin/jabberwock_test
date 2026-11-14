@echo off
setlocal EnableExtensions DisableDelayedExpansion
pushd "%~dp0.." || exit /b 2

rem Native Windows four-way benchmark: self-hosted jwc / rustc / clang++ / go.
rem Do not invoke the safety test suite merely to initialize the toolchain.
if defined JW_VSDEVCMD goto vs_found
set "JW_VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%JW_VSWHERE%" goto vs_fallback
for /f "usebackq delims=" %%I in (`"%JW_VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "JW_VSDEVCMD=%%I\Common7\Tools\VsDevCmd.bat"
if defined JW_VSDEVCMD goto vs_found
:vs_fallback
set "JW_VSDEVCMD=%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat"
:vs_found
if not exist "%JW_VSDEVCMD%" (
  echo ERROR: VsDevCmd.bat not found.
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
clang++ --version
if errorlevel 1 goto failed
rustc --version
if errorlevel 1 goto failed
go version
if errorlevel 1 goto failed

if defined JW_PYTHON goto explicit_python
py -3 -c "import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)"
if errorlevel 1 (
  echo ERROR: Python 3.10+ via py -3 is required.
  goto failed
)
py -3 scripts/benchmark_report.py %*
goto finished

:explicit_python
"%JW_PYTHON%" -c "import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)"
if errorlevel 1 goto failed
"%JW_PYTHON%" scripts/benchmark_report.py %*
:finished
set "JW_BENCH_RESULT=%ERRORLEVEL%"
popd
exit /b %JW_BENCH_RESULT%
:failed
echo ERROR: Windows benchmark preflight failed; no benchmark was launched.
popd
exit /b 2
